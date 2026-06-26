use std::{
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use anyhow::{anyhow, Context};
use cargo_metadata::{
    semver, Artifact, ArtifactDebuginfo, Message, MetadataCommand, Package, TargetKind,
};
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

    /// Kind of target (lib or bin) when running with <unit-test> or <unit-bench> which is
    /// may be required when we have two targets with the same name.
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

#[cfg(unix)]
static NO_ROSEGMENT_LINK_ARG: &str = "link-arg=-Wl,--no-rosegment";

fn build(opt: &Opt, kind: Vec<TargetKind>) -> anyhow::Result<Vec<Artifact>> {
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
        match kind
            .iter()
            .any(|k| matches!(k, TargetKind::Lib | TargetKind::RLib))
        {
            true => cmd.arg("--lib"),
            false => cmd.args(["--bin", unit_test]),
        };
    }

    if let Some(Some(ref unit_bench)) = opt.unit_bench {
        match kind
            .iter()
            .any(|k| matches!(k, TargetKind::Lib | TargetKind::RLib))
        {
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

    #[cfg(unix)]
    {
        let (should_add_flag, rustflags_env_var) = should_add_no_rosegment_flag("+nightly")?;
        if should_add_flag {
            cmd.env(
                "RUSTFLAGS",
                format!(
                    "{} -C{NO_ROSEGMENT_LINK_ARG}",
                    rustflags_env_var.as_ref().map_or("", String::as_str)
                ),
            );
        }
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

#[cfg(unix)]
fn should_add_no_rosegment_flag(
    toolchain_specifier: &'static str,
) -> anyhow::Result<(bool, Option<String>)> {
    // `cargo metadata` doesn't provide this, so the `cargo_metadata` crate isn't a help here.
    let cargo_version_stdout = Command::new("cargo")
        .arg("--version")
        .spawn()
        // .spawn and .wait_with_output don't have distinct enough fail conditions for us to
        // provide special error messages for each one
        .and_then(|c| c.wait_with_output())
        .context("`cargo --version` failed to run")?
        .stdout;

    let cargo_version = std::str::from_utf8(&cargo_version_stdout)
        .context("`cargo --version`'s output was not valid utf8")?
        .split(' ')
        .nth(1)
        .ok_or_else(|| anyhow!("`cargo --version` provided an answer in a format unlike the expected 'cargo <version> (<hash> <date>)'"))?;

    let cargo_semver =
        semver::Version::parse(cargo_version).context("cargo's version was not a valid semver")?;

    let at_least_1_90 = cargo_semver >= semver::Version::new(1, 90, 0);
    let mut using_gold = false;
    let mut specified_no_rosegment = false;
    let mut using_linker_that_needs_flag = false;

    let rustflags_env_var = std::env::var("RUSTFLAGS").ok();
    if let Some(ref flags) = rustflags_env_var {
        detect_linker_settings(
            flags.split(' '),
            &mut using_gold,
            &mut specified_no_rosegment,
            &mut using_linker_that_needs_flag,
        );
    }

    let rustc_print_target_output = Command::new("rustc")
        .args([
            toolchain_specifier,
            "-Z",
            "unstable-options",
            "--print",
            "target-spec-json",
        ])
        .spawn()
        .and_then(|c| c.wait_with_output())
        .context("Failed to execute `rustc` to determine current target")?;

    'get_profile: {
        if !rustc_print_target_output.status.success() {
            let rustc_target_json = serde_json::from_slice::<serde_json::Value>(&rustc_print_target_output.stdout)
                .context("`rustc -Z unstable-options --print target-spec-json` provided non-json output despite exiting with an OK exit code")?;

            let Some(rustc_target) = rustc_target_json
                .as_object()
                .and_then(|obj| obj.get("llvm-target"))
                .and_then(|llvm_target| llvm_target.as_str())
            else {
                // It's an unstable feature, so it makes sense it wouldn't stay the same - we should
                // probably warn here or smth, though, so that someone can report when it changes.
                break 'get_profile;
            };

            let cargo_config = Command::new("cargo")
                .args([
                    toolchain_specifier,
                    "-Z",
                    "unstable-options",
                    "config",
                    "get",
                ])
                .spawn()
                .and_then(|c| c.wait_with_output())
                .context("Failed to execute `cargo` to determine current config options")?;

            // theoretically, we should be able to run nightly options with `cargo` if we can with
            // `rustc`, but I guess we should be tolerant of if we can't.
            if !cargo_config.status.success() {
                break 'get_profile;
            }

            // it's nightly, it could change I guess. Really shouldn't be non-utf8 but we can't
            // guarantee anything.
            let Ok(cargo_opts_utf8) = std::str::from_utf8(&cargo_config.stdout) else {
                break 'get_profile;
            };

            // This command outputs a bunch of lines like:
            // ```
            // profile.perf.debug = true
            // profile.perf.inherits = "release"
            // target.aarch64-unknown-linux-gnu.rustflags = ["-C", "linker=clang"]
            // ```
            // So the lines after `target.{triple}.rustflags = ` should be valid json.
            // Theoretically. I guess they can change the format at any point.
            let rustflags = cargo_opts_utf8
                .lines()
                .find_map(|l| {
                    let mut splits = l.split(' ');
                    splits.next().and_then(|config_name| {
                        if config_name.starts_with("target.")
                            && config_name.contains(rustc_target)
                            && config_name.ends_with(".rustflags")
                        {
                            // nth(1) because we've already moved over the first one with the
                            // `.next()`
                            splits.nth(1)
                        } else {
                            None
                        }
                    })
                })
                // If it's not a json array, anymore, we don't want this to start throwing
                // errors since it's not stabilized.
                .and_then(|toml_json| serde_json::from_str::<Vec<&str>>(toml_json).ok());

            // silently ignoring errors here since they're liable to change the format at any time
            if let Some(target_rustflags) = rustflags {
                detect_linker_settings(
                    target_rustflags.into_iter(),
                    &mut using_gold,
                    &mut specified_no_rosegment,
                    &mut using_linker_that_needs_flag,
                );
            }
        }
    }

    let should_add =
        ((at_least_1_90 && !using_gold) || using_linker_that_needs_flag) && !specified_no_rosegment;
    Ok((should_add, rustflags_env_var))
}

#[cfg(unix)]
fn detect_linker_settings<'a>(
    flags: impl Iterator<Item = &'a str>,
    using_gold: &mut bool,
    specified_no_rosegment: &mut bool,
    using_linker_that_needs_flag: &mut bool,
) {
    for flag in flags {
        if flag.starts_with("link-arg=-fuse-ld=") || flag.starts_with("-Clink-arg=-fuse-ld=") {
            if !*using_gold {
                *using_gold = flag.ends_with("/ld") || flag.ends_with("/gold");
            }

            if !*using_linker_that_needs_flag {
                // does wild need this flag? are there other linkers we should include?
                *using_linker_that_needs_flag =
                    flag.ends_with("/wild") || flag.ends_with("/lld") || flag.ends_with("/mold");
            }
        }

        if !*specified_no_rosegment {
            *specified_no_rosegment = flag.ends_with(NO_ROSEGMENT_LINK_ARG);
        }
    }
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
        } => (&[TargetKind::Lib, TargetKind::RLib, TargetKind::Bin], t),
        Opt {
            unit_bench: Some(Some(t)),
            ..
        } => {
            trailing_arguments.push("--bench".to_string());
            (&[TargetKind::Lib, TargetKind::RLib, TargetKind::Bin], t)
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
                    a.target.name == *target && a.target.kind.iter().any(|k| kind.contains(k))
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
                    "failed to canonicalize manifest parent directory '{}'\nHint: make sure your manifest path exists and points to a Cargo.toml file",
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
            Some(pkg) => pkg == *p.name,
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
                if !t.kind.iter().any(|s| kind.contains(s)) {
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
                    package: name.as_ref().to_owned(),
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
            Some(UnitTestTargetKind::Lib) => &[TargetKind::Lib, TargetKind::RLib],
            None => &[TargetKind::Bin, TargetKind::Lib, TargetKind::RLib],
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
        let kinds = match opt.unit_test_kind {
            Some(UnitTestTargetKind::Bin) => &[TargetKind::Bin][..],
            Some(UnitTestTargetKind::Lib) => &[TargetKind::Lib, TargetKind::RLib],
            None => &[TargetKind::Bin, TargetKind::Lib, TargetKind::RLib],
        };

        let target = find_unique_target(
            kinds,
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

    let artifacts = build(&opt, kind)?;
    let workload = workload(&opt, &artifacts)?;
    flamegraph::generate_flamegraph_for_workload(Workload::Command(workload), opt.graph)
}
