# Cargo dephell

![cargo dephell](https://i.imgur.com/NUNFPfC.png)

**Cargo dephell** is a tool to analyze the third-party dependencies imported by a rust crate or rust workspace.
It makes use of [guppy](https://crates.io/crates/guppy) to parse dependencies, [geiger](https://crates.io/crates/guppy) to find unsafe code and [loc](https://crates.io/crates/loc) to count the number of lines of code.
Cargo dephell is heavily biased towards the libra codebase (where we have a workspace, we don't have internal crates that are not listed in the workspace, we don't care about the rust edition of dependencies too much, etc.)

## Usage

**Make sure you've built your crate or workspace first.**

Just run the program on the relevant `Cargo.toml` and output the result to an HTML file:

```sh
cargo run -- --manifest-path ./Cargo.toml --o analysis_results.html
```

Note that you might need a personnal access token to query the Github API. You can get one easily by following these steps:

Go to your github *Settings*:

![github settings](https://i.imgur.com/X026V85.png)

Go to the *Developer settings*:

![github dev settings](https://i.imgur.com/ldj82nR.png)

Go to the *Personall Access Token* page and click on the *Generate new token* button:

![github personal access token](https://i.imgur.com/BpqGdoE.png)

Once there, just:

* give it a name
* don't check any boxes
* generate the token

once you have it, pass it as:

```
cargo run -- --manifest-path ./Cargo.toml -o analysis_results.html --github-token <username>:<token>
```

so for example:

```
cargo run -- --manifest-path ./Cargo.toml -o analysis_results.html --github-token mimoo:3902jfoiewjf130fjeowijfw
```

## Limitations

Keep in mind that this is a best-effort way to assess third party dependencies, this is for a number of reasons that we document here:

* The transitives dependencies imported by a dependency are not feature-dependent, which is deceiving to say the least (this should be fixed soon).
* The *lines of code* metric is not accurate in general as it includes EVERY files of the crate folder.
* The *lines of rust code* metric is not accurate in general because it includes EVERY .rs files of the crate folder, and for every file it includes every features, tests, etc.
* If several versions of a dependency are imported, the results are computed on the first dependency we encounter. This is deceiving because versions can change the repository, the lines of code, the dependencies they import, etc.

## Roadmap

If you want to help:

1. Check if the code on the given repository is the same as the code uploaded on crates.io
1. Display the date of the last commit, or last version released, of a dependency.
1. Add an `AUDIT.toml` file to track who has audited what SHA-1 commit of which repository.
1. Add feature-sensisite support (blocked on guppy at the moment). Furthermore it would be great if we can dynamically enable and disable features in the HTML output.
1. Display the dependency graph with dot (and d3).
1. Add the number of committers in the last 12 months
1. Add number of importers from crates.io
1. Add audits of crates (from https://github.com/RustSec/cargo-audit/blob/master/src/auditor.rs#L4)
