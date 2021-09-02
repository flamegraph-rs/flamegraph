use std::path::PathBuf;

use cargo_metadata::{Artifact, Message, MetadataCommand, Package};
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
        conflicts_with = "example",
        conflicts_with = "test"
    )]
    bin: Option<String>,

    /// Example to run
    #[structopt(
        long = "example",
        conflicts_with = "bench",
        conflicts_with = "bin",
        conflicts_with = "test"
    )]
    example: Option<String>,

    /// Test binary to run (currently profiles the test harness and all tests in the binary)
    #[structopt(
        long = "test",
        conflicts_with = "bench",
        conflicts_with = "bin",
        conflicts_with = "example"
    )]
    test: Option<String>,

    /// Benchmark to run
    #[structopt(
        long = "bench",
        conflicts_with = "bin",
        conflicts_with = "example",
        conflicts_with = "test"
    )]
    bench: Option<String>,

    /// Path to Cargo.toml
    #[structopt(long = "manifest-path")]
    manifest_path: Option<PathBuf>,

    /// Output file, flamegraph.svg if not present
    #[structopt(parse(from_os_str), short = "o", long = "output")]
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
    #[structopt(short = "c", long = "cmd", conflicts_with = "freq")]
    custom_cmd: Option<String>,

    /// Disable inlining for perf script because of performace issues
    #[structopt(long = "no-inline")]
    script_no_inline: bool,

    #[structopt(flatten)]
    flamegraph_options: flamegraph::FlamegraphOptions,

    trailing_arguments: Vec<String>,
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

fn build(opt: &Opt) -> Vec<Artifact> {
    use std::process::{Command, Output, Stdio};
    let mut cmd = Command::new("cargo");

    // This will build benchmarks with the `bench` profile. This is needed
    // because the `--profile` argument for `cargo build` is unstable.
    if !opt.dev && opt.bench.is_some() {
        cmd.args(&["bench", "--no-run"]);
    } else {
        cmd.arg("build");
    }

    // do not use `--release` when we are building for `bench`
    if !opt.dev && opt.bench.is_none() {
        cmd.arg("--release");
    }

    if let Some(ref package) = opt.package {
        cmd.arg("--package");
        cmd.arg(package);
    }

    if let Some(ref bin) = opt.bin {
        cmd.arg("--bin");
        cmd.arg(bin);
    }

    if let Some(ref example) = opt.example {
        cmd.arg("--example");
        cmd.arg(example);
    }

    if let Some(ref test) = opt.test {
        cmd.arg("--test");
        cmd.arg(test);
    }

    if let Some(ref bench) = opt.bench {
        cmd.arg("--bench");
        cmd.arg(bench);
    }

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

    if !status.success() {
        eprintln!("cargo build failed!");
        std::process::exit(1);
    }

    Message::parse_stream(&*stdout)
        .filter_map(|m| match m {
            Ok(Message::CompilerArtifact(artifact)) => Some(artifact),
            Ok(_) => None,
            Err(e) => {
                panic!("failed to parse cargo build output: {:?}", e);
            }
        })
        .collect()
}

fn workload(opt: &Opt, artifacts: &[Artifact]) -> Vec<String> {
    if artifacts.iter().all(|a| a.executable.is_none()) {
        eprintln!("build artifacts do not contain any executable to profile");
        std::process::exit(1);
    }

    let (kind, target) = match opt {
        Opt { bin: Some(t), .. } => ("bin", t),
        Opt {
            example: Some(t), ..
        } => ("example", t),
        Opt { test: Some(t), .. } => ("test", t),
        Opt { bench: Some(t), .. } => ("bench", t),
        _ => panic!("No target for profiling."),
    };

    // target.kind is an array for some reason. No idea why though, it always seems to contain exactly one element.
    // If you know why, feel free to PR and handle kind properly.
    let (debug_level, binary_path) = artifacts
        .iter()
        .find_map(|a| {
            a.executable
                .as_deref()
                .filter(|_| a.target.name == *target && a.target.kind.iter().any(|k| k == kind))
                .map(|e| (a.profile.debuginfo, e))
        })
        .unwrap_or_else(|| {
            let targets: Vec<_> = artifacts
                .iter()
                .map(|a| (&a.target.kind, &a.target.name))
                .collect();
            eprintln!(
                "could not find desired target {:?} in the targets for this crate: {:?}",
                (kind, target),
                targets
            );
            std::process::exit(1);
        });

    const NONE: u32 = 0;
    if !opt.dev && debug_level.unwrap_or(NONE) == NONE {
        let profile = match opt.bench {
            Some(_) => "bench",
            None => "release",
        };

        eprintln!("\nWARNING: profiling without debuginfo. Enable symbol information by adding the following lines to Cargo.toml:\n");
        eprintln!("[profile.{}]", profile);
        eprintln!("debug = true\n");
        eprintln!("Or set this environment variable:\n");
        eprintln!("CARGO_PROFILE_{}_DEBUG=true\n", profile.to_uppercase());
    }

    let mut result = opt.trailing_arguments.clone();
    result.insert(0, binary_path.to_string());
    result
}

#[derive(Clone, Debug)]
struct BinaryTarget {
    package: String,
    target: String,
}

impl std::fmt::Display for BinaryTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "target {} in package {}", self.target, self.package)
    }
}

fn find_unique_bin_target() -> BinaryTarget {
    let mut bin_targets: Vec<_> = MetadataCommand::new()
        .no_deps()
        .exec()
        .expect("failed to access crate metadata")
        .packages
        .into_iter()
        .flat_map(|p| {
            let Package { targets, name, .. } = p;
            targets.into_iter().filter_map(move |t| {
                t.kind.iter().any(|s| s == "bin").then(|| BinaryTarget {
                    package: name.clone(),
                    target: t.name,
                })
            })
        })
        .collect();

    match bin_targets.as_slice() {
        [_] => {
            let target = bin_targets.remove(0);
            eprintln!(
                "automatically selected {} as it is the only binary target",
                target
            );
            target
        }
        [] => {
            eprintln!("crate has no binary targets: try passing `--example <example>` or similar to choose a binary");
            std::process::exit(1);
        }
        _ => {
            eprintln!(
                "several possible targets found: {:?}, please pass `--bin <binary>` or \
                `--example <example>` to cargo flamegraph to choose one of them",
                bin_targets
            );
            std::process::exit(1);
        }
    }
}

fn main() {
    let Opts::Flamegraph(mut opt) = Opts::from_args();

    if opt.bin.is_none() || opt.bench.is_none() || opt.example.is_none() || opt.test.is_none() {
        let BinaryTarget { target, package } = find_unique_bin_target();
        opt.bin = Some(target);
        opt.package = Some(package);
    }

    let artifacts = build(&opt);
    let workload = workload(&opt, &artifacts);

    if opt.verbose {
        println!("workload: {:?}", workload);
    }

    let flamegraph_filename: PathBuf = opt.output.take().unwrap_or_else(|| "flamegraph.svg".into());

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
