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

    pub(crate) fn initial_command(_: &Opt) -> Command {
        let mut command = Command::new("perf");

        for arg in "record -F 99 -g".split_whitespace() {
            command.arg(arg);
        }

        command
    }

    pub fn output() -> Vec<u8> {
        Command::new("perf")
            .arg("script")
            .output()
            .expect("unable to call perf script")
            .stdout
    }
}

#[cfg(not(target_os = "linux"))]
mod arch {
    use super::*;

    pub const SPAWN_ERROR: &'static str = "could not spawn dtrace";
    pub const WAIT_ERROR: &'static str = "unable to wait for dtrace \
                                          child command to exit";

    pub fn initial_command(opt: &Opt) -> Command {
        let basename: String = if let Some(exec) = self.exec {
            let first = exec
                .split_whitespace()
                .nth(0)
                .expect("the exec argument expects a binary to run");

            exec.split('/').last().unwrap()
        } else {
            "cargo".into()
        };

        let dtrace_script = format!(
            "profile-997 /execname == \"{}\"/ { @[ustack(100)] = count(); }",
            basename
        );

        let mut command = Command::new("dtrace");

        command.arg("-n");
        command.arg(dtrace_script);

        for arg in "-o out.stacks -c".split_whitespace() {
            command.arg(arg);
        }

        command
    }
}

impl Into<Command> for Opt {
    fn into(self) -> Command {
        let mut command = arch::initial_command(&self);

        let mut cmd: Vec<String> = vec![];

        if let Some(exec) = self.exec {
            for e in exec.split_whitespace() {
                cmd.push(e.into());
            }
        } else {
            cmd.push("cargo".into());
            cmd.push("run".into());

            if self.release {
                cmd.push("--release".into())
            }

            if let Some(bin) = self.bin {
                cmd.push("--bin".into());
                cmd.push(bin.into());
            }

            if let Some(features) = self.features {
                cmd.push("--features".into());
                cmd.push(features.into());
            }
        };

        for arg in cmd.into_iter() {
            command.arg(arg);
        }

        command
    }
}

fn main() {
    let Opts::Flamegraph(mut opt) = Opts::from_args();

    let flamegraph_filename = opt.output.take().unwrap_or("flamegraph.svg".into());

    let mut command: Command = opt.into();

    let mut recorder = command.spawn().expect(arch::SPAWN_ERROR);
    recorder.wait().expect(arch::WAIT_ERROR);

    let output = arch::output();

    let perf_reader = BufReader::new(&*output);

    let mut collapsed = vec![];

    let collapsed_writer = BufWriter::new(&mut collapsed);

    let collapse_options = CollapseOptions::default();

    Folder::from(collapse_options)
        .collapse(perf_reader, collapsed_writer)
        .expect("unable to collapse generated profile data");

    let collapsed_reader = BufReader::new(&*collapsed);

    let flamegraph_file =
        File::create(flamegraph_filename).expect("unable to create flamegraph.svg output file");

    let flamegraph_writer = BufWriter::new(flamegraph_file);

    let flamegraph_options = FlamegraphOptions::default();

    from_reader(flamegraph_options, collapsed_reader, flamegraph_writer)
        .expect("unable to generate a flamegraph from the collapsed stack data");
}
