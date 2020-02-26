use askama::Template;
use clap::{App, Arg};
use guppy::graph::{DependencyDirection, PackageGraph};
use guppy::{MetadataCommand, PackageId};
use regex::Regex;
use std::fs::File;
use std::io::prelude::*;

#[derive(Template)]
#[template(path = "list.html")]
struct HtmlList<'a, 'b> {
    path: &'b str,
    packages: Vec<&'a PackageRisk<'a>>,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct PackageRisk<'a> {
    name: &'a str,
    is_dev: bool,
    repo: &'a str,

    // total number of transitive third party dependencies imported
    // by this dependency (not including this dependency)
    total_third_deps: u64,
    // total number of transitive third party dependencies imported
    // by this dependency, and only this dependency
    total_new_third_deps: u64,
    // number of lines-of-code for this dependency as well as
    // all the third party dependencies it imports
    loc: u64,
    // number of lines of unsafe code for this dependency as
    // well as all the third party dependencies it imports
    unsafe_loc: u64,
    // number of github stars, if any
    stargazers_count: u64,
}

impl PackageRisk<'_> {
    fn risk_score(&self) -> u64 {
        let mut risk_score = self.total_third_deps * 5000;
        risk_score += self.loc;
        risk_score += self.unsafe_loc * 5000;
        risk_score
    }
}

#[derive(serde::Deserialize, Debug)]
struct GithubResponse {
    #[serde(rename = "stargazers_count")]
    stargazers_count: u64,
}

fn main() {
    // parse arguments
    let matches = App::new("cargo-dephell")
        .version("1.0")
        .author("David W. <davidwg@fb.com>")
        .about("Risk management for third-party dependencies")
        .arg(
            Arg::with_name("manifest-path")
                .help("Sets the path to the Cargo.toml to analyze")
                .short("m")
                .long("manifest-path")
                .takes_value(true)
                .value_name("PATH")
                .default_value("./Cargo.toml"),
        )
        .arg(
            Arg::with_name("html-output")
                .help("prints the output as HTML (default JSON)")
                .short("o")
                .long("html-output")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("github-token")
                .short("g")
                .long("github-token")
                .takes_value(true)
                .value_name("USER:TOKEN")
                .help("allows the CLI to retrieve github repos stats"),
        )
        .arg(
            Arg::with_name("proxy")
                .short("p")
                .long("proxy")
                .takes_value(true)
                .value_name("PROTOCOL://IP:PORT")
                .help("uses a proxy to make external requests to github"),
        )
        .arg(
            Arg::with_name("ignore-workspace")
                .short("i")
                .multiple(true)
                .takes_value(true)
                .value_name("CRATE_NAME")
                .help("can be used multiple times to list workplace crates to ignore"),
        )
        .get_matches();

    let manifest_path = matches
        .value_of("manifest-path")
        .expect("must provide a manifest-path");

    let mut cmd = MetadataCommand::new();
    cmd.manifest_path(manifest_path);

    // create a client to query github (to get # of stars)
    let mut github_client =
        reqwest::blocking::ClientBuilder::new().user_agent("mimoo/cargo-dephell");
    // (potentially use a proxy)
    if let Some(proxy) = matches.value_of("proxy") {
        let reqwest_proxy = match reqwest::Proxy::all(proxy) {
            Ok(x) => x,
            Err(err) => {
                eprintln!("{}", err);
                return;
            }
        };
        github_client = github_client.proxy(reqwest_proxy);
    }
    let github_client = github_client.build().unwrap();

    // construct graph from metadata command
    let package_graph = match PackageGraph::from_command(&mut cmd) {
        Ok(x) => x,
        Err(err) => {
            eprintln!("{}", err);
            return;
        }
    };

    // get all internal dependencies
    // (either main package or members of the workspace)
    use std::collections::HashSet;
    use std::iter::FromIterator;
    let root_crates = package_graph.workspace().member_ids();
    let mut root_crates: HashSet<&PackageId> = HashSet::from_iter(root_crates);

    // remove workspace crates that we want to ignore
    if let Some(to_ignore) = matches.values_of("ignore-workspace") {
        root_crates = root_crates
            .into_iter()
            .filter(|pkg_id| {
                let package_metadata = package_graph.metadata(pkg_id).unwrap();
                let package_name = package_metadata.name();
                let to_ignore: Vec<&str> = to_ignore.clone().collect();
                !to_ignore.contains(&package_name)
            })
            .collect();
    }

    // find all direct dependencies
    use std::collections::HashMap;
    let mut direct_deps: HashMap<&PackageId, PackageRisk> = HashMap::new();
    for package_id in package_graph.package_ids() {
        // ignore root dependencies
        if root_crates.contains(package_id) {
            continue;
        }
        // who's importing it?
        let importers = package_graph.reverse_dep_links(package_id).unwrap();
        for dependency_link in importers {
            // it is imported by a root dependency, add it
            if root_crates.contains(dependency_link.from.id()) {
                let mut package_risk = PackageRisk::default();
                package_risk.name = dependency_link.edge.dep_name();
                package_risk.is_dev = dependency_link.edge.dev_only();
                if let Some(repo) = dependency_link.to.repository() {
                    package_risk.repo = repo;
                }
                direct_deps.insert(package_id, package_risk);
                break;
            }
        }
    }

    // rank every direct dependency
    for (direct_dep, package_risk) in direct_deps.iter_mut() {
        // check how many root pkgs end up making use of this dependency
        let root_importers = package_graph.select_reverse(vec![*direct_dep]).unwrap();
        let root_importers = root_importers.into_iter_metadatas(Some(DependencyDirection::Reverse));
        package_risk.total_new_third_deps = root_importers.len() as u64;

        // check number of stars on github
        let package_metadata = package_graph.metadata(direct_dep).unwrap();
        if let Some(repo) = package_metadata.repository() {
            let re = Regex::new(r"github\.com/([a-zA-Z0-9._-]*/[a-zA-Z0-9._-]*)").unwrap();
            let caps = re.captures(repo);
            if let Some(caps) = caps {
                if let Some(repo) = caps.get(1) {
                    let request_url = format!("https://api.github.com/repos/{}", repo.as_str());

                    let mut request = github_client.get(&request_url);

                    if let Some(github_token) = matches.value_of("github-token") {
                        let github_token: Vec<&str> = github_token.split(":").collect();
                        if github_token.len() != 2 {
                            eprintln!("wrong github-token, must be of the form username:token");
                            return;
                        }
                        let username = github_token[0];
                        let token = github_token[1];
                        request = request.basic_auth(username, Some(token));
                    }

                    if let Ok(resp) = request.send() {
                        if !resp.status().is_success() {
                            eprintln!("github request failed");
                            eprintln!("request to {}", request_url);
                            eprintln!("status: {}", resp.status());
                            eprintln!("text: {:?}", resp.text());
                        } else {
                            let resp_json: reqwest::Result<GithubResponse> = resp.json();
                            match resp_json {
                                Ok(x) => package_risk.stargazers_count = x.stargazers_count,
                                Err(err) => {
                                    eprintln!("{}", err);
                                }
                            };
                        }
                    }
                }
            }
        }

        // find out every transitive dependencies
        let mut to_analyze = HashSet::new();
        to_analyze.insert(*direct_dep);
        for possible_dep in package_graph.package_ids() {
            // ignore root dependencies
            if root_crates.contains(possible_dep) {
                continue;
            }
            if package_graph.depends_on(*direct_dep, possible_dep).unwrap() {
                to_analyze.insert(possible_dep);
            }
        }
        package_risk.total_third_deps = (to_analyze.len() - 1) as u64;

        // analyze every dependency and store result in direct_dep
        for dep in to_analyze {
            // get path to dependency
            let package_metadata = package_graph.metadata(dep).unwrap();
            let package_path = package_metadata.manifest_path().parent().unwrap();

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

                // look for safe lines of code
                let lang = loc::lang_from_ext(filepath);
                if lang != loc::Lang::Unrecognized {
                    let count = loc::count(filepath);
                    // update
                    package_risk.loc += u64::from(count.code);
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
    }

    // sort result (via Btrees)
    use std::collections::btree_map::BTreeMap;
    let mut deps_by_risk: BTreeMap<u64, &PackageRisk> = BTreeMap::new();
    for (_, package_risk) in direct_deps.iter() {
        let risk = package_risk.risk_score();
        deps_by_risk.insert(risk, package_risk);
    }
    let deps_by_risk_reverted: Vec<&PackageRisk> =
        deps_by_risk.iter().rev().map(|item| *item.1).collect();

    match matches.value_of("html-output") {
        None => {
            // print result order by risk_score DESCENDING
            let j = serde_json::to_string(&deps_by_risk_reverted).unwrap();
            println!("{}", j);
        }
        Some(html_output) => {
            let html_page = HtmlList {
                path: manifest_path,
                packages: deps_by_risk_reverted,
            };
            let mut file = match File::create(html_output) {
                Ok(x) => x,
                Err(err) => {
                    eprintln!("{}", err);
                    return;
                }
            };
            let _ = write!(&mut file, "{}", html_page.render().unwrap()).unwrap();
            println!("html output saved at {}", html_output);
        }
    };
}
