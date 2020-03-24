// needed for rocket
#![feature(proc_macro_hygiene, decl_macro)] // Nightly-only language features needed by Rocket

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use askama::Template;
use clap::{App, Arg};
use guppy::PackageId;
use serde::{Deserialize, Serialize};

mod analysis;

//
// HTML Stuff
// ==========
//

#[derive(Template)]
#[template(path = "list.html", escape = "none")]
struct HtmlList {
    name: String,
    json_result: String,
}

//
// JSON Stuff
// ==========
//

#[derive(Serialize, Deserialize)]
struct JsonResult {
    main_dependencies: HashSet<PackageId>,
    analysis_result: HashMap<PackageId, analysis::PackageRisk>,
}

//
// Main
// ====
//

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
                .value_name("PATH"),
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

    // get metadata from manifest path
    let manifest_path = matches
        .value_of("manifest-path")
        .map(|s| s.to_owned())
        .unwrap_or_else(|| {
            let mut current_dir = PathBuf::from(std::env::current_dir().unwrap());
            current_dir.push("Cargo.toml");
            current_dir.to_str().unwrap().to_owned()
        });

    // pretty hello world :>
    let pretty_line = "=========================";
    println!("{}", pretty_line);
    println!("~~ CARGO DEPHELL ~~");
    println!("{}", pretty_line);

    // parse github token (if given)
    let github_token = matches.value_of("github-token").and_then(|github_token| {
        let github_token: Vec<&str> = github_token.split(":").collect();
        if github_token.len() != 2 {
            eprintln!("wrong github-token, must be of the form username:token");
            return None;
        }
        let username = github_token[0];
        let token = github_token[1];
        Some((username, token))
    });

    // create an HTTP client (used for example to query github API to get # of stars)
    let mut http_client = reqwest::blocking::ClientBuilder::new().user_agent("mimoo/cargo-dephell");
    if let Some(proxy) = matches.value_of("proxy") {
        let reqwest_proxy = match reqwest::Proxy::all(proxy) {
            Ok(x) => x,
            Err(err) => {
                eprintln!("{}", err);
                return;
            }
        };
        http_client = http_client.proxy(reqwest_proxy);
    }
    let http_client = http_client.build().unwrap();

    // parse dependencies to ignore
    let to_ignore = matches.values_of("ignore-workspace");
    let to_ignore: Option<Vec<&str>> = to_ignore.map(|x| x.collect());
    // do the analysis
    let result = analysis::analyze_repo(&manifest_path, http_client, github_token, to_ignore);
    let (main_dependencies, analysis_result) = match result {
        Err(err) => {
            eprintln!("{}", err);
            return;
        }
        Ok(x) => x,
    };

    // convert result to JSON
    let json_result = JsonResult {
        main_dependencies,
        analysis_result,
    };
    let json_result = serde_json::to_string(&json_result).unwrap();

    // print out result
    use std::fs::File;
    use std::io::prelude::*;
    match matches.value_of("html-output") {
        None => {
            println!("{}", json_result);
        }
        Some(html_output) => {
            let name = std::path::Path::new(&manifest_path)
                .parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            let html_page = HtmlList {
                name: name,
                json_result: json_result,
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
    //
}
