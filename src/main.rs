use std::{
    fs::File,
    io::{BufReader, BufWriter},
    path::PathBuf,
    process::Command,
};

use inferno::{
    collapse::{
        perf::{Folder, Options as CollapseOptions},
        Collapse,
    },
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

impl Into<Command> for Opt {
    fn into(self) -> Command {
        let mut command = Command::new("perf");

        let mut cmd: Vec<String> = "record -F 99 -g"
            .split_whitespace()
            .map(|s| s.into())
            .collect();

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
        };

        let parts = cmd.into_iter();

        for arg in parts {
            command.arg(arg);
        }

        command
    }
}

fn main() {
    let Opts::Flamegraph(mut opt) = Opts::from_args();

    let output = opt.output.take().unwrap_or("flamegraph.svg".into());

    let mut command: Command = opt.into();

    let mut recorder = command.spawn().expect("unable to spawn perf command");
    recorder
        .wait()
        .expect("unable to wait for perf child command to exit");

    let perf_script_output = Command::new("perf")
        .arg("script")
        .output()
        .expect("unable to call perf script");

    let perf_reader = BufReader::new(&*perf_script_output.stdout);
    let mut collapsed = vec![];

    let collapsed_writer = BufWriter::new(&mut collapsed);

    let collapse_options = CollapseOptions::default();
    Folder::from(collapse_options)
        .collapse(perf_reader, collapsed_writer)
        .expect("unable to collapse generated profile data");

    let collapsed_reader = BufReader::new(&*collapsed);

    let flamegraph_file =
        File::create(output).expect("unable to create flamegraph.svg output file");
    let flamegraph_writer = BufWriter::new(flamegraph_file);

    let flamegraph_options = FlamegraphOptions::default();
    from_reader(flamegraph_options, collapsed_reader, flamegraph_writer)
        .expect("unable to generate a flamegraph from the collapsed stack data");
}
