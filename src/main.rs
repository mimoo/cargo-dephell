use guppy::graph::PackageGraph;
use guppy::{MetadataCommand, PackageId};

const USAGE: &str = "
    Cargo dephell

    Usage:
        cargo dephell [--manifest-path PATH]
";

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

/*
fn get_path(package_name: &str, package_version: &str) -> String {
    use std::path::PathBuf;
    // figure out path of crate source
    let mut path = PathBuf::from(CRATES_PATH);
    let mut folder_name = package_name.to_string();
    folder_name.push_str("-");
    folder_name.push_str(package_version);
    path.push(folder_name);
    // return as string
    path.into_os_string().into_string().unwrap()
}
*/

fn main() {
    // get manifest path of project
    let mut cmd = MetadataCommand::new();
    let mut args = std::env::args().skip_while(|val| !val.starts_with("--manifest-path"));
    match args.next() {
        Some(ref p) if p == "--manifest-path" => {
            cmd.manifest_path(args.next().unwrap());
        }
        Some(p) => {
            cmd.manifest_path(p.trim_start_matches("--manifest-path="));
        }
        None => {
            eprintln!("{}", USAGE);
            return;
        }
    };

    // construct graph from metadata command
    let package_graph = PackageGraph::from_command(&mut cmd).expect("command should work");

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
                //            let package_metadata = package_graph.metadata(package_id).unwrap();
                //            package_risk.package_name = package_metadata.name();
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

    // print result order by risk_score DESCENDING
    let deps_by_risk_reverted: Vec<&PackageRisk> =
        deps_by_risk.iter().rev().map(|item| *item.1).collect();
    let j = serde_json::to_string(&deps_by_risk_reverted).unwrap();
    println!("{}", j);
}
