use std::path::PathBuf;

use anyhow::{anyhow, Context};
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

fn build(opt: &Opt) -> anyhow::Result<Vec<Artifact>> {
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
        .context("failed to execute cargo build command")?;

    if !status.success() {
        return Err(anyhow!("cargo build failed"));
    }

    Message::parse_stream(&*stdout)
        .filter_map(|m| match m {
            Ok(Message::CompilerArtifact(artifact)) => Some(Ok(artifact)),
            Ok(_) => None,
            Err(e) => Some(Err(e).context("failed to parse cargo build output")),
        })
        .collect()
}

fn workload(opt: &Opt, artifacts: &[Artifact]) -> anyhow::Result<Vec<String>> {
    if artifacts.iter().all(|a| a.executable.is_none()) {
        return Err(anyhow!(
            "build artifacts do not contain any executable to profile"
        ));
    }

    let (kind, target) = match opt {
        Opt { bin: Some(t), .. } => ("bin", t),
        Opt {
            example: Some(t), ..
        } => ("example", t),
        Opt { test: Some(t), .. } => ("test", t),
        Opt { bench: Some(t), .. } => ("bench", t),
        _ => return Err(anyhow!("no target for profiling")),
    };

    // `target.kind` is a `Vec`, but it always seems to contain exactly one element.
    let (debug_level, binary_path) = artifacts
        .iter()
        .find_map(|a| {
            a.executable
                .as_deref()
                .filter(|_| a.target.name == *target && a.target.kind.iter().any(|k| k == kind))
                .map(|e| (a.profile.debuginfo, e))
        })
        .ok_or_else(|| {
            let targets: Vec<_> = artifacts
                .iter()
                .map(|a| (&a.target.kind, &a.target.name))
                .collect();
            anyhow!(
                "could not find desired target {:?} in the targets for this crate: {:?}",
                (kind, target),
                targets
            )
        })?;

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

    let mut command = Vec::with_capacity(1 + opt.trailing_arguments.len());
    command.push(binary_path.to_string());
    command.extend(opt.trailing_arguments.iter().cloned());
    Ok(command)
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

fn find_unique_bin_target() -> anyhow::Result<BinaryTarget> {
    let mut bin_targets: Vec<_> = MetadataCommand::new()
        .no_deps()
        .exec()
        .context("failed to access crate metadata")?
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
            Ok(target)
        }
        [] => Err(anyhow!(
            "crate has no binary targets: try passing `--example <example>` \
                or similar to choose a binary"
        )),
        _ => Err(anyhow!(
            "several possible targets found: {:?}, please pass `--bin <binary>` or \
                `--example <example>` to cargo flamegraph to choose one of them",
            bin_targets
        )),
    }
}

fn main() -> anyhow::Result<()> {
    let Opts::Flamegraph(mut opt) = Opts::from_args();

    if opt.bin.is_none() || opt.bench.is_none() || opt.example.is_none() || opt.test.is_none() {
        let BinaryTarget { target, package } = find_unique_bin_target()?;
        opt.bin = Some(target);
        opt.package = Some(package);
    }

    let artifacts = build(&opt)?;
    let workload = workload(&opt, &artifacts)?;

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
    )?;

    if opt.open {
        opener::open(&flamegraph_filename).context(format!(
            "failed to open '{}'",
            flamegraph_filename.display()
        ))?;
    }

    Ok(())
}
