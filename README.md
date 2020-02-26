# Cargo-dephell

This is work-in-progress.

The tool currently analyzes your crate, and prints out its list of direct dependencies sorted by risk.
The risk is calculated based on:

* Total number of (transitive) dependencies a dependency end up importing.
* Total number of lines-of-code of these dependencies.
* Total number of unsafe lines-of-code of these dependencies.
* Number of workplace packages making use of the dependency
* Number of github stars

## Usage

```sh
cargo run -- --manifest-path .../Cargo.toml
```

## Things that would be nice to have

Other metrics we are considering:

* Total number of new (transitive) dependencies added by a dependency. E.g. without dependency X, we could get rid of Y dependencies.
* Is the code on the given github.com the same as the code uploaded on crates.io?
* When was the last commit made? Or last version released?
* Amount of C code imported (or other languages)

Other stuff:

* click on a dependency, and get the same page
* enable and disable features dynamically (currently every feature is grabbed)
* display dependency graphs