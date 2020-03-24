use guppy::graph::{DependencyDirection, DependencyLink, PackageGraph, PackageMetadata};
use guppy::{MetadataCommand, PackageId};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{
  hash_map::{Entry, HashMap},
  HashSet,
};
use std::ffi::OsStr;
use std::fs;
use std::iter::FromIterator;
use std::path::Path;
use std::path::PathBuf;
use tempdir::TempDir;

//
// Essential Structs
// =================
//

/// PackageRisk contains information about a package after analysis.
#[derive(Default, Serialize, Deserialize)]
pub struct PackageRisk {
  // metadata
  // --------
  /// name of the dependency
  pub name: String,
  /// potentially different versions are pulled (bad)
  pub versions: HashSet<String>,
  /// link to its repository
  pub repo: Option<String>,
  // useful for analysis
  // -------------------
  /// path to the actual source code on disk
  #[serde(skip)]
  pub manifest_path: PathBuf,
  // analysis result
  // ---------------
  /// transitive dependencies (in reversed direction)
  pub transitive_dependencies: HashSet<PackageId>,
  /// total number of transitive third party dependencies imported
  /// by this dependency (not including this dependency)
  pub total_third_deps: u64,
  /// number of root crates that import this package
  pub root_importers: Vec<PackageId>,
  // TODO: implement this
  /// total number of transitive third party dependencies imported
  /// by this dependency, and only by this dependency
  pub exclusive_deps_introduced: Vec<PackageId>,
  /// number of non-rust lines-of-code
  pub non_rust_loc: u64,
  /// number of rust lines-of-code
  pub rust_loc: u64,
  /// number of lines of unsafe code
  pub unsafe_loc: u64,
  /// number of github stars, if any
  pub stargazers_count: u64,
}

//
// Analysis Functions
// ==================
//

// GithubResponse is used to parse the response from github
#[derive(serde::Deserialize, Debug)]
struct GithubResponse {
  #[serde(rename = "stargazers_count")]
  stargazers_count: u64,
}
impl GithubResponse {
  fn get_github_stars(
    github_client: reqwest::blocking::Client,
    github_token: Option<(&str, &str)>,
    repo: &str,
  ) -> Option<u64> {
    // is this a github repo?
    let re = Regex::new(r"github\.com/([a-zA-Z0-9._-]*/[a-zA-Z0-9._-]*)").unwrap();
    let caps = re.captures(repo);
    caps
      .and_then(|caps| caps.get(1))
      // yes it is a github repo
      .and_then(|repo| {
        // create request to github API
        let request_url = format!(
          "https://api.github.com/repos/{}",
          repo.as_str().trim_end_matches(".git")
        );
        let mut request = github_client.get(&request_url);
        // if we have a github token, use it
        if let Some((username, token)) = github_token {
          request = request.basic_auth(username, Some(token));
        }
        // send the request and convert to option
        eprintln!("sending request to {}", request_url);
        request.send().ok()
      })
      .and_then(|resp| {
        if !resp.status().is_success() {
          eprintln!("github request failed");
          eprintln!("status: {}", resp.status());
          eprintln!("text: {:?}", resp.text());
          return None;
        }
        let resp: reqwest::Result<GithubResponse> = resp.json();
        match resp {
          Ok(x) => Some(x.stargazers_count),
          Err(err) => {
            eprintln!("{}", err);
            None
          }
        }
      })
  }
}

fn get_root_importers(
  package_graph: &PackageGraph,
  root_crates: &HashSet<&PackageId>,
  dependency: &PackageId,
) -> Vec<PackageId> {
  let root_importers = package_graph
    .select_reverse(std::iter::once(dependency))
    .unwrap();
  let root_importers = root_importers.into_iter_metadatas(Some(DependencyDirection::Reverse));
  let root_importers: Vec<&PackageMetadata> = root_importers
    .filter(|pkg_metadata| root_crates.contains(&pkg_metadata.id())) // a root crate is an importer
    .collect();
  let root_importers = root_importers
    .iter()
    .map(|pkg_metadata| pkg_metadata.id().clone())
    .collect();
  root_importers
}

fn get_exclusive_deps(
  package_graph: &PackageGraph,
  root_crates: &HashSet<&PackageId>,
  dependency: &PackageId,
) -> Vec<PackageId> {
  // get all the transitive dependencies of `dependency`
  let transitive_deps = package_graph
    .select_forward(std::iter::once(dependency))
    .unwrap();
  let transitive_deps = transitive_deps.into_iter_ids(Some(DependencyDirection::Forward));
  // re-create a graph without edges leading to our dependency (and its tree)
  let mut package_graph = package_graph.clone();
  package_graph.retain_edges(|_, dep_link| !(dep_link.to.id() == dependency));
  // obtain all dependencies from the root_crates
  let new_all = package_graph
    .select_forward(root_crates.iter().copied())
    .unwrap();
  let new_all: Vec<_> = new_all
    .into_iter_ids(Some(DependencyDirection::Forward))
    .collect();
  // check if the original transitive dependencies are in there
  let mut exclusive_deps = Vec::new();
  for transitive_dep in transitive_deps {
    // don't include the dependency itself in this list
    if transitive_dep == dependency {
      continue;
    }
    // if it's not in the new graph, it's exclusive to our dependency!
    if !new_all.contains(&transitive_dep) {
      exclusive_deps.push(transitive_dep.clone());
    }
  }
  exclusive_deps
}

// count the lines-of-code of all the given files
fn get_loc(package_risk: &mut PackageRisk, dependency_files: &HashSet<String>) {
  for dependency_file in dependency_files {
    // look for all lines of code (not just rust)
    let lang = loc::lang_from_ext(dependency_file);
    if lang != loc::Lang::Unrecognized {
      let count = loc::count(dependency_file);
      // update LOC
      package_risk.non_rust_loc += u64::from(count.code);
      if lang == loc::Lang::Rust {
        package_risk.rust_loc += u64::from(count.code);
      }
    }
  }
}

// count the unsafe lines-of-code of all the given rust files
fn get_unsafe(package_risk: &mut PackageRisk, dependency_files: &HashSet<String>) {
  for dependency_file in dependency_files {
    let dependency_path = Path::new(dependency_file);
    if dependency_path.extension() != Some(OsStr::new(".rs")) {
      continue;
    }

    // TODO: is this the right way to count unsafe?
    if let Ok(res) = geiger::find_unsafe_in_file(dependency_path, geiger::IncludeTests::No) {
      // update
      let mut unsafe_loc = res.counters.functions.unsafe_;
      unsafe_loc += res.counters.exprs.unsafe_;
      unsafe_loc += res.counters.item_impls.unsafe_;
      unsafe_loc += res.counters.item_traits.unsafe_;
      unsafe_loc += res.counters.methods.unsafe_;
      package_risk.unsafe_loc += unsafe_loc;
    }
  }
}

// parse the dep-info files that contain all the files relevant to the compilation of a dependency (these files are like Makefiles)
// (minus libraries linked via bindings)
// TODO: what to do about them?
fn parse_rustc_dep_info(rustc_dep_info: &Path) -> HashSet<String> {
  let contents = fs::read_to_string(rustc_dep_info).unwrap();
  // inspired from https://github.com/rust-lang/cargo/blob/13cd4fb1e8be5b8fb44008052cf31a839d745a45/src/cargo/core/compiler/fingerprint.rs#L1646
  let mut dependency_files = HashSet::new();
  for line in contents.lines() {
    if let Some(pos) = line.find(": ") {
      let _target = &line[..pos];
      let mut deps = line[pos + 2..].split_whitespace();
      while let Some(file) = deps.next() {
        let mut file = file.to_string();
        while file.ends_with('\\') {
          file.pop();
          file.push(' ');
          //file.push_str(deps.next().ok_or_else(|| {
          //internal("malformed dep-info format, trailing \\".to_string())
          //})?);
          file.push_str(deps.next().expect("malformed dep-info format, trailing \\"));
        }
        dependency_files.insert(file);
      }
    }
  }
  dependency_files
}

// retrieve every single file in the dependency
fn get_every_file_in_folder(package_path: &Path) -> HashSet<String> {
  let mut dependency_files = HashSet::new();
  let walker = ignore::WalkBuilder::new(package_path).build();
  for result in walker {
    let file = result.unwrap();
    // TODO: we ignore symlink here, do we want this? (we could canonicalize)
    if !file.file_type().unwrap().is_file() {
      continue;
    }
    match file.path().to_str() {
      Some(filepath) => dependency_files.insert(filepath.to_string()),
      None => {
        eprintln!("couldn't convert the path to string {:?}", file.path());
        continue;
      }
    };
  }
  //
  dependency_files
}

fn get_dependency_files(package_risk: &mut PackageRisk, target_dir: &Path) -> HashSet<String> {
  use glob::glob;

  // find the dep-info file for that dependency
  let mut dep_files_path = target_dir.to_path_buf();
  dep_files_path.push("debug/deps");
  let without_underscore_name = package_risk.name.clone().replace("-", "_");
  let dependency_file = format!("{}-*.d", without_underscore_name);
  dep_files_path.push(dependency_file);
  println!("debug: {:?}", dep_files_path);
  let dep_files_path = glob(dep_files_path.to_str().unwrap()).unwrap().next();
  match dep_files_path {
    // we found a dep-info file
    Some(glob_result) => {
      let dep_files_path = glob_result.unwrap();
      parse_rustc_dep_info(dep_files_path.as_path())
    }
    // we didn't find a dep-info file, let's do it the old fashion way
    None => {
      let package_path = package_risk.manifest_path.parent().unwrap();
      get_every_file_in_folder(package_path)
    }
  }
}

//
// Helper
// ------
//

fn create_or_update_dependency(
  analysis_result: &mut HashMap<PackageId, PackageRisk>,
  dep_link: &DependencyLink,
) {
  match analysis_result.entry(dep_link.to.id().to_owned()) {
    Entry::Occupied(mut entry) => {
      let package_risk = entry.get_mut();
      package_risk
        .versions
        .insert(dep_link.to.version().to_string());
    }
    Entry::Vacant(entry) => {
      let mut package_risk = PackageRisk::default();
      package_risk.name = dep_link.to.name().to_owned();
      package_risk
        .versions
        .insert(dep_link.to.version().to_string());
      package_risk.repo = dep_link.to.repository().map(|x| x.to_owned());
      package_risk.manifest_path = dep_link.to.manifest_path().to_path_buf();
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
  to_ignore: Option<Vec<&str>>,
) -> Result<(HashSet<PackageId>, HashMap<PackageId, PackageRisk>), String> {
  // Obtain package graph via guppy
  // ------------------------------
  //

  // obtain metadata from manifest_path
  let mut cmd = MetadataCommand::new();
  cmd.manifest_path(manifest_path);
  // TODO: save this metadata to a json file

  // construct graph with guppy
  let package_graph = PackageGraph::from_command(&mut cmd).map_err(|err| err.to_string())?;

  // Obtain internal dependencies
  // ----------------------------
  // Either the sole main crate,
  // or every crate members of the workspace (if there is a workspace)
  //

  let root_crates = package_graph.workspace().member_ids();
  let root_crates: HashSet<&PackageId> = HashSet::from_iter(root_crates);
  let mut root_crates_to_analyze: HashSet<&PackageId> = root_crates.clone();
  // and remove workspace crates that we want to ignore
  if let Some(to_ignore) = to_ignore {
    root_crates_to_analyze = root_crates_to_analyze
      .into_iter()
      .filter(|pkg_id| {
        let package_metadata = package_graph.metadata(pkg_id).unwrap();
        let package_name = package_metadata.name();
        !to_ignore.contains(&package_name)
      })
      .collect();
  }

  // What dependencies do we want to analyze?
  // ----------------------------------------
  //

  let mut analysis_result: HashMap<PackageId, PackageRisk> = HashMap::new();

  // TODO: combine the two loops and inline `create_or_update...`
  // find all direct dependencies
  let mut main_dependencies: HashSet<PackageId> = HashSet::new();
  for root_crate in &root_crates_to_analyze {
    // (non-ignored) root crate > direct dependency
    for dep_link in package_graph.dep_links(root_crate).unwrap() {
      // ignore dev dependencies
      if dep_link.edge.dev_only() {
        continue;
      }
      // ignore root crates (when used as dependency)
      if root_crates.contains(dep_link.to.id()) {
        continue;
      }
      main_dependencies.insert(dep_link.to.id().to_owned());
      create_or_update_dependency(&mut analysis_result, &dep_link);
    }
  }

  // find all transitive dependencies
  let transitive_dependencies = package_graph
    .select_forward(&main_dependencies)
    .unwrap()
    .into_iter_links(Some(DependencyDirection::Forward));
  // (non-ignored) root crate > direct dependency > transitive dependencies
  for dep_link in transitive_dependencies {
    // ignore dev dependencies
    if dep_link.edge.dev_only() {
      continue;
    }
    // ignore root crates (when used as dependency)
    if root_crates.contains(dep_link.to.id()) {
      continue;
    }
    create_or_update_dependency(&mut analysis_result, &dep_link);
  }

  //
  // Build the workspace/crate to obtain dep files
  // ---------------------------------------------
  //

  let target_dir = TempDir::new("target_dir").expect("could not create temporary folder");
  let target_dir = target_dir.path();
  println!("target-dir: {:?}", target_dir);
  let a = std::process::Command::new("cargo")
    .args(&[
      "build",
      "--manifest-path",
      manifest_path,
      "--target-dir",
      target_dir.to_str().unwrap(),
      "-q",
    ])
    .output()
    .expect("failed to build crate");

  // Analyze!
  // --------
  //

  for (package_id, mut package_risk) in analysis_result.iter_mut() {
    // .transitive_dependencies
    package_risk.transitive_dependencies = package_graph
      .select_forward(std::iter::once(package_id))
      .unwrap()
      .into_iter_links(Some(DependencyDirection::Reverse))
      .map(|package_id| package_id.to.id().to_owned())
      .collect();

    // TODO: might not need this field, since we have len(transitive_dependencies)
    // TODO: also, aren't we doing the same thing above here?
    // .total_third_deps
    let mut transitive_deps = HashSet::new();
    for possible_dep in package_graph.package_ids() {
      // ignore root dependencies
      if root_crates.contains(possible_dep) {
        continue;
      }
      if package_graph.depends_on(package_id, possible_dep).unwrap() {
        transitive_deps.insert(possible_dep);
      }
    }
    package_risk.total_third_deps = transitive_deps.len() as u64;

    // .root_importers
    let root_importers = get_root_importers(&package_graph, &root_crates_to_analyze, package_id);
    package_risk.root_importers = root_importers;

    // .exclusive_deps_introduced
    let exclusive_deps_introduced =
      get_exclusive_deps(&package_graph, &root_crates_to_analyze, package_id);
    package_risk.exclusive_deps_introduced = exclusive_deps_introduced;

    // .non_rust_loc + .rust_loc + unsafe_loc
    let dependency_files = get_dependency_files(&mut package_risk, &target_dir);
    get_loc(&mut package_risk, &dependency_files);
    get_unsafe(&mut package_risk, &dependency_files);

    // .stargazers_count
    // TODO: also retrieve latest SHA commit (of release)
    // TODO: also compare it to the hash to the repo we have (this signals a big problem)
    if let Some(repo) = &package_risk.repo {
      let stars = GithubResponse::get_github_stars(http_client.clone(), github_token, &repo);
      if let Some(stars) = stars {
        package_risk.stargazers_count = stars;
      }
    }
  }

  //
  Ok((main_dependencies, analysis_result))
}
