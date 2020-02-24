# Cargo-dephell

This is work-in-progress.

The tool currently analyzes your crate, and prints out its list of direct dependencies sorted by risk.
The risk is calculated based on:

* Total number of (transitive) dependencies a dependency end up importing.
* Total number of lines-of-code of these dependencies.
* Total number of unsafe lines-of-code of these dependencies.

Other metrics we are considering:

* Total number of new (transitive) dependencies added by a dependency. E.g. without dependency X, we could get rid of Y dependencies.
* Is the code on the given github.com the same as the code uploaded on crates.io?
* How many stars does the github repository have?
* When was the last commit made? Or last version released?

## Usage

```sh
cargo run -- --manifest-path .../Cargo.toml
```
