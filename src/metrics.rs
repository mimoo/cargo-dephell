use guppy::graph::{DependencyDirection, PackageGraph, PackageMetadata};
use guppy::PackageId;
use serde::Deserialize;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use crate::analysis::PackageRisk;

//
// Analysis Functions
// ==================
//

/// get number of stars a repo has
pub fn get_github_stars(
  http_client: reqwest::blocking::Client,
  github_token: Option<(&str, &str)>,
  repo: &str,
) -> Option<u64> {
  #[derive(Deserialize, Debug)]
  pub struct GithubResponse {
    #[serde(rename = "stargazers_count")]
    stargazers_count: u64,
  }

  // create request to github API
  let request_url = format!(
    "https://api.github.com/repos/{}",
    repo.trim_end_matches(".git")
  );
  let mut request = http_client.get(&request_url);

  // if we have a github token, use it
  if let Some((username, token)) = github_token {
    request = request.basic_auth(username, Some(token));
  }
  // send the request
  let resp = match request.send() {
    Err(err) => {
      eprintln!("{}", err);
      return None;
    }
    Ok(resp) => resp,
  };

  if !resp.status().is_success() {
    eprintln!("dephell: github request failed");
    eprintln!("status: {}", resp.status());
    eprintln!("text: {:?}", resp.text());
    return None;
  }
  let resp: reqwest::Result<GithubResponse> = resp.json();
  match resp {
    Ok(x) => Some(x.stargazers_count),
    Err(err) => {
      eprintln!("dephell: {}", err);
      None
    }
  }
}

/// get number of maintainers in the last 6 months
pub fn get_active_maintainers(
  http_client: reqwest::blocking::Client,
  github_token: Option<(&str, &str)>,
  repo: &str,
) -> Option<u64> {
  #[derive(Deserialize, Debug)]
  pub struct Author {
    email: String,
  }
  #[derive(Deserialize, Debug)]
  pub struct Commit {
    author: Author,
  }
  #[derive(Deserialize, Debug)]
  pub struct CommitInfo {
    commit: Commit,
  }
  // create request to crates.io API
  let six_months_ago = chrono::Utc::now()
    .checked_sub_signed(chrono::Duration::weeks(4 * 6)) // 6 months
    .unwrap()
    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
  let request_url = format!(
    "https://api.github.com/repos/{}/commits?since={}",
    repo.trim_end_matches(".git"),
    six_months_ago,
  );
  let mut request = http_client.get(&request_url);
  // if we have a github token, use it
  if let Some((username, token)) = github_token {
    request = request.basic_auth(username, Some(token));
  }
  // send the request
  let resp = match request.send() {
    Err(err) => {
      eprintln!("{}", err);
      return None;
    }
    Ok(resp) => resp,
  };
  // parse response
  if !resp.status().is_success() {
    eprintln!("dephell: crates.io request failed");
    eprintln!("status: {}", resp.status());
    eprintln!("text: {:?}", resp.text());
    return None;
  }
  let resp: reqwest::Result<Vec<CommitInfo>> = resp.json();
  match resp {
    Err(err) => {
      eprintln!("dephell: {}", err);
      None
    }
    Ok(commit_infos) => {
      let mut commiters = HashSet::new();
      for commit_info in commit_infos {
        commiters.insert(commit_info.commit.author.email);
      }
      Some(commiters.len() as u64)
    }
  }
}

/// CratesIoResponse is used to parse the response from crates.io
pub fn get_crates_io_dependent(
  http_client: reqwest::blocking::Client,
  crate_name: &str,
) -> Option<u64> {
  #[derive(Deserialize, Debug)]
  struct Meta {
    total: u64,
  }
  #[derive(Deserialize, Debug)]
  pub struct Response {
    meta: Meta,
  }
  // create request to crates.io API
  let request_url = format!(
    "https://crates.io/api/v1/crates/{}/reverse_dependencies",
    crate_name,
  );
  let request = http_client.get(&request_url);
  // send the request
  let resp = match request.send() {
    Err(err) => {
      eprintln!("{}", err);
      return None;
    }
    Ok(resp) => resp,
  };
  // parse response
  if !resp.status().is_success() {
    eprintln!("dephell: crates.io request failed");
    eprintln!("query: {}", request_url);
    eprintln!("status: {}", resp.status());
    eprintln!("text: {:?}", resp.text());
    return None;
  }
  let resp: reqwest::Result<Response> = resp.json();
  match resp {
    Err(err) => {
      eprintln!("dephell: {}", err);
      None
    }
    Ok(x) => Some(x.meta.total),
  }
}

/// CratesIoResponse is used to parse the response from crates.io
pub fn get_crates_io_last_updated(
  http_client: reqwest::blocking::Client,
  crate_name: &str,
) -> Option<String> {
  #[derive(Deserialize, Debug)]
  struct Crate {
    updated_at: String,
  }
  #[derive(Deserialize, Debug)]
  pub struct Response {
    #[serde(rename = "crate")]
    crate_: Crate,
  }
  // create request to crates.io API
  let request_url = format!("https://crates.io/api/v1/crates/{}", crate_name,);
  let request = http_client.get(&request_url);
  // send the request
  let resp = match request.send() {
    Err(err) => {
      eprintln!("{}", err);
      return None;
    }
    Ok(resp) => resp,
  };
  // parse response
  if !resp.status().is_success() {
    eprintln!("dephell: crates.io request failed");
    eprintln!("status: {}", resp.status());
    eprintln!("text: {:?}", resp.text());
    return None;
  }
  let resp: reqwest::Result<Response> = resp.json();
  match resp {
    Err(err) => {
      eprintln!("dephell: {}", err);
      None
    }
    Ok(resp) => {
      let updated_at = resp.crate_.updated_at;
      let formatted_date = chrono::DateTime::parse_from_rfc3339(&updated_at)
        .unwrap()
        .format("%Y-%m-%d")
        .to_string();
      Some(formatted_date)
    }
  }
}

/// obtains all root crates that end up importing this dependency
pub fn get_root_importers(
  package_graph: &PackageGraph,
  root_crates: &HashSet<PackageId>,
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

/// obtains all the dependencies that are introduced by this dependency, and this dependency only
pub fn get_exclusive_deps(
  package_graph: &PackageGraph,
  root_crates: &HashSet<PackageId>,
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
  let new_all = package_graph.select_forward(root_crates.iter()).unwrap();
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

/// counts the lines-of-code of all the given files
pub fn get_loc(package_risk: &mut PackageRisk, dependency_files: &HashSet<String>) {
  for dependency_file in dependency_files {
    // look for all lines of code (not just rust)
    let lang = loc::lang_from_ext(dependency_file);
    if lang != loc::Lang::Unrecognized {
      let count = loc::count(dependency_file);
      // update LOC
      // TODO: compute the .loc from all files, not from .d
      package_risk.loc += u64::from(count.code);
      if lang == loc::Lang::Rust {
        package_risk.rust_loc += u64::from(count.code);
      }
    }
  }
}

/// counts the unsafe lines-of-code of all the given rust files
pub fn get_unsafe(package_risk: &mut PackageRisk, dependency_files: &HashSet<String>) {
  for dependency_file in dependency_files {
    let dependency_path = Path::new(dependency_file);
    if dependency_path.extension() != Some(OsStr::new("rs")) {
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

/// parses the dep-info files that contain all the files relevant to the compilation of a dependency (these files are like Makefiles)
// TODO: what to do about libraries linked via bindings
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
          file.push_str(deps.next().expect("malformed dep-info format, trailing \\"));
        }
        dependency_files.insert(file);
      }
    }
  }
  dependency_files
}

/// retrieves every single file in the folder of the dependency
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
        eprintln!(
          "dephell: couldn't convert the path to string {:?}",
          file.path()
        );
        continue;
      }
    };
  }
  //
  dependency_files
}

/// obtains a dependency's files (might be accurate or not)
pub fn get_dependency_files(
  package_name: &str,
  manifest_path: &Path,
  target_dir: &Path,
) -> (bool, HashSet<String>) {
  use glob::glob;

  // find the dep-info file for that dependency
  let mut dep_files_path = target_dir.to_path_buf();
  dep_files_path.push("debug/deps");
  let without_underscore_name = package_name.replace("-", "_");
  let dependency_file = format!("{}-*.d", without_underscore_name);
  dep_files_path.push(dependency_file);
  let dep_files_path = glob(dep_files_path.to_str().unwrap()).unwrap().next();
  match dep_files_path {
    // we found a dep-info file
    Some(glob_result) => {
      let dep_files_path = glob_result.unwrap();
      let dependency_files = parse_rustc_dep_info(dep_files_path.as_path());
      (true, dependency_files)
    }
    // this dependency is not part of our target+features: let's do it the old fashion way
    None => {
      // eprintln!("dephell: no dep-info file found for {}", package_name);
      let package_path = manifest_path.parent().unwrap();
      let dependency_files = get_every_file_in_folder(package_path);
      (false, dependency_files)
    }
  }
}
