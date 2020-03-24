// what I need
// - function to collect real rs files in-use
// - then I can use this with geiger and loc
// - actually, what about non-rs files like C files?
//   - in this example the .d file have the list.html
//
//
//
// OK so idea:
// - simply `cargo clean` & `cargo build` the manifest-path
//  - in a temporary OUT_DIR (/tmp)
// - then find out the .d file for each dependency
// - then analyze the files directly 
//
//
// - this doesn't seem to work with build.rs and C libraries
// - can I just check if a build.rs exist? Or if there is [[build]] in cargo.toml?
//  - maybe guppy does it
// - what about C bindings?
// - I can maybe see this in compilation messages...
//  - or if cc/bindgen are imported
// - or #[extern(C)] stuff!!!!!!!!!

use cargo::core::compiler::Executor;
use cargo::core::package_id::PackageId;
use cargo::core::manifest::Target;
use cargo::core::compiler::CompileMode;
use cargo::util::{ProcessBuilder, errors::CargoResult};

use std::collections::HashSet;
use std::path::{PathBuf, Path};
use std::sync::{Arc, Mutex};

/// To resolve the rust files used in the crate, we use the rust compiler.
/// To extract the files, we give it our custom executor.

#[derive(Debug, Default)]
struct CustomExecutorInnerContext {
  rs_file_args: HashSet<PathBuf>,
  out_dir_args: HashSet<PathBuf>,
}
#[derive(Debug)]
enum CustomExecutorError {
  OutDirKeyMissing(String),
  OutDirValueMissing(String),
  InnerContextMutex(String),
  Io(std::io::Error, std::path::PathBuf),
}
impl std::error::Error for CustomExecutorError {}
impl std::fmt::Display for CustomExecutorError {
  fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
    std::fmt::Debug::fmt(self, f)
  }
}
// About the Executor trait:
// > A glorified callback for executing calls to rustc. Rather than calling rustc directly, we'll use an Executor, giving clients an opportunity to intercept the build calls.
struct CustomExecutor {
  crate_name: String,
  inner_ctx: Arc<Mutex<CustomExecutorInnerContext>>,
}
impl Executor for CustomExecutor {
  fn exec(
    &self,
    cmd: ProcessBuilder,
    id: PackageId,
    target: &Target,
    mode: CompileMode,
    _on_stdout_line: &mut dyn FnMut(&str) -> CargoResult<()>,
    _on_stderr_line: &mut dyn FnMut(&str) -> CargoResult<()>,
  ) -> CargoResult<()> {

//    println!("cmd: {:?}", cmd);
    // cmd: ProcessBuilder { program: "rustc", args: ["--crate-name", "C", "--edition=2018", "src/lib.rs", "--error-format=json", "--json=diagnostic-rendered-ansi", "--crate-type", "lib", "--emit=dep-info,metadata", "-C", "debuginfo=2", "-C", "metadata=4cb22360bc34386a", "-C", "extra-filename=-4cb22360bc34386a", "--out-dir", "/Users/davidwg/Desktop/sample_dephell/C/target/debug/deps", "-C", "incremental=/Users/davidwg/Desktop/sample_dephell/C/target/debug/incremental", "-L", "dependency=/Users/davidwg/Desktop/sample_dephell/C/target/debug/deps", "--extern", "common=/Users/davidwg/Desktop/sample_dephell/C/target/debug/deps/libcommon-26f2aa8ec33dcdde.rmeta"], env: {"CARGO_PKG_VERSION_MINOR": Some("1"), "CARGO_PKG_VERSION_PATCH": Some("0"), "CARGO": Some("/Users/davidwg/Work/cargo-dephell/target/debug/cargo-dephell"), "CARGO_PKG_VERSION": Some("0.1.0"), "CARGO_PKG_HOMEPAGE": Some(""), "CARGO_MANIFEST_DIR": Some("/Users/davidwg/Desktop/sample_dephell/C"), "CARGO_PKG_VERSION_MAJOR": Some("0"), "CARGO_PKG_VERSION_PRE": Some(""), "CARGO_PKG_NAME": Some("C"), "CARGO_PKG_DESCRIPTION": Some(""), "CARGO_PKG_AUTHORS": Some("David Wong <davidwg@calibra.com>"), "CARGO_PKG_REPOSITORY": Some(""), "DYLD_FALLBACK_LIBRARY_PATH": Some("/Users/davidwg/Desktop/sample_dephell/C/target/debug/deps:/Users/davidwg/.rustup/toolchains/nightly-x86_64-apple-darwin/lib:/Users/davidwg/Work/cargo-dephell/target/debug/build/libgit2-sys-12926fa6c157be98/out/build:/Users/davidwg/Work/cargo-dephell/target/debug/build/libnghttp2-sys-143e112126b5a6ce/out/i/lib:/Users/davidwg/Work/cargo-dephell/target/debug/build/libssh2-sys-bdbf0634cf1902f2/out/build:/Users/davidwg/Work/cargo-dephell/target/debug/deps:/Users/davidwg/Work/cargo-dephell/target/debug:/Users/davidwg/.rustup/toolchains/nightly-x86_64-apple-darwin/lib/rustlib/x86_64-apple-darwin/lib:/Users/davidwg/.rustup/toolchains/nightly-x86_64-apple-darwin/lib:/Users/davidwg/lib:/usr/local/lib:/usr/lib")}, cwd: Some("/Users/davidwg/Desktop/sample_dephell/C"), jobserver: Some(Client { inner: Client { read: File { fd: 6, read: true, write: false }, write: File { fd: 7, read: false, write: true } } }), display_env_vars: false }
//    println!("package id: {:?}", id);
    // id: PackageId { name: "C", version: "0.1.0", source: "/Users/davidwg/Desktop/sample_dephell/C" }
//    println!("target: {:?}", target);
    // target: Target { ..: lib_target("C", ["lib"], "/Users/davidwg/Desktop/sample_dephell/C/src/lib.rs", Edition2018) }

    // get args from cmd
    let args = cmd.get_args();
//    println!("cmd args: {:?}", args);
    // cmd args: ["--crate-name", "C", "--edition=2018", "src/lib.rs", "--error-format=json", "--json=diagnostic-rendered-ansi", "--crate-type", "lib", "--emit=dep-info,metadata", "-C", "debuginfo=2", "-C", "metadata=4cb22360bc34386a", "-C", "extra-filename=-4cb22360bc34386a", "--out-dir", "/Users/davidwg/Desktop/sample_dephell/C/target/debug/deps", "-C", "incremental=/Users/davidwg/Desktop/sample_dephell/C/target/debug/incremental", "-L", "dependency=/Users/davidwg/Desktop/sample_dephell/C/target/debug/deps", "--extern", "common=/Users/davidwg/Desktop/sample_dephell/C/target/debug/deps/libcommon-26f2aa8ec33dcdde.rmeta"]
    let mut crate_name = args.iter().map(|s| s.to_str().unwrap()).skip_while(|arg| arg != &"--crate-name");
    crate_name.next(); // skip --crate-name
    if let Some(arg) = crate_name.next() {
      if arg == self.crate_name {
        println!("\n FOUND IT \n");
        println!("{:?}", args);
      }
    }

    // exec cmd
    cmd.exec()?;

    // no error
    Ok(())
  }
}

/// Trigger a `cargo clean` + `cargo check` and listen to the cargo/rustc
/// communication to figure out which source files were used by the build.
pub fn get_rs_files(package_name: String, manifest_path: &Path) -> HashSet<PathBuf> {
  println!("=========\nget_rs_files({}, {:?})", package_name, manifest_path);
  // get information about crate
  let workspace_config = cargo::util::config::Config::default().unwrap();
  let workspace = cargo::core::Workspace::new(manifest_path, &workspace_config).unwrap();

  // cargo clean
  std::process::Command::new("cargo")
    .args(&["clean", "--manifest-path", manifest_path.to_str().unwrap()])
    .output()
    .expect("failed to clean crate");

  // config for cargo check
  let mut compile_config = cargo::util::config::Config::default().unwrap();
  compile_config.configure(
    0,          // verbose
    Some(true), // quiet
    None,       // color
    false,      // frozen
    false,      // locked
    false,      // offline: bool,
    &None,       // target_dir: &Option<PathBuf>,
    &vec![],     // unstable_flags: &[String],
    &vec![],     // cli_config: &[&str]
  ).unwrap();
  let compile_mode = cargo::core::compiler::CompileMode::Build;
  let compiler_option = cargo::ops::CompileOptions::new(
    &compile_config, 
    compile_mode,
  ).unwrap();

  // compile with our custom executor
  // this seems to only write something with src/lib.rs, what about the other files?s
  // TODO: replace this with a hashset directly
  let custom_executor_inner_context = CustomExecutorInnerContext::default();
  let custom_executor_context = Arc::new(Mutex::new(custom_executor_inner_context));
  {
    let custom_executor = CustomExecutor {
      crate_name: package_name,
      inner_ctx: custom_executor_context.clone(),
    };
    let custom_executor: Arc<dyn Executor> = Arc::new(custom_executor);
    cargo::ops::compile_with_exec(
      &workspace, 
      &compiler_option, 
      &custom_executor
    ).unwrap();
  }

  //
  let mut rust_files = HashSet::<PathBuf>::new();

  // looks like we need to find .d files instead    
  // documentation on files produced by compilation: https://github.com/rust-lang/cargo/blob/972b9f55a72e3eae21c826b2f806c753696ef2ec/src/cargo/core/compiler/layout.rs
  // TODO: so do we want to build libra directly and get the .d once for all deps?


  // target/debug/deps/

  /*
  let ws_root = workspace.root().to_path_buf();
  let inner_mutex = Arc::try_unwrap(custom_executor_context).map_err(|_| RsResolveError::ArcUnwrap())?;
  let (rs_files, out_dir_args) = {
    let ctx = inner_mutex.into_inner()?;
    (ctx.rs_file_args, ctx.out_dir_args)
  };
  for out_dir in out_dir_args {
    for ent in WalkDir::new(&out_dir) {
      let ent = ent.map_err(RsResolveError::Walkdir)?;
      if !is_file_with_ext(&ent, "d") {
        continue;
      }
      let deps = parse_rustc_dep_info(ent.path()).map_err(|e| {
        RsResolveError::DepParse(
          e.to_string(),
          ent.path().to_path_buf(),
        )
      })?;
      let canon_paths = deps
        .into_iter()
        .flat_map(|t| t.1)
        .map(PathBuf::from)
        .map(|pb| ws_root.join(pb))
        .map(|pb| {
          pb.canonicalize().map_err(|e| RsResolveError::Io(e, pb))
      });
      for p in canon_paths {
        rust_files.insert(p?);
      }
    }
  }
  for pb in rs_files {
    // rs_files must already be canonicalized
    rust_files.insert(pb);
  }
  */
  rust_files
}

/*
/// Copy-pasted (almost) from the private module cargo::core::compiler::fingerprint.
///
/// TODO: Make a PR to the cargo project to expose this function or to expose
/// the dependency data in some other way.
fn parse_rustc_dep_info(
  rustc_dep_info: &Path,
) -> CargoResult<Vec<(String, Vec<String>)>> {
  let contents = paths::read(rustc_dep_info)?;
  contents
  .lines()
  .filter_map(|l| l.find(": ").map(|i| (l, i)))
  .map(|(line, pos)| {
    let target = &line[..pos];
    let mut deps = line[pos + 2..].split_whitespace();
    let mut ret = Vec::new();
    while let Some(s) = deps.next() {
      let mut file = s.to_string();
      while file.ends_with('\\') {
        file.pop();
        file.push(' ');
        //file.push_str(deps.next().ok_or_else(|| {
          //internal("malformed dep-info format, trailing \\".to_string())
          //})?);
          file.push_str(
            deps.next()
            .expect("malformed dep-info format, trailing \\"),
          );
        }
        ret.push(file);
      }
      Ok((target.to_string(), ret))
    })
    .collect()
  }
  


pub fn build_compile_options<'a>(args: &'a Args, config: &'a cargo::Config) -> CompileOptions<'a> {
  let features = Method::split_features(&args.features.clone().into_iter().collect::<Vec<_>>())
  .into_iter()
  .map(|s| s.to_string());
  let mut opt = CompileOptions::new(&config, CompileMode::Check { test: false }).unwrap();
  opt.features = features.collect::<_>();
  opt.all_features = args.all_features;
  opt.no_default_features = args.no_default_features;
  
  // BuildConfig, see https://docs.rs/cargo/0.31.0/cargo/core/compiler/struct.BuildConfig.html
  if let Some(jobs) = args.jobs {
    opt.build_config.jobs = jobs;
  }
  
  opt.build_config.build_plan = args.build_plan;
  
  opt
}












fn find_unsafe_in_packages<F>(
  packs: &PackageSet,
  allow_partial_results: bool,
  include_tests: IncludeTests,
  mode: ScanMode,
  mut progress_step: F,
) -> GeigerContext
where
F: FnMut(usize, usize) -> CargoResult<()>,
{
  let mut pack_id_to_metrics = HashMap::new();
  let packs = packs.get_many(packs.package_ids()).unwrap();
  let pack_code_files: Vec<_> = find_rs_files_in_packages(&packs).collect();
  let pack_code_file_count = pack_code_files.len();
  for (i, (pack_id, rs_code_file)) in pack_code_files.into_iter().enumerate()
  {
    let (is_entry_point, p) = match rs_code_file {
      RsFile::LibRoot(pb) => (true, pb),
      RsFile::BinRoot(pb) => (true, pb),
      RsFile::CustomBuildRoot(pb) => (true, pb),
      RsFile::Other(pb) => (false, pb),
    };
    if let (false, ScanMode::EntryPointsOnly) = (is_entry_point, &mode) {
      continue;
    }
    match find_unsafe_in_file(&p, include_tests) {
      Err(e) => {
        if allow_partial_results {
          eprintln!(
            "Failed to parse file: {}, {:?} ",
            &p.display(),
            e
          );
        } else {
          panic!("Failed to parse file: {}, {:?} ", &p.display(), e);
        }
      }
      Ok(file_metrics) => {
        let package_metrics = pack_id_to_metrics
        .entry(pack_id)
        .or_insert_with(PackageMetrics::default);
        let wrapper = package_metrics
        .rs_path_to_metrics
        .entry(p)
        .or_insert_with(RsFileMetricsWrapper::default);
        wrapper.metrics = file_metrics;
        wrapper.is_crate_entry_point = is_entry_point;
      }
    }
    let _ = progress_step(i, pack_code_file_count);
  }
  GeigerContext { pack_id_to_metrics }
}

fn analyze_unsafe(package_risk: &mut PackageRisk) {
  let package_path = package_risk.manifest_path.parent().unwrap();
  //
  let copt = build_compile_options(args, config);
  let rs_files_used_in_compilation = resolve_rs_file_deps(&copt, &ws).unwrap();
  
  let mut out_file =
  std::fs::File::create(&args.out_path).expect("Could not open output file for writing");
  
  let rs_files_scanned = find_unsafe_in_packages(
    &mut out_file,
    &packages,
    rs_files_used_in_compilation,
    true,
    IncludeTests::No,
  );
  
  rs_files_scanned
  .iter()
  .filter(|(_k, v)| **v == 0)
  .for_each(|(k, _v)| {
    // TODO: Ivestigate if this is related to code generated by build
    // scripts and/or macros. Some of the warnings of this kind is
    // printed for files somewhere under the "target" directory.
    // TODO: Find out if we can lookup PackageId associated with each
    // `.rs` file used by the build, including the file paths extracted
    // from `.d` dep files.
    warn!("Dependency file was never scanned: {}", k.display())
  });
}

*/