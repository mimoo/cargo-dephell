use askama::Template;
use clap::{App, Arg};
use guppy::graph::PackageGraph;
use guppy::{MetadataCommand, PackageId};
use std::fs::File;
use std::io::prelude::*;

#[derive(Template)]
#[template(path = "list.html")]
struct HtmlList<'a, 'b> {
    packages: Vec<&'a PackageRisk<'b>>,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct PackageRisk<'a> {
    name: &'a str,
    is_dev: bool,

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
}

impl PackageRisk<'_> {
    fn risk_score(&self) -> u64 {
        let mut risk_score = self.total_third_deps * 5000;
        risk_score += self.loc;
        risk_score += self.unsafe_loc * 5000;
        risk_score
    }
}

fn main() {
    // parse arguments
    let matches = App::new("cargo-dephell")
        .version("1.0")
        .author("David W. <davidwg@fb.com>")
        .about("Risk management for third-party dependencies")
        .arg(
            Arg::with_name("manifest-path")
                .short("m")
                .long("manifest-path")
                .value_name("PATH")
                .help("Sets the path to the Cargo.toml to analyze")
                .takes_value(true)
                .default_value("./Cargo.toml"),
        )
        .arg(
            Arg::with_name("html-output")
                .short("o")
                .long("html-output")
                .takes_value(true)
                .help("prints the output as HTML (default JSON)"),
        )
        .get_matches();

    let manifest_path = matches
        .value_of("manifest-path")
        .expect("must provide a manifest-path");

    let mut cmd = MetadataCommand::new();
    cmd.manifest_path(manifest_path);

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
    let root_deps = package_graph.workspace().member_ids();
    let root_deps: HashSet<&PackageId> = HashSet::from_iter(root_deps);

    // find all direct dependencies
    use std::collections::HashMap;
    let mut direct_deps: HashMap<&PackageId, PackageRisk> = HashMap::new();
    for package_id in package_graph.package_ids() {
        // ignore root dependencies
        if root_deps.contains(package_id) {
            continue;
        }
        // unwrap: if it's not a root deps, it is being imported
        let importers = package_graph.reverse_dep_links(package_id).unwrap();
        for dependency_link in importers {
            // it is imported by a root dependency
            if root_deps.contains(dependency_link.from.id()) {
                // metadata
                let mut package_risk = PackageRisk::default();
                package_risk.name = dependency_link.edge.dep_name();
                package_risk.is_dev = dependency_link.edge.dev_only();
                // insert
                direct_deps.insert(package_id, package_risk);
                break;
            }
        }
    }

    // rank every direct dependency
    for (direct_dep, package_risk) in direct_deps.iter_mut() {
        // find out every transitive dependencies
        let mut to_analyze = HashSet::new();
        to_analyze.insert(*direct_dep);
        for possible_dep in package_graph.package_ids() {
            // ignore root dependencies
            if root_deps.contains(possible_dep) {
                continue;
            }
            if package_graph.depends_on(*direct_dep, possible_dep).unwrap() {
                to_analyze.insert(possible_dep);
            }
        }
        package_risk.total_third_deps = (to_analyze.len() - 1) as u64;

        // analyze every dependency and store result in direct_dep
        for dep in to_analyze {
            let package_metadata = package_graph.metadata(dep).unwrap();
            // get path to dependency
            let package_path = package_metadata.manifest_path().parent().unwrap();

            // TODO: use WalkParallel?
            let walker = ignore::WalkBuilder::new(package_path).build();
            for result in walker {
                let file = result.unwrap();
                if !file.file_type().unwrap().is_file() {
                    continue; // TODO: we ignore symlink here, do we want this?
                }
                let filepath = file.path().to_str().unwrap();
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
