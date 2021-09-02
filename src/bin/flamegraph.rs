use std::path::PathBuf;

use anyhow::{anyhow, Context};
use structopt::StructOpt;

use flamegraph::Workload;

#[derive(Debug, StructOpt)]
#[structopt(
    setting = structopt::clap::AppSettings::TrailingVarArg
)]
struct Opt {
    /// Output file, flamegraph.svg if not present
    #[structopt(parse(from_os_str), short = "o", long = "output")]
    output: Option<PathBuf>,

    /// Open the output .svg file with default program
    #[structopt(long = "open")]
    open: bool,

    /// Run with root privileges (using `sudo`)
    #[structopt(long = "root")]
    root: bool,

    /// Profile a running process by pid
    #[structopt(short = "p", long = "pid")]
    pid: Option<u32>,

    /// Sampling frequency
    #[structopt(short = "F", long = "freq")]
    frequency: Option<u32>,

    /// Print extra output to help debug problems
    #[structopt(short = "v", long = "verbose")]
    verbose: bool,

    /// Custom command for invoking perf/dtrace
    #[structopt(short = "c", long = "cmd", conflicts_with = "freq")]
    custom_cmd: Option<String>,

    /// Disable inlining for perf script because of performance issues
    #[structopt(long = "no-inline")]
    script_no_inline: bool,

    #[structopt(flatten)]
    flamegraph_options: flamegraph::FlamegraphOptions,

    trailing_arguments: Vec<String>,
}

fn workload(opt: &Opt) -> anyhow::Result<Workload> {
    match (opt.pid, opt.trailing_arguments.is_empty()) {
        (Some(p), true) => Ok(Workload::Pid(p)),
        (Some(_), false) => Err(anyhow!("cannot pass in command with --pid")),
        (None, true) => Err(anyhow!("no workload given to generate a flamegraph for")),
        (None, false) => Ok(Workload::Command(opt.trailing_arguments.clone())),
    }
}

fn main() -> anyhow::Result<()> {
    let mut opt = Opt::from_args();

    let workload = workload(&opt)?;

    let flamegraph_filename: PathBuf = opt.output.take().unwrap_or_else(|| "flamegraph.svg".into());

    flamegraph::generate_flamegraph_for_workload(
        workload,
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
