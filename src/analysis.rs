use camino::Utf8PathBuf;
use guppy::graph::{DependencyDirection, PackageGraph, PackageLink};
use guppy::{MetadataCommand, PackageId};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{
    hash_map::{Entry, HashMap},
    HashSet,
};
use std::iter::FromIterator;
use tempdir::TempDir;

use crate::metrics;

//
// Essential Structs
// =================
//

/// PackageRisk contains information about a package after analysis.
/// Note that the word "total" means that it includes transitive dependencies.
#[rustfmt::skip]
#[derive(Default, Serialize, Deserialize, Clone)]
pub struct PackageRisk {

  // metadata
  // --------

  /// name of the dependency
  #[serde(skip)]
  pub name: String,
  /// potentially different versions are pulled (bad)
  pub versions: HashSet<String>,
  /// link to its repository
  pub repo: Option<String>,
  /// description from Cargo.toml
  pub description: Option<String>,

  // useful for analysis
  // -------------------

  /// path to the actual source code on disk
  #[serde(skip)]
  pub manifest_path: Utf8PathBuf,
  /// have we calculated the total LOCs for this dep?
  #[serde(skip)]
  pub total_calculated: bool,

  // analysis result
  // ---------------

  /// is this an internal package?
  pub internal: bool,
  /// is this dependency used for the host target and features?
  pub used: bool,
  
  /// direct dependencies
  pub direct_dependencies: HashSet<String>,
  /// transitive dependencies (not including this dependency)
  pub transitive_dependencies: HashSet<String>,
  /// number of root crates that import this package
  pub root_importers: Vec<String>,
  /// total number of transitive third party dependencies imported
  /// by this dependency, and only by this dependency
  pub exclusive_deps_introduced: Vec<String>,
  /// (total) number of non-rust lines-of-code
  pub loc: u64,
  pub total_loc: u64,
  /// (total) number of rust lines-of-code
  pub rust_loc: u64,
  pub total_rust_loc: u64,
  /// (total) number of lines of unsafe code
  pub unsafe_loc: u64,
  pub total_unsafe_loc: u64,
  /// number of github stars, if any
  pub stargazers_count: Option<u64>,
  /// active contributors on github (in the last 6 months)
  pub active_contributors: Option<u64>,
  /// number of dependent crates on crates.io
  pub crates_io_dependent: Option<u64>,
  /// last update according to crates.io
  pub crates_io_last_updated: Option<String>,
}

//
// Helper
// ------
//

fn create_or_update_dependency(
    analysis_result: &mut HashMap<PackageId, PackageRisk>,
    dep_link: &PackageLink,
) {
    match analysis_result.entry(dep_link.to().id().to_owned()) {
        Entry::Occupied(mut entry) => {
            let package_risk = entry.get_mut();
            package_risk
                .versions
                .insert(dep_link.to().version().to_string());
        }
        Entry::Vacant(entry) => {
            let mut package_risk = PackageRisk::default();
            package_risk.name = dep_link.to().name().to_owned();
            package_risk
                .versions
                .insert(dep_link.to().version().to_string());
            package_risk.repo = dep_link.to().repository().map(|x| x.to_owned());
            package_risk.description = dep_link.to().description().map(|x| x.to_owned());
            package_risk.manifest_path = dep_link.to().manifest_path().to_path_buf();
            package_risk.internal = dep_link.to().in_workspace();
            entry.insert(package_risk);
        }
    };
}

/// Takes a `manifest_path` and produce an analysis stored in `analysis_result`.
///
/// Optionally, you can pass:
/// - `proxy`, a proxy (used to query github to fetch number of stars)
/// - `github_token`, a github personnal access token (PAT) used to query the github API
///   this is useful due to github limiting queries that are not authenticated.
/// - `to_ignore`, a list of direct dependencies to ignore.
///
/// Let's define some useful terms as well:
/// - **workspace packages** or **root crates**: crates that live in the workspace
///   (and not on crates.io for example)
/// - **direct dependency**: third-party dependencies (from crates.io for example)
///   that are imported from the root crates.
/// - **transitive dependencies**: third-party dependencies that end up getting imported
///   at some point. For example if A imports B and B imports C,
///   then C is a transitive dependency of A.
///
pub fn analyze_repo(
    manifest_path: &str,
    http_client: reqwest::blocking::Client,
    github_token: Option<(&str, &str)>,
    packages: Option<Vec<&str>>,
    to_ignore: Option<Vec<&str>>,
    quiet: bool,
) -> Result<
    (
        HashSet<String>,              // root_crates
        HashSet<String>,              // main_dependencies
        HashMap<String, PackageRisk>, // analysis_result
    ),
    String,
> {
    //
    // Obtain package graph via guppy
    // ------------------------------
    //

    // obtain metadata from manifest_path
    let mut cmd = MetadataCommand::new();
    cmd.manifest_path(manifest_path);

    // construct graph with guppy
    let package_graph = PackageGraph::from_command(&mut cmd).map_err(|err| err.to_string())?;

    // check for dependencies
    if !quiet {
        let cycles = package_graph.cycles().all_cycles();
        for cycle in cycles {
            let cycle: Vec<&str> = cycle
                .iter()
                .map(|x| package_graph.metadata(&x).unwrap().name())
                .collect();
            println!("- dependency cycle detected: {:?}", cycle);
        }
    }

    // Obtain internal dependencies
    // ----------------------------
    // Either the sole main crate,
    // or every crate members of the workspace (if there is a workspace)
    //

    let root_crates = package_graph.workspace().member_ids().map(|x| x.clone());
    let root_crates: HashSet<PackageId> = HashSet::from_iter(root_crates);
    let mut root_crates_to_analyze: HashSet<PackageId> = root_crates.clone();
    // either select specific packages or remove ignored packages
    if let Some(packages) = packages {
        root_crates_to_analyze = root_crates_to_analyze
            .into_iter()
            .filter(|pkg_id| {
                let package_metadata = package_graph.metadata(pkg_id).unwrap();
                let package_name = package_metadata.name();
                packages.contains(&package_name)
            })
            .collect();
    } else if let Some(to_ignore) = to_ignore {
        root_crates_to_analyze = root_crates_to_analyze
            .into_iter()
            .filter(|pkg_id| {
                let package_metadata = package_graph.metadata(pkg_id).unwrap();
                let package_name = package_metadata.name();
                !to_ignore.contains(&package_name)
            })
            .collect();
    }

    if root_crates_to_analyze.len() == 0 {
        return Err("dephell: no package to analyze was found".to_string());
    }

    // What dependencies do we want to analyze?
    // ----------------------------------------
    //

    let mut analysis_result: HashMap<PackageId, PackageRisk> = HashMap::new();

    // TODO: combine the two loops and inline `create_or_update...`
    // find all direct dependencies
    let mut main_dependencies_ids: HashSet<PackageId> = HashSet::new();
    let mut main_dependencies: HashSet<String> = HashSet::new();
    for root_crate in &root_crates_to_analyze {
        // (non-ignored) root crate > direct dependency
        let dep_links = package_graph
            .metadata(root_crate)
            .unwrap()
            .direct_links()
            // ignore dev dependencies
            .filter(|dep_link| !dep_link.dev_only());
        for dep_link in dep_links {
            main_dependencies_ids.insert(dep_link.to().id().to_owned());
            main_dependencies.insert(dep_link.to().name().to_string());
            create_or_update_dependency(&mut analysis_result, &dep_link);
        }
    }

    // find all transitive dependencies
    let transitive_dependencies = package_graph.query_forward(&main_dependencies_ids).unwrap();
    // ignore dev dependencies
    let transitive_dependencies =
        transitive_dependencies.resolve_with_fn(|_, link| !link.dev_only());
    let transitive_dependencies = transitive_dependencies.links(DependencyDirection::Reverse);
    // (non-ignored) root crate > direct dependency > transitive dependencies
    for dep_link in transitive_dependencies {
        create_or_update_dependency(&mut analysis_result, &dep_link);
    }

    //
    // Build the workspace/crate to obtain dep files
    // ---------------------------------------------
    //

    let target_dir = TempDir::new("target_dir").expect("could not create temporary folder");
    let target_dir = target_dir.path();
    let output = std::process::Command::new("cargo")
        .env("RUSTFLAGS", "-Funsafe-code  --cap-lints=warn")
        .args(&[
            "check",
            "-vv",
            "--message-format=json-diagnostic-rendered-ansi",
            "--manifest-path",
            manifest_path,
            "--target-dir",
            target_dir.to_str().unwrap(),
        ])
        .output()
        .expect("failed to build crate");

    if !output.status.success() && !quiet {
        eprintln!("dephell: could not build the target manifest path.");
        eprintln!("{}", std::str::from_utf8(&output.stderr).unwrap());
        return Err("Could not build the target manifest path.".to_string());
    }

    // .unsafe_loc - find unsafe by analyzing the compiler's output
    let output = std::io::Cursor::new(output.stdout);
    for message in cargo_metadata::Message::parse_stream(output) {
        match message {
            Ok(cargo_metadata::Message::CompilerMessage(msg)) => {
                if let Some(code) = msg.message.code {
                    if code.code == "unsafe_code" {
                        let package_id = PackageId::new(msg.package_id.repr);
                        analysis_result
                            .entry(package_id)
                            .and_modify(|r| r.unsafe_loc += 1);
                    }
                }
            }
            _ => (),
        }
    }

    // TODO: find # of panic

    /*
    cargo clippy -vv --message-format=json-diagnostic-rendered-ansi -- -Fclippy::panic --cap-lints=warn 2>/dev/null | egrep '^\{' | jq -r 'select(.message?.code?.code? == "clippy::panic").message?.rendered?'
    */

    // Analyze!
    // --------
    //

    for (package_id, mut package_risk) in analysis_result.iter_mut() {
        // .direct_dependencies
        package_risk.direct_dependencies = package_graph
            .metadata(package_id)
            .unwrap()
            .direct_links()
            .filter(|dep_link| !dep_link.dev_only())
            .map(|dep_link| dep_link.to().name().to_string())
            .collect();

        // .transitive_dependencies
        package_risk.transitive_dependencies = package_graph
            .query_forward(std::iter::once(package_id))
            .unwrap()
            .resolve()
            .links(DependencyDirection::Forward)
            .filter(|dep_link| !dep_link.dev_only())
            .map(|dep_link| dep_link.to().name().to_string())
            .collect();

        // .root_importers
        let root_importers =
            metrics::get_root_importers(&package_graph, &root_crates_to_analyze, package_id);
        package_risk.root_importers = root_importers;

        // .exclusive_deps_introduced
        let exclusive_deps_introduced =
            metrics::get_exclusive_deps(&package_graph, &root_crates_to_analyze, package_id);
        package_risk.exclusive_deps_introduced = exclusive_deps_introduced;

        // .in_host_target
        let (used, dependency_files) = metrics::get_dependency_files(
            &package_risk.name,
            package_risk.manifest_path.as_path(),
            &target_dir,
        );
        package_risk.used = used;

        // .loc + .rust_loc
        metrics::get_loc(&mut package_risk, &dependency_files);

        // is this a github repo?
        if let Some(repo_url) = &package_risk.repo {
            if let Some(github_token) = github_token {
                let re = Regex::new(r"github\.com/([a-zA-Z0-9._-]*/[a-zA-Z0-9._-]*)").unwrap();
                if let Some(repo_name) = re
                    .captures(repo_url)
                    .and_then(|caps| caps.get(1))
                    .map(|m| m.as_str())
                {
                    // .stargazers_count
                    let stars =
                        metrics::get_github_stars(http_client.clone(), github_token, &repo_name);
                    package_risk.stargazers_count = stars;

                    // .active_contributors
                    let active_contributors = metrics::get_active_maintainers(
                        http_client.clone(),
                        github_token,
                        &repo_name,
                    );
                    package_risk.active_contributors = active_contributors;
                }
            }
        }

        // .crates_io_dependent
        // TODO: do not make a request to crates.io if this is not a crates.io dep
        let crates_io_dependent =
            metrics::get_crates_io_dependent(http_client.clone(), &package_risk.name);
        package_risk.crates_io_dependent = crates_io_dependent;

        // .crates_io_dependent
        let crates_io_last_updated =
            metrics::get_crates_io_last_updated(http_client.clone(), &package_risk.name);
        package_risk.crates_io_last_updated = crates_io_last_updated;
    }

    // total LOC
    // ---------
    // we need to calculate total LOC after the fact

    let transitive_dependencies = package_graph.query_forward(&main_dependencies_ids).unwrap();
    // ignore dev dependencies
    let transitive_dependencies =
        transitive_dependencies.resolve_with_fn(|_, link| !link.dev_only());
    let transitive_dependencies = transitive_dependencies.package_ids(DependencyDirection::Reverse);

    'main_loop: for package_id in transitive_dependencies {
        // already calculated
        if analysis_result[&package_id].total_calculated {
            continue;
        }

        // get direct deps
        let direct_deps: Vec<_> = package_graph
            .metadata(package_id)
            .unwrap()
            .direct_links()
            .filter(|dep_link| !dep_link.dev_only())
            .map(|dep_link| dep_link.to().id())
            .collect();

        // easy, no deps
        if direct_deps.len() == 0 {
            let loc = analysis_result[&package_id].loc;
            let rust_loc = analysis_result[&package_id].rust_loc;
            let unsafe_loc = analysis_result[&package_id].unsafe_loc;
            let package_risk = analysis_result.get_mut(&package_id).unwrap();
            package_risk.total_loc = loc;
            package_risk.total_rust_loc = rust_loc;
            package_risk.total_unsafe_loc = unsafe_loc;
            package_risk.total_calculated = true;
            // next
            continue;
        }

        // otherwise compute from direct dependencies total count
        let mut total_loc = 0;
        let mut total_rust_loc = 0;
        let mut total_unsafe_loc = 0;

        for direct_dep_id in direct_deps {
            let direct_dep = &analysis_result[direct_dep_id];
            if !direct_dep.total_calculated {
                println!(
                    "total loc error: {:?} was not calculated due to {:?} not being calculated",
                    analysis_result[&package_id].name, direct_dep.name
                );
                continue 'main_loop;
            }

            total_loc += direct_dep.total_loc;
            total_rust_loc += direct_dep.total_rust_loc;
            total_unsafe_loc += direct_dep.total_unsafe_loc;
        }

        // set
        let package_risk = analysis_result.get_mut(&package_id).unwrap();
        package_risk.total_loc = total_loc;
        package_risk.total_rust_loc = total_rust_loc;
        package_risk.total_unsafe_loc = total_unsafe_loc;
        package_risk.total_calculated = true;

        // next
    }

    // PackageId -> name
    // -----------------
    // this is useful because PackageIds are long strings,
    // and we only care about package names for the result
    let root_crates_to_analyze: HashSet<String> = root_crates_to_analyze
        .iter()
        .map(|pkg_id| {
            let package_metadata = package_graph.metadata(pkg_id).unwrap();
            package_metadata.name().to_owned()
        })
        .collect();
    let analysis_result: HashMap<String, PackageRisk> = analysis_result
        .iter()
        .map(|(_, package_risk)| (package_risk.name.clone(), package_risk.clone()))
        .collect();

    //
    Ok((root_crates_to_analyze, main_dependencies, analysis_result))
}
