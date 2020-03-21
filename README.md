# Cargo-dephell

This is work-in-progress.
It's heavily biased towards the libra codebase (where we have a workspace, we don't have internal crates that are not listed in the workspace, we don't care about the rust edition of dependencies too much, etc.)

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

## Limitations

* Nothing is really accurate
* LOC is not accurate in general (features, tests, etc.)
  - we ignore filepath that have the word "test" in it...
* all features of a dependency are imported
* even if we import only the right features, results are going to be not-so-accurate because different dependencies can import the same dependency but with different feature
* if several versions of a dependency are imported, the results are obtained via the first dependency the program encounters (this is because the repository could change, the LOC, etc.)

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