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

    /// Disable inlining for perf script because of performace issues
    #[structopt(long = "no-inline")]
    script_no_inline: bool,

    #[structopt(flatten)]
    flamegraph_options: flamegraph::FlamegraphOptions,

    trailing_arguments: Vec<String>,
}

impl Opt {
    fn has_explicit_target(&self) -> bool {
        self.bin.is_some()
            || self.bench.is_some()
            || self.example.is_some()
            || self.test.is_some()
            || self.unit_test.is_some()
    }

    fn kind(&self) -> TargetKind {
        match self {
            Opt { bin: Some(_), .. } => TargetKind::Bin,
            Opt {
                example: Some(_), ..
            } => TargetKind::Example,
            Opt { test: Some(_), .. } => TargetKind::Test,
            Opt { bench: Some(_), .. } => TargetKind::Bench,
            Opt {
                unit_test: Some(Some(_)),
                ..
            } => TargetKind::UnitTest,
            _ => panic!("No target for profiling."),
        }
    }

    fn target_name(&self) -> &str {
        match self {
            Opt { bin: Some(t), .. } => t,
            Opt {
                example: Some(t), ..
            } => t,
            Opt { test: Some(t), .. } => t,
            Opt { bench: Some(t), .. } => t,
            Opt {
                unit_test: Some(Some(t)),
                ..
            } => t,
            _ => panic!("No target for profiling."),
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

// TODO make OptTarget struct that contains target name, kind, crate kind for unit test,
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TargetKind {
    Bin,
    Example,
    UnitTest,
    Test,
    Bench,
}

fn build(
    target: &VerifiedTarget,
    opt: &Opt,
) -> Vec<Artifact> {
    use std::process::{Command, Output, Stdio};
    let mut cmd = Command::new("cargo");

    let kind = opt.kind();
    let name = &target.target;
    match kind {
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
            if target.kind.contains(&"lib".to_owned()) {
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
    if !opt.dev && kind != TargetKind::Bench {
        cmd.arg("--release");
    }

    cmd.args(&["--package", &target.package]);

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
    opt: &Opt,
    target: &VerifiedTarget,
    artifacts: &[Artifact],
) -> Vec<String> {
    let binary_path = select_executable(target, artifacts);

    let mut result = opt.trailing_arguments.clone();
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

fn find_unique_unit_test_target(
    targets: &[VerifiedTarget],
) -> &VerifiedTarget {
    let unit_test_targets: Vec<_> = targets
        .iter()
        .filter(|t| t.is_kind(TargetKind::UnitTest))
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

fn find_unique_bin_target(
    targets: &[VerifiedTarget],
) -> &VerifiedTarget {
    let bin_targets: Vec<_> = targets
        .iter()
        .filter(|t| t.is_kind(TargetKind::Bin))
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

fn verify_explicit_target<'a>(
    targets: &'a [VerifiedTarget],
    opt: &Opt,
) -> &'a VerifiedTarget {
    let name = opt.target_name();
    let kind = opt.kind();
    let package = opt.package.as_ref();
    let maybe_target = targets.iter().find(|t| {
        let matching_package = package
            .map(|p| t.package == *p)
            .unwrap_or(true); // ignore package name if not given explicitly
        matching_package
            && t.target == name
            && t.is_kind(kind)
    });

    match maybe_target {
        Some(target) => target,
        None => {
            eprintln!("workspace does not contain target {} of kind {:?}{}",
            name, kind, package.map(|p| format!(" in package {}", p)).unwrap_or_default(),
        );
            std::process::exit(1);
        }
    }
}
fn main() {
    let Opts::Flamegraph(mut opt) = Opts::from_args();

    let targets = find_targets();

    let target;
    match (opt.has_explicit_target(), &opt.unit_test) {
        (false, _) => {
            target = find_unique_bin_target(&targets);
            opt.bin = Some(target.target.clone());
        }
        (true, Some(None)) => {
            target = find_unique_unit_test_target(&targets);
            opt.unit_test =
                Some(Some(target.target.clone()));
        }
        _ => {
            target = verify_explicit_target(&targets, &opt)
        }
    };

    let artifacts = build(target, &opt);
    let workload = workload(&opt, target, &artifacts);

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
