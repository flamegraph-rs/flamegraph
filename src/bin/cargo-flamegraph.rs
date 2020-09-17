use std::fs;
use std::path::{Path, PathBuf};

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

    /// Open the output .svg file with default program
    #[structopt(long = "open")]
    open: bool,

    /// Run with root privileges (using `sudo`)
    #[structopt(long = "root")]
    root: bool,

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

fn build(opt: &Opt) {
    let mut cmd = std::process::Command::new("cargo");
    cmd.arg("build");

    if !opt.dev {
        cmd.arg("--release");
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

    if let Some(ref features) = opt.features {
        cmd.arg("--features");
        cmd.arg(features);
    }

    let mut child = cmd
        .spawn()
        .expect("failed to spawn cargo build command");

    let exit_status = child.wait().expect(
        "failed to wait for cargo build child to finish",
    );

    if !opt.dev {
        cmd.arg("--message-format=json");

        let output: Vec<u8> = cmd
            .output()
            .expect("failed to execute cargo build command")
            .stdout;

        let messages =
            cargo_metadata::Message::parse_stream(&*output);

        let mut has_debuginfo = false;

        // This is an extremely coarse check to see
        // if any of our build artifacts have debuginfo
        // enabled.
        for message in messages {
            let artifact = if let Ok(
                cargo_metadata::Message::CompilerArtifact(
                    artifact,
                ),
            ) = message
            {
                artifact
            } else {
                continue;
            };

            // Since workload() returns a Vec of paths, artifact.target.name could be contained in
            // the path (e.g. in the project name). Thus .ends_with() is required to ensure the
            // actual binary name is matched.
            if workload(opt)
                .iter()
                .any(|w| w.ends_with(&artifact.target.name))
                && artifact.profile.debuginfo.unwrap_or(0)
                    != 0
            {
                has_debuginfo = true;
            }
        }

        if !has_debuginfo {
            let profile = if opt.bench.is_some() {
                "bench"
            } else {
                "release"
            };
            eprintln!(
                "\nWARNING: building without debuginfo. \
                 Enable symbol information by adding \
                 the following lines to Cargo.toml:\n"
            );
            eprintln!("[profile.{}]", profile);
            eprintln!("debug = true\n");
        }
    }

    if !exit_status.success() {
        eprintln!("cargo build failed: {:?}", child.stderr);
        std::process::exit(1);
    }
}

fn find_binary(ty: &str, path: &Path, bin: &str) -> String {
    // Ignorance-based error handling. We really do not care about any errors
    // popping up from the filesystem search here. Thus, we just bash them into
    // place using `Option`s monadic properties. Not pretty though.
    fs::read_dir(path)
        .ok()
        .and_then(|mut r| {
            r.find(|f| {
                if let Ok(f) = f {
                    let file_name = f.file_name();
                    let name = file_name.to_string_lossy();
                    name.starts_with(bin)
                        && !name.ends_with(".d")
                } else {
                    false
                }
            })
            .and_then(|r| r.ok())
        })
        .and_then(|f| {
            f.path()
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| {
            eprintln!(
                "could not find desired target {} \
                 in the {} targets for this crate",
                bin, ty
            );
            std::process::exit(1);
        })
}

fn workload(opt: &Opt) -> Vec<String> {
    let mut metadata_cmd =
        cargo_metadata::MetadataCommand::new();
    metadata_cmd.no_deps();
    let metadata = metadata_cmd
        .exec()
        .expect("could not access crate metadata");

    let mut binary_path = metadata.target_directory;

    if opt.dev {
        binary_path.push("debug");
    } else if opt.example.is_some() {
        binary_path.push("examples");
    } else {
        binary_path.push("release");
    }
    binary_path.push("deps");

    let targets: Vec<String> = metadata
        .packages
        .into_iter()
        .flat_map(|p| p.targets)
        .filter(|t| t.crate_types.contains(&"bin".into()))
        .map(|t| t.name)
        .collect();

    if targets.is_empty() {
        eprintln!("no Rust binary targets found");
        std::process::exit(1);
    }

    let target = if let Some(ref test) = opt.test {
        find_binary("test", &binary_path, test)
    } else if let Some(ref bench) = opt.bench {
        find_binary("bench", &binary_path, bench)
    } else if let Some(ref bin) =
        opt.bin.as_ref().or(opt.example.as_ref())
    {
        if targets.contains(&bin) {
            bin.to_string()
        } else {
            eprintln!(
                "could not find desired target {} \
                 in the targets for this crate: {:?}",
                bin, targets
            );
            std::process::exit(1);
        }
    } else if targets.len() == 1 {
        targets[0].to_owned()
    } else {
        eprintln!(
            "several possible targets found: {:?}, \
             please pass `--bin <binary>` or `--example <example>` \
             to cargo flamegraph to choose one of them",
            targets
        );
        std::process::exit(1);
    };

    binary_path.push(target);

    let mut result = opt.trailing_arguments.clone();
    result.insert(0, binary_path.to_string_lossy().into());
    result
}

fn main() {
    let Opts::Flamegraph(mut opt) = Opts::from_args();

    build(&opt);

    let workload = workload(&opt);

    let flamegraph_filename: PathBuf = opt
        .output
        .take()
        .unwrap_or("flamegraph.svg".into());

    flamegraph::generate_flamegraph_for_workload(
        Workload::Command(workload),
        &flamegraph_filename,
        opt.root,
        opt.frequency,
        opt.custom_cmd,
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
