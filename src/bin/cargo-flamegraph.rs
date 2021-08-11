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

    /// Build features to enable
    #[structopt(short = "f", long = "features")]
    features: Option<String>,

    /// Disable default features
    #[structopt(long = "no-default-features")]
    no_default_features: bool,

    #[structopt(flatten)]
    graph: flamegraph::Options,

    /// Trailing arguments are passed to the binary being profiled.
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

    if opt.unit_test.is_some() {
        cmd.arg("--tests");
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

    if opt.graph.verbose {
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

    let (kind, target): (&[&str], _) = match opt {
        Opt { bin: Some(t), .. } => (&["bin"], t),
        Opt {
            example: Some(t), ..
        } => (&["example"], t),
        Opt { test: Some(t), .. } => (&["test"], t),
        Opt { bench: Some(t), .. } => (&["bench"], t),
        Opt {
            unit_test: Some(Some(t)),
            ..
        } => (&["lib", "bin"], t),
        _ => return Err(anyhow!("no target for profiling")),
    };

    // `target.kind` is a `Vec`, but it always seems to contain exactly one element.
    let (debug_level, binary_path) = artifacts
        .iter()
        .find_map(|a| {
            a.executable
                .as_deref()
                .filter(|_| {
                    a.target.name == *target
                        && a.target.kind.iter().any(|k| kind.contains(&k.as_str()))
                })
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

fn find_unique_target(kind: &[&str]) -> anyhow::Result<BinaryTarget> {
    let mut targets: Vec<_> = MetadataCommand::new()
        .no_deps()
        .exec()
        .context("failed to access crate metadata")?
        .packages
        .into_iter()
        .flat_map(|p| {
            let Package { targets, name, .. } = p;
            targets.into_iter().filter_map(move |t| {
                t.kind
                    .iter()
                    .any(|s| kind.contains(&s.as_str()))
                    .then(|| BinaryTarget {
                        package: name.clone(),
                        target: t.name,
                    })
            })
        })
        .collect();

    match targets.as_slice() {
        [_] => {
            let target = targets.remove(0);
            eprintln!(
                "automatically selected {} as it is the only valid target",
                target
            );
            Ok(target)
        }
        [] => Err(anyhow!(
            "crate has no automatically selectable target: try passing `--example <example>` \
                or similar to choose a binary"
        )),
        _ => Err(anyhow!(
            "several possible targets found: {:?}, please pass an explicit target.",
            targets
        )),
    }
}

fn main() -> anyhow::Result<()> {
    let Opts::Flamegraph(mut opt) = Opts::from_args();

    if opt.bin.is_none()
        && opt.bench.is_none()
        && opt.example.is_none()
        && opt.test.is_none()
        && opt.unit_test.is_none()
    {
        let BinaryTarget { target, package } = find_unique_target(&["bin"])?;
        opt.bin = Some(target);
        opt.package = Some(package);
    }

    if opt.unit_test == Some(None) {
        let BinaryTarget { target, package } = find_unique_target(&["bin", "lib"])?;
        opt.unit_test = Some(Some(target));
        opt.package = Some(package);
    }

    let artifacts = build(&opt)?;
    let workload = workload(&opt, &artifacts)?;
    flamegraph::generate_flamegraph_for_workload(Workload::Command(workload), opt.graph)
}
