use std::{
    fs::File,
    io::{BufReader, BufWriter},
    path::PathBuf,
    process::Command,
};

#[cfg(target_os = "linux")]
use inferno::collapse::perf::{Folder, Options as CollapseOptions};

#[cfg(not(target_os = "linux"))]
use inferno::collapse::dtrace::{Folder, Options as CollapseOptions};

use inferno::{
    collapse::Collapse,
    flamegraph::{from_reader, Options as FlamegraphOptions},
};

use structopt::StructOpt;

#[derive(Debug, StructOpt)]
struct Opt {
    /// Activate release mode
    #[structopt(short = "r", long = "release")]
    release: bool,

    /// Binary to run
    #[structopt(short = "b", long = "bin")]
    bin: Option<String>,

    /// Other command to run
    #[structopt(short = "e", long = "exec")]
    exec: Option<String>,

    /// Output file, flamegraph.svg if not present
    #[structopt(parse(from_os_str), short = "o", long = "output")]
    output: Option<PathBuf>,

    /// Build features to enable
    #[structopt(short = "f", long = "features")]
    features: Option<String>,
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

#[cfg(target_os = "linux")]
mod arch {
    use super::*;

    pub const SPAWN_ERROR: &'static str = "could not spawn perf";
    pub const WAIT_ERROR: &'static str = "unable to wait for perf \
                                          child command to exit";

    pub(crate) fn initial_command(_: Opt) -> Command {
        let mut command = Command::new("perf");

        for arg in "record -F 99 -g".split_whitespace() {
            command.arg(arg);
        }

        command
    }

    pub fn reader() -> BufReader<File> {
        let perf_script_output = Command::new("perf")
            .arg("script")
            .output()
            .expect("unable to call perf script");
        BufReader::new(&*perf_script_output.stdout)
    }
}

#[cfg(not(target_os = "linux"))]
mod arch {
    use super::*;

    pub const SPAWN_ERROR: &'static str = "could not spawn dtrace";
    pub const WAIT_ERROR: &'static str = "unable to wait for dtrace \
                                          child command to exit";

    pub(crate) fn initial_command(opt: Opt) -> Command {
        let mut command = Command::new("dtrace");
        let mut cmd: Vec<String> = "-x ustackframes=100 -n"
            .split_whitespace()
            .map(|s| s.into())
            .collect();
        cmd.push("profile-997 /pid == $target/ { @[ustack()] = count(); }".to_string());
        cmd.push("-o".to_string());
        cmd.push("out.stacks".to_string());
        cmd.push("-c".to_string());

        if let Some(exec) = opt.exec {
            cmd.push(exec);
        } else {
            let mut cargo = String::from("cargo run");

            if opt.release {
                cargo.push_str(" --release")
            }

            if let Some(bin) = opt.bin {
                cargo.push_str(" --bin ");
                cargo.push_str(&bin);
            }

            if let Some(features) = opt.features {
                cargo.push_str(" --features ");
                cargo.push_str(&features);
            }
            cmd.push(cargo);
        };

        let parts = cmd.into_iter();

        for arg in parts {
            command.arg(arg);
        }
        command
    }

    pub fn reader() -> BufReader<File> {
        let dtrace_output =
            File::open("out.stacks").expect("failed to open dtrace output file out.stacks");
        BufReader::new(dtrace_output)
    }
}

fn main() {
    let Opts::Flamegraph(mut opt) = Opts::from_args();

    let flamegraph_filename = opt.output.take().unwrap_or("flamegraph.svg".into());

    let mut command: Command = arch::initial_command(opt);

    let mut recorder = command.spawn().expect(arch::SPAWN_ERROR);
    recorder.wait().expect(arch::WAIT_ERROR);

    let reader = arch::reader();

    let mut collapsed = vec![];

    let collapsed_writer = BufWriter::new(&mut collapsed);

    let collapse_options = CollapseOptions::default();

    Folder::from(collapse_options)
        .collapse(reader, collapsed_writer)
        .expect("unable to collapse generated profile data");

    let collapsed_reader = BufReader::new(&*collapsed);

    let flamegraph_file =
        File::create(flamegraph_filename).expect("unable to create flamegraph.svg output file");

    let flamegraph_writer = BufWriter::new(flamegraph_file);

    let flamegraph_options = FlamegraphOptions::default();

    from_reader(flamegraph_options, collapsed_reader, flamegraph_writer)
        .expect("unable to generate a flamegraph from the collapsed stack data");
}
