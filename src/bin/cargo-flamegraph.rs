use std::path::PathBuf;

use cargo_metadata::{
    Artifact, Message, MetadataCommand, Package,
};

use structopt::StructOpt;

use flamegraph::Workload;

#[derive(Debug, StructOpt)]
#[structopt(
    setting = structopt::clap::AppSettings::TrailingVarArg
)]
struct Opt {
    /// Build with the dev profile
    #[structopt(long = "dev")]
    dev: bool,

    /// package with the binary to run
    #[structopt(short = "p", long = "package")]
    package: Option<String>,

    /// Binary to run
    #[structopt(
        short = "b",
        long = "bin",
        conflicts_with = "bench",
        conflicts_with = "unit-test",
        conflicts_with = "example",
        conflicts_with = "test"
    )]
    bin: Option<String>,

    /// Example to run
    #[structopt(
        long = "example",
        conflicts_with = "bench",
        conflicts_with = "unit-test",
        conflicts_with = "bin",
        conflicts_with = "test"
    )]
    example: Option<String>,

    /// Test binary to run (currently profiles the test harness and all tests in the binary)
    #[structopt(
        long = "test",
        conflicts_with = "bench",
        conflicts_with = "unit-test",
        conflicts_with = "bin",
        conflicts_with = "example"
    )]
    test: Option<String>,

    /// Crate target to unit test, <unit-test> may be omitted if crate only has one target
    /// (currently profiles the test harness and all tests in the binary; test selection
    /// can be passed as trailing arguments after `--` as separator)
    #[structopt(
        long = "unit-test",
        conflicts_with = "bench",
        conflicts_with = "bin",
        conflicts_with = "test",
        conflicts_with = "example"
    )]
    unit_test: Option<Option<String>>,

    /// Benchmark to run
    #[structopt(
        long = "bench",
        conflicts_with = "bin",
        conflicts_with = "unit-test",
        conflicts_with = "example",
        conflicts_with = "test"
    )]
    bench: Option<String>,

    /// Path to Cargo.toml
    #[structopt(long = "manifest-path")]
    manifest_path: Option<PathBuf>,

    /// Output file, flamegraph.svg if not present
    #[structopt(
        parse(from_os_str),
        short = "o",
        long = "output"
    )]
    output: Option<PathBuf>,

    /// Build features to enable
    #[structopt(short = "f", long = "features")]
    features: Option<String>,

    /// Disable default features
    #[structopt(long = "no-default-features")]
    no_default_features: bool,

    /// Open the output .svg file with default program
    #[structopt(long = "open")]
    open: bool,

    /// Run with root privileges (using `sudo`)
    #[structopt(long = "root")]
    root: bool,

    /// Print extra output to help debug problems
    #[structopt(short = "v", long = "verbose")]
    verbose: bool,

    /// Sampling frequency
    #[structopt(short = "F", long = "freq")]
    frequency: Option<u32>,

    /// Custom command for invoking perf/dtrace
    #[structopt(
        short = "c",
        long = "cmd",
        conflicts_with = "freq"
    )]
    custom_cmd: Option<String>,

    /// Disable inlining for perf script because of performance issues
    #[structopt(long = "no-inline")]
    script_no_inline: bool,

    #[structopt(flatten)]
    flamegraph_options: flamegraph::FlamegraphOptions,

    trailing_arguments: Vec<String>,
}

impl Opt {
    fn target(&self) -> Option<NamedTargetKind> {
        use NamedTargetKind::*;
        match self {
            Opt { bin: Some(t), .. } => Some(Bin(t)),
            Opt {
                example: Some(t), ..
            } => Some(Example(t)),
            Opt { test: Some(t), .. } => Some(Test(t)),
            Opt { bench: Some(t), .. } => Some(Bench(t)),
            Opt {
                unit_test: Some(t), ..
            } => Some(UnitTest(t.as_deref())),
            _ => None,
        }
    }
}

struct BuildOpts<'t> {
    dev: bool,
    target: &'t VerifiedTarget,
    kind: TargetKind,
    manifest_path: Option<PathBuf>,
    features: Option<String>,
    no_default_features: bool,
    verbose: bool,
}

impl<'t> BuildOpts<'t> {
    fn new(
        opt: &mut Opt,
        target: &'t VerifiedTarget,
    ) -> Self {
        Self {
            dev: opt.dev,
            target,
            kind: opt
                .target()
                .map(Into::into)
                .unwrap_or(TargetKind::Bin),
            manifest_path: opt.manifest_path.take(),
            features: opt.features.take(),
            no_default_features: opt.no_default_features,
            verbose: opt.verbose,
        }
    }
}

#[derive(Copy, Clone)]
enum NamedTargetKind<'a> {
    Bin(&'a str),
    Example(&'a str),
    UnitTest(Option<&'a str>),
    Test(&'a str),
    Bench(&'a str),
}

impl NamedTargetKind<'_> {
    fn target_name(&self) -> Option<&str> {
        match *self {
            NamedTargetKind::Bin(t) => t.into(),
            NamedTargetKind::Example(t) => t.into(),
            NamedTargetKind::UnitTest(Some(t)) => t.into(),
            NamedTargetKind::UnitTest(_) => None,
            NamedTargetKind::Test(t) => t.into(),
            NamedTargetKind::Bench(t) => t.into(),
        }
    }
}

#[derive(Debug, StructOpt)]
#[structopt(
    name = "cargo-flamegraph",
    about = "A cargo subcommand for generating flamegraphs, using inferno"
)]
enum Opts {
    #[structopt(name = "flamegraph")]
    Flamegraph(Opt),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TargetKind {
    Bin,
    Example,
    UnitTest,
    Test,
    Bench,
}

impl From<NamedTargetKind<'_>> for TargetKind {
    fn from(f: NamedTargetKind) -> Self {
        use NamedTargetKind::*;
        match f {
            Bin(_) => Self::Bin,
            Example(_) => Self::Example,
            UnitTest(_) => Self::UnitTest,
            Test(_) => Self::Test,
            Bench(_) => Self::Bench,
        }
    }
}

fn build(opt: BuildOpts) -> Vec<Artifact> {
    use std::process::{Command, Output, Stdio};
    let mut cmd = Command::new("cargo");

    let name = &opt.target.target;
    match opt.kind {
        TargetKind::Bin => {
            cmd.args(&["build", "--bin", name]);
        }
        TargetKind::Example => {
            cmd.args(&["build", "--example", name]);
        }
        TargetKind::Test => {
            cmd.args(&["build", "--test", name]);
        }
        TargetKind::UnitTest => {
            // `cargo test` is required because `cargo build` does not
            // have flags to build individual unit test targets.
            // `cargo test` requires differentiating between lib and bin.
            if opt.target.kind.contains(&"lib".to_owned()) {
                cmd.args(&["test", "--no-run", "--lib"]);
            } else {
                cmd.args(&[
                    "test", "--no-run", "--bin", name,
                ]);
            }
        }
        TargetKind::Bench => {
            if opt.dev {
                cmd.args(&["build", "--bench", name]);
            } else {
                // This will build benchmarks with the `bench` profile. This is needed
                // because the `--profile` argument for `cargo build` is unstable.
                cmd.args(&["bench", "--no-run", name]);
            }
        }
    }

    // do not use `--release` when we are building for `bench`
    if !opt.dev && opt.kind != TargetKind::Bench {
        cmd.arg("--release");
    }

    cmd.args(&["--package", &opt.target.package]);

    if let Some(ref manifest_path) = opt.manifest_path {
        cmd.arg("--manifest-path");
        cmd.arg(manifest_path);
    }

    if let Some(ref features) = opt.features {
        cmd.arg("--features");
        cmd.arg(features);
    }

    if opt.no_default_features {
        cmd.arg("--no-default-features");
    }

    cmd.arg("--message-format=json-render-diagnostics");

    if opt.verbose {
        println!("build command: {:?}", cmd);
    }

    let Output { status, stdout, .. } = cmd
        .stderr(Stdio::inherit())
        .output()
        .expect("failed to execute cargo build command");

    let messages = Message::parse_stream(&*stdout);
    let artifacts: Vec<_> = messages
        .filter_map(|m| match m {
            Ok(Message::CompilerArtifact(artifact)) => {
                Some(artifact)
            }
            Ok(_) => None,
            Err(e) => {
                panic!("failed to parse cargo build output: {:?}", e);
            }
        })
        .collect();

    if !status.success() {
        eprintln!("cargo build failed!");
        std::process::exit(1);
    }

    artifacts
}

fn check_debug_info(
    target: &VerifiedTarget,
    artifact: &Artifact,
) {
    const NONE: u32 = 0;
    let debug = artifact.profile.debuginfo.unwrap_or(NONE);

    if debug != NONE {
        return;
    }

    let profile = if target.is_kind(TargetKind::Bench) {
        "bench"
    } else {
        "release"
    };

    eprintln!(
        "\nWARNING: profiling without debuginfo. \
                 Enable symbol information by adding \
                 the following lines to Cargo.toml:\n"
    );
    eprintln!("[profile.{}]", profile);
    eprintln!("debug = true\n");
    eprintln!("Or set this environment variable:\n");
    eprintln!(
        "CARGO_PROFILE_{}_DEBUG=true\n",
        profile.to_uppercase()
    );
}

fn select_executable(
    target: &VerifiedTarget,
    artifacts: &[Artifact],
) -> PathBuf {
    let kinds = &target.kind;
    let name = &target.target;

    if artifacts.iter().all(|a| a.executable.is_none()) {
        eprintln!( "build artifacts do not contain any executable to profile");
        std::process::exit(1);
    }

    // target.kind is an array for some reason. No idea why though, it always seems to contain exactly one element.
    // If you know why, feel free to PR and handle kind properly.
    let artifact = artifacts.iter().find(|a| {
        a.target.name == *name
            && a.target.kind == *kinds
            && a.executable.is_some()
    });

    let artifact = artifact.unwrap_or_else(|| {
        let targets: Vec<_> = artifacts
            .iter()
            .map(|a| (&a.target.kind, &a.target.name))
            .collect();
        eprintln!(
            "could not find desired target {:?} \
                 in the artifacts of this build: {:?}",
            (kinds, name),
            targets
        );
        std::process::exit(1);
    });

    check_debug_info(target, artifact);

    artifact
        .executable
        .clone()
        // filtered above in find predicate
        .expect("target artifact does have an executable")
}

fn workload(
    trailing_arguments: Vec<String>,
    target: &VerifiedTarget,
    artifacts: &[Artifact],
) -> Vec<String> {
    let binary_path = select_executable(target, artifacts);

    let mut result = trailing_arguments;
    result.insert(0, binary_path.to_string_lossy().into());
    result
}

#[derive(Clone, Debug)]
struct VerifiedTarget {
    package: String,
    target: String,
    kind: Vec<String>,
}

impl VerifiedTarget {
    fn is_kind(&self, kind: TargetKind) -> bool {
        match kind {
            TargetKind::Bin => {
                self.kind.contains(&"bin".to_owned())
            }
            TargetKind::Example => {
                self.kind.contains(&"example".to_owned())
            }
            TargetKind::Test => {
                self.kind.contains(&"test".to_owned())
            }
            TargetKind::Bench => {
                self.kind.contains(&"bench".to_owned())
            }
            TargetKind::UnitTest => {
                self.kind.contains(&"bin".to_owned())
                    || self.kind.contains(&"lib".to_owned())
            }
        }
    }

    /// Returns true if the package name matches or `package` is None.
    /// This allows ignoring the package if it's not given explicitly.
    fn is_in_package(&self, package: Option<&str>) -> bool {
        package.map_or(true, |p| p == self.package)
    }
}

impl std::fmt::Display for VerifiedTarget {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter,
    ) -> std::fmt::Result {
        write!(
            f,
            "target {} in package {}",
            self.target, self.package
        )
    }
}

fn find_targets() -> Vec<VerifiedTarget> {
    let mut metadata_command = MetadataCommand::new();
    metadata_command.no_deps();
    let metadata = metadata_command
        .exec()
        .expect("failed to access crate metadata");

    let targets: Vec<VerifiedTarget> = metadata
        .packages
        .into_iter()
        .flat_map(|p| {
            let Package { targets, name, .. } = p;
            targets
                .into_iter()
                .map(move |t| (name.clone(), t))
        })
        .map(|(p, t)| VerifiedTarget {
            package: p,
            target: t.name,
            kind: t.kind,
        })
        .collect();

    targets
}

fn find_unique_unit_test_target<'t>(
    targets: &'t [VerifiedTarget],
    package: Option<&str>,
) -> &'t VerifiedTarget {
    let unit_test_targets: Vec<_> = targets
        .iter()
        .filter(|t| {
            t.is_kind(TargetKind::UnitTest)
                && t.is_in_package(package)
        })
        .collect();

    match unit_test_targets.as_slice() {
        [target] => {
            eprintln!(
                "automatically selected {} as it is the only unit test target",
                target
            );
            target
        }
        [] => {
            eprintln!(
                "crate has no unit test targets: try passing \
                    `--example <example>` or similar to choose a binary"
            );
            std::process::exit(1);
        }
        _ => {
            eprintln!(
                "several possible targets found: {:?}, \
                     please pass `--unit-test <target>` to cargo flamegraph \
                     to choose one of them",
                unit_test_targets
            );
            std::process::exit(1);
        }
    }
}

fn find_unique_bin_target<'t>(
    targets: &'t [VerifiedTarget],
    package: Option<&str>,
) -> &'t VerifiedTarget {
    let bin_targets: Vec<_> = targets
        .iter()
        .filter(|t| {
            t.is_kind(TargetKind::Bin)
                && t.is_in_package(package)
        })
        .collect();

    match bin_targets.as_slice() {
        [target] => {
            eprintln!(
                "automatically selected {} as it is the only binary target",
                target
            );
            target
        }
        [] => {
            eprintln!(
                "crate has no binary targets: try passing \
                    `--example <example>` or similar to choose a binary"
            );
            std::process::exit(1);
        }
        _ => {
            eprintln!(
                "several possible targets found: {:?}, \
                     please pass `--bin <binary>` or `--example <example>` \
                     to cargo flamegraph to choose one of them",
                bin_targets
            );
            std::process::exit(1);
        }
    }
}

fn verify_explicit_target<'t>(
    targets: &'t [VerifiedTarget],
    kind: NamedTargetKind,
    package: Option<&str>,
) -> &'t VerifiedTarget {
    let target_name = kind
        .target_name()
        .expect("No explicit target to verify.");
    let kind = kind.into();
    let maybe_target = targets.iter().find(|t| {
        t.is_in_package(package)
            && t.target == target_name
            && t.is_kind(kind)
    });

    match maybe_target {
        Some(target) => target,
        None => {
            eprintln!("workspace does not contain target {} of kind {:?}{}",
            target_name, kind, package.map(|p| format!(" in package {}", p)).unwrap_or_default(),
        );
            std::process::exit(1);
        }
    }
}
fn main() {
    let Opts::Flamegraph(mut opt) = Opts::from_args();

    let package = opt.package.as_deref();
    let targets = find_targets();
    let target = match opt.target() {
        None => find_unique_bin_target(&targets, package),
        Some(NamedTargetKind::UnitTest(None)) => {
            find_unique_unit_test_target(&targets, package)
        }
        Some(kind) => {
            verify_explicit_target(&targets, kind, package)
        }
    };

    let build_opts = BuildOpts::new(&mut opt, target);
    let artifacts = build(build_opts);
    let workload = workload(
        std::mem::take(&mut opt.trailing_arguments),
        target,
        &artifacts,
    );

    if opt.verbose {
        println!("workload: {:?}", workload);
    }

    let flamegraph_filename: PathBuf = opt
        .output
        .take()
        .unwrap_or_else(|| "flamegraph.svg".into());

    flamegraph::generate_flamegraph_for_workload(
        Workload::Command(workload),
        &flamegraph_filename,
        opt.root,
        opt.script_no_inline,
        opt.frequency,
        opt.custom_cmd,
        opt.flamegraph_options.into_inferno(),
        opt.verbose,
    );

    if opt.open {
        if let Err(e) = opener::open(&flamegraph_filename) {
            eprintln!(
                "Failed to open [{}]. Error: {}",
                flamegraph_filename.display(),
                e
            );
        }
    }
}
