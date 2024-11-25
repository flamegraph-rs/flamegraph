use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use cargo_metadata::{Artifact, ArtifactDebuginfo, Message, MetadataCommand, Package, TargetKind};
use clap::{Args, Parser};

use flamegraph::Workload;

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
#[clap(rename_all = "snake_case")]
enum UnitTestTargetKind {
    Bin,
    Lib,
}

#[derive(Args, Debug)]
struct Opt {
    /// Build with the dev profile
    #[clap(long)]
    dev: bool,

    /// Build with the specified profile
    #[clap(long)]
    profile: Option<String>,

    /// package with the binary to run
    #[clap(short, long)]
    package: Option<String>,

    /// Binary to run
    #[clap(short, long, group = "exec-args")]
    bin: Option<String>,

    /// Build for the target triple
    #[clap(long, group = "exec-args")]
    target: Option<String>,

    /// Example to run
    #[clap(long, group = "exec-args")]
    example: Option<String>,

    /// Test binary to run (currently profiles the test harness and all tests in the binary)
    #[clap(long, group = "exec-args")]
    test: Option<String>,

    /// Crate target to unit test, <unit-test> may be omitted if crate only has one target
    /// (currently profiles the test harness and all tests in the binary; test selection
    /// can be passed as trailing arguments after `--` as separator)
    #[clap(long, group = "exec-args")]
    unit_test: Option<Option<String>>,

    /// Kind of target (lib or bin) when running with <unit-test> which is may be
    /// required when we have two targets with the same name.
    #[clap(long)]
    unit_test_kind: Option<UnitTestTargetKind>,

    /// Crate target to unit benchmark, <bench> may be omitted if crate only has one target
    /// (currently profiles the test harness and all tests in the binary; test selection
    /// can be passed as trailing arguments after `--` as separator)
    #[clap(long, group = "exec-args")]
    unit_bench: Option<Option<String>>,

    /// Benchmark to run
    #[clap(long, group = "exec-args")]
    bench: Option<String>,

    /// Path to Cargo.toml
    #[clap(long)]
    manifest_path: Option<PathBuf>,

    /// Build features to enable
    #[clap(short, long)]
    features: Option<String>,

    /// Disable default features
    #[clap(long)]
    no_default_features: bool,

    /// No-op. For compatibility with `cargo run --release`.
    #[clap(short, long)]
    release: bool,

    #[clap(flatten)]
    graph: flamegraph::Options,

    /// Trailing arguments passed to the binary being profiled.
    #[clap(last = true)]
    trailing_arguments: Vec<String>,
}

#[derive(Parser, Debug)]
#[clap(bin_name = "cargo")]
enum Cli {
    /// A cargo subcommand for generating flamegraphs, using inferno
    #[clap(version)]
    Flamegraph(Opt),
}

fn build(opt: &Opt, kind: Vec<TargetKind>) -> anyhow::Result<Vec<Artifact>> {
    use std::process::{Command, Output, Stdio};
    let mut cmd = Command::new("cargo");

    // This will build benchmarks with the `bench` profile. This is needed
    // because the `--profile` argument for `cargo build` is unstable.
    if !opt.dev && (opt.bench.is_some() || opt.unit_bench.is_some()) {
        cmd.args(["bench", "--no-run"]);
    } else if opt.unit_test.is_some() {
        cmd.args(["test", "--no-run"]);
    } else {
        cmd.arg("build");
    }

    if let Some(profile) = &opt.profile {
        cmd.arg("--profile").arg(profile);
    } else if !opt.dev && opt.bench.is_none() && opt.unit_bench.is_none() {
        // do not use `--release` when we are building for `bench`
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

    if let Some(ref target) = opt.target {
        cmd.arg("--target");
        cmd.arg(target);
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

    if let Some(Some(ref unit_test)) = opt.unit_test {
        match kind.iter().any(|k| k == &TargetKind::Lib) {
            true => cmd.arg("--lib"),
            false => cmd.args(["--bin", unit_test]),
        };
    }

    if let Some(Some(ref unit_bench)) = opt.unit_bench {
        match kind.iter().any(|k| k == &TargetKind::Lib) {
            true => cmd.arg("--lib"),
            false => cmd.args(["--bin", unit_bench]),
        };
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
    let mut trailing_arguments = opt.trailing_arguments.clone();

    if artifacts.iter().all(|a| a.executable.is_none()) {
        return Err(anyhow!(
            "build artifacts do not contain any executable to profile"
        ));
    }

    let (kind, target): (&[TargetKind], _) = match opt {
        Opt { bin: Some(t), .. } => (&[TargetKind::Bin], t),
        Opt {
            example: Some(t), ..
        } => (&[TargetKind::Example], t),
        Opt { test: Some(t), .. } => (&[TargetKind::Test], t),
        Opt { bench: Some(t), .. } => (&[TargetKind::Bench], t),
        Opt {
            unit_test: Some(Some(t)),
            ..
        } => (&[TargetKind::Lib, TargetKind::Bin], t),
        Opt {
            unit_bench: Some(Some(t)),
            ..
        } => {
            trailing_arguments.push("--bench".to_string());
            (&[TargetKind::Lib, TargetKind::Bin], t)
        }
        _ => return Err(anyhow!("no target for profiling")),
    };

    // `target.kind` is a `Vec`, but it always seems to contain exactly one element.
    let (debug_level, binary_path) = artifacts
        .iter()
        .find_map(|a| {
            a.executable
                .as_deref()
                .filter(|_| {
                    a.target.name == *target && a.target.kind.iter().any(|k| kind.contains(&k))
                })
                .map(|e| (&a.profile.debuginfo, e))
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

    if !opt.dev && debug_level == &ArtifactDebuginfo::None {
        let profile = match opt
            .example
            .as_ref()
            .or(opt.bin.as_ref())
            .or_else(|| opt.unit_test.as_ref().unwrap_or(&None).as_ref())
        {
            // binaries, examples and unit tests use release profile
            Some(_) => "release",
            // tests use the bench profile in release mode.
            _ => "bench",
        };

        eprintln!("\nWARNING: profiling without debuginfo. Enable symbol information by adding the following lines to Cargo.toml:\n");
        eprintln!("[profile.{}]", profile);
        eprintln!("debug = true\n");
        eprintln!("Or set this environment variable:\n");
        eprintln!("CARGO_PROFILE_{}_DEBUG=true\n", profile.to_uppercase());
    }

    let mut command = Vec::with_capacity(1 + trailing_arguments.len());
    command.push(binary_path.to_string());
    command.extend(trailing_arguments);
    Ok(command)
}

#[derive(Clone, Debug)]
struct BinaryTarget {
    package: String,
    target: String,
    kind: Vec<TargetKind>,
}

impl std::fmt::Display for BinaryTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "target {} in package {}", self.target, self.package)
    }
}

pub fn find_crate_root(manifest_path: Option<&Path>) -> anyhow::Result<PathBuf> {
    match manifest_path {
        Some(path) => {
            let path = path.parent().ok_or_else(|| {
                anyhow!(
                    "the manifest path '{}' must point to a Cargo.toml file",
                    path.display()
                )
            })?;

            path.canonicalize().with_context(|| {
                anyhow!(
                    "failed to canonicalize manifest parent directory '{}'\nHint: make sure your manifest path is exists and points to a Cargo.toml file",
                    path.display()
                )
            })
        }
        None => {
            let cargo_toml = "Cargo.toml";
            let cwd = std::env::current_dir().context("failed to determine working directory")?;

            for current in cwd.ancestors() {
                if current.join(cargo_toml).exists() {
                    return Ok(current.to_path_buf());
                }
            }

            Err(anyhow!(
                "could not find '{}' in '{}' or any parent directory",
                cargo_toml,
                cwd.display()
            ))
        }
    }
}

fn find_unique_target(
    kind: &[TargetKind],
    pkg: Option<&str>,
    manifest_path: Option<&Path>,
    target_name: Option<&str>,
) -> anyhow::Result<BinaryTarget> {
    let mut metadata_command = MetadataCommand::new();
    metadata_command.no_deps();
    if let Some(ref manifest_path) = manifest_path {
        metadata_command.manifest_path(manifest_path);
    }

    let crate_root = find_crate_root(manifest_path)?;

    let mut packages = metadata_command
        .exec()
        .context("failed to access crate metadata")?
        .packages
        .into_iter()
        .filter(|p| match pkg {
            Some(pkg) => pkg == p.name,
            None => p.manifest_path.starts_with(&crate_root),
        })
        .peekable();

    if packages.peek().is_none() {
        return Err(match pkg {
            Some(pkg) => anyhow!("workspace has no package named {}", pkg),
            None => anyhow!(
                "failed to find any package in '{}' or below",
                crate_root.display()
            ),
        });
    }

    let mut num_packages = 0;
    let mut is_default = false;

    let mut targets: Vec<_> = packages
        .flat_map(|p| {
            let Package {
                targets,
                name,
                default_run,
                ..
            } = p;
            num_packages += 1;
            if default_run.is_some() {
                is_default = true;
            }
            targets.into_iter().filter_map(move |t| {
                // Keep only targets that are of the right kind.
                if !t.kind.iter().any(|s| kind.contains(&s)) {
                    return None;
                }

                // When `default_run` is set, keep only the target with that name.
                match &default_run {
                    Some(name) if name != &t.name => return None,
                    _ => {}
                }

                match target_name {
                    Some(name) if name != t.name => return None,
                    _ => {}
                }

                Some(BinaryTarget {
                    package: name.clone(),
                    target: t.name,
                    kind: t.kind,
                })
            })
        })
        .collect();

    match targets.as_slice() {
        [_] => {
            let target = targets.remove(0);
            // If the selected target is the default_run of the only package, do not print a message.
            if num_packages != 1 || !is_default {
                eprintln!(
                    "automatically selected {} as it is the only valid target",
                    target
                );
            }
            Ok(target)
        }
        [] => Err(anyhow!(
            "crate has no automatically selectable target:\nHint: try passing `--example <example>` \
                or similar to choose a binary"
        )),
        _ => Err(anyhow!(
            "several possible targets found: {:#?}, please pass an explicit target.",
            targets
        )),
    }
}

fn main() -> anyhow::Result<()> {
    let Cli::Flamegraph(mut opt) = Cli::parse();
    opt.graph.check()?;

    let kind = if opt.bin.is_none()
        && opt.bench.is_none()
        && opt.example.is_none()
        && opt.test.is_none()
        && opt.unit_test.is_none()
        && opt.unit_bench.is_none()
    {
        let target = find_unique_target(
            &[TargetKind::Bin],
            opt.package.as_deref(),
            opt.manifest_path.as_deref(),
            None,
        )?;
        opt.bin = Some(target.target);
        opt.package = Some(target.package);
        target.kind
    } else if let Some(unit_test) = opt.unit_test {
        let kinds = match opt.unit_test_kind {
            Some(UnitTestTargetKind::Bin) => &[TargetKind::Bin][..], // get slice to help type inference
            Some(UnitTestTargetKind::Lib) => &[TargetKind::Lib],
            None => &[TargetKind::Bin, TargetKind::Lib],
        };

        let target = find_unique_target(
            kinds,
            opt.package.as_deref(),
            opt.manifest_path.as_deref(),
            unit_test.as_deref(),
        )?;
        opt.unit_test = Some(Some(target.target));
        opt.package = Some(target.package);
        target.kind
    } else if let Some(unit_bench) = opt.unit_bench {
        let target = find_unique_target(
            &[TargetKind::Bin, TargetKind::Lib],
            opt.package.as_deref(),
            opt.manifest_path.as_deref(),
            unit_bench.as_deref(),
        )?;
        opt.unit_bench = Some(Some(target.target));
        opt.package = Some(target.package);
        target.kind
    } else {
        Vec::new()
    };

    #[cfg(target_os = "macos")]
    if let None = opt.graph.root {
        return Err(anyhow!(
            "DTrace requires elevated permissions on MacOS; re-invoke using 'cargo flamegraph --root ...'",
        ));
    }

    let artifacts = build(&opt, kind)?;
    let workload = workload(&opt, &artifacts)?;
    flamegraph::generate_flamegraph_for_workload(Workload::Command(workload), opt.graph)
}
