use std::collections::{hash_set::HashSet, hash_map::{HashMap, Entry}};
use std::iter::FromIterator;
use std::path::{PathBuf};
use guppy::graph::{DependencyDirection, PackageGraph, PackageMetadata, DependencyLink};
use guppy::{MetadataCommand, PackageId};
use regex::Regex;
use serde::{Serialize, Deserialize};

//
// Essential Structs
// =================
//

/// PackageRisk contains information about a package after analysis.
#[derive(Default, Serialize, Deserialize)]
pub struct PackageRisk {

  // metadata
  // --------

  pub name: String,              // name of the dependency
  pub versions: HashSet<String>, // potentially different versions are pulled (bad)
  pub repo: Option<String>,        // link to its repository

  // useful for analysis
  // -------------------

  #[serde(skip)]
  pub manifest_path: PathBuf,     // path to the actual source code on disk

  // analysis result
  // ---------------

  // transitive dependencies (in reversed direction)
  pub transitive_dependencies: HashSet<PackageId>,

  // total number of transitive third party dependencies imported
  // by this dependency (not including this dependency)
  pub total_third_deps: u64,

  // number of root crates that import this package
  pub root_importers: Vec<PackageId>,

  // total number of transitive third party dependencies imported
  // by this dependency, and only this dependency
  pub total_new_third_deps: u64,

  // number of non-rust lines-of-code
  pub non_rust_loc: u64,

  // number of rust lines-of-code
  pub rust_loc: u64,

  // number of lines of unsafe code
  pub unsafe_loc: u64,

  // number of github stars, if any
  pub stargazers_count: u64,

  // number of non-rust lines-of-code, including transitive dependencies
  pub total_non_rust_loc: u64,

  // number of rust lines-of-code, including transitive dependencies
  pub total_rust_loc: u64,

  // number of lines of unsafe code, including transitive dependencies
  pub total_unsafe_loc: u64,
}

/*
should be implemented client side no?
impl PackageRisk {
    /// risk_score computes a weighted score based on the analysis of the crate.
    fn risk_score(&self) -> u64 {
        let mut risk_score = self.total_third_deps * 5000;
        risk_score += self.loc;
        risk_score += self.unsafe_loc * 5000;
        risk_score
    }
}
*/

/// AnalysisResult contains the result of all the analyzed packages.
/// (It can potentially be used by the served webpage, so it uses Arc and RwLock.)
#[derive(Serialize, Deserialize, Default)]
pub struct AnalysisResult {
    main_dependencies: HashSet<PackageId>,
    packages_risk: HashMap<PackageId, PackageRisk>,
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
    repo: &str
  ) -> Option<u64> {
    // is this a github repo?
    let re = Regex::new(r"github\.com/([a-zA-Z0-9._-]*/[a-zA-Z0-9._-]*)").unwrap();
    let caps = re.captures(repo);
    caps
      .and_then(|caps| caps.get(1) )
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

fn get_root_importers(package_graph: &PackageGraph, root_crates: &HashSet<&PackageId>, dependency: &PackageId) -> Vec<PackageId> {
  let root_importers = package_graph.select_reverse(vec![dependency]).unwrap();
  let root_importers = root_importers.into_iter_metadatas(Some(DependencyDirection::Reverse));
  let root_importers: Vec<&PackageMetadata> = root_importers
    .filter(|pkg_metadata| root_crates.contains(&pkg_metadata.id())) // a root crate is an importer
    .collect();
  let root_importers = root_importers.iter().map(|pkg_metadata| pkg_metadata.id().clone()).collect();
  root_importers
}


fn get_total_loc(analysis_result: &mut HashMap<PackageId, PackageRisk>, package_id: &PackageId) -> (u64, u64, u64) {
  let mut total_non_rust_loc = 0;
  let mut total_rust_loc = 0;
  let mut total_unsafe_loc = 0;

  // obtain transitive dependencies (sorted in reverse order)
  let package_risk = analysis_result.get(package_id).unwrap();
  let transitive_dependencies = package_risk.transitive_dependencies.clone();
  for transitive_dependency in transitive_dependencies {
    // calculate its total LOC first
    let package_risk = analysis_result.get(&transitive_dependency).unwrap();
    if package_risk.total_rust_loc == 0 {
      let mut total_non_rust_loc = 0;
      let mut total_rust_loc = 0;
      let mut total_unsafe_loc = 0;
      let transitive_dependencies = &package_risk.transitive_dependencies;
      for transitive_dependency in transitive_dependencies {
        let package_risk = analysis_result.get(&transitive_dependency).unwrap();
        if package_risk.total_rust_loc == 0 {
          unreachable!(); // because the dependencies are given in reverse order
        }
        total_non_rust_loc += package_risk.total_non_rust_loc;
        total_rust_loc += package_risk.total_rust_loc;
        total_unsafe_loc += package_risk.total_unsafe_loc;
      }
      let mut package_risk = analysis_result.get_mut(&transitive_dependency).unwrap();
      package_risk.total_non_rust_loc = total_non_rust_loc;
      package_risk.total_rust_loc = total_rust_loc;
      package_risk.total_unsafe_loc = total_unsafe_loc;
    }
    // add to the total
    let package_risk = analysis_result.get(package_id).unwrap();
    total_non_rust_loc += package_risk.total_non_rust_loc;
    total_rust_loc += package_risk.total_rust_loc;
    total_unsafe_loc += package_risk.total_unsafe_loc;
  }

  //
  (total_non_rust_loc, total_rust_loc, total_unsafe_loc)
}

fn create_or_update_dependency(analysis_result: &mut HashMap<PackageId, PackageRisk>, dep_link: &DependencyLink) {
  match analysis_result.entry(dep_link.to.id().to_owned()) {
    Entry::Occupied(mut entry) => {
      let package_risk = entry.get_mut();
      package_risk.versions.insert(dep_link.to.version().to_string());
    }
    Entry::Vacant(entry) => {
      let mut package_risk = PackageRisk::default();
      package_risk.name = dep_link.to.name().to_owned();
      package_risk.versions.insert(dep_link.to.version().to_string());
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

  // First pass analyze
  // ------------------
  //

  for (package_id, package_risk) in analysis_result.iter_mut() {

    // .transitive_dependencies
    package_risk.transitive_dependencies = package_graph
      .select_forward(std::iter::once(package_id))
      .unwrap()
      .into_iter_links(Some(DependencyDirection::Reverse))
      .map(|package_id| package_id.to.id().to_owned())
      .collect();

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

    // .total_new_third_deps

    // .loc

    // .rust_loc

    // .unsafe_loc

    // .total_...
    if package_risk.transitive_dependencies.len() == 0 {
      package_risk.total_non_rust_loc = package_risk.non_rust_loc;
      package_risk.total_rust_loc = package_risk.rust_loc;
      package_risk.total_unsafe_loc = package_risk.unsafe_loc;
    }

    // .stargazers_count
    if let Some(repo) = &package_risk.repo {
      let stars = GithubResponse::get_github_stars(http_client.clone(), github_token, &repo);
      if let Some(stars) = stars {
        package_risk.stargazers_count = stars;
      }
    }
  }

  // Second pass analyze
  // -------------------
  // we can now compute stats that include a dependency's transitive dependencies
  //

  let package_ids: Vec<PackageId> = analysis_result.keys().map(|x| x.to_owned()).collect();
  for package_id in package_ids {
    let (total_non_rust_loc, total_rust_loc, total_unsafe_loc) = get_total_loc(&mut analysis_result, &package_id);

    let package_risk = analysis_result.get_mut(&package_id).unwrap();
    package_risk.total_non_rust_loc = total_non_rust_loc;
    package_risk.total_rust_loc = total_rust_loc;
    package_risk.total_unsafe_loc = total_unsafe_loc;
  }

  //
  Ok((main_dependencies, analysis_result))
}

/*
fn loc_and_everyhing() {
  // TODO: use WalkParallel?
  let walker = ignore::WalkBuilder::new(package_path).build();
  for result in walker {
    let file = result.unwrap();
    if !file.file_type().unwrap().is_file() {
      continue; // TODO: we ignore symlink here, do we want this?
    }
    let filepath = match file.path().to_str() {
      Some(x) => x,
      None => {
        eprintln!("couldn't convert the path to string {:?}", file.path());
        return;
      }
    };
    if filepath.contains("test") {
      continue; // TODO: this is a ghetto way of ignore tests
    }

    // look for all lines of code (not just rust)
    let lang = loc::lang_from_ext(filepath);
    if lang != loc::Lang::Unrecognized {
      let count = loc::count(filepath);
      // update
      package_risk.loc += u64::from(count.code);
      if lang == loc::Lang::Rust {
        package_risk.rust_loc += u64::from(count.code);
      }
    }

    // look for unsafe lines of code (not including tests)
    if let Ok(res) = geiger::find_unsafe_in_file(
      std::path::Path::new(filepath),
      geiger::IncludeTests::No,
    ) {
      // update
      // TODO: this gives bad results for some reason
      let mut unsafe_loc = res.counters.functions.unsafe_;
      unsafe_loc += res.counters.exprs.unsafe_;
      unsafe_loc += res.counters.item_impls.unsafe_;
      unsafe_loc += res.counters.item_traits.unsafe_;
      unsafe_loc += res.counters.methods.unsafe_;
      package_risk.unsafe_loc += unsafe_loc;
    }
  }
}
*/