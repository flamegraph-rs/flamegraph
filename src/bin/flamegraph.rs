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

    /// Profile a running process by pid
    #[structopt(short = "p", long = "pid")]
    pid: Option<u32>,

    #[structopt(flatten)]
    graph: flamegraph::Options,

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

    flamegraph::generate_flamegraph_for_workload(workload, &flamegraph_filename, opt.graph)?;

    if opt.open {
        opener::open(&flamegraph_filename).context(format!(
            "failed to open '{}'",
            flamegraph_filename.display()
        ))?;
    }

    Ok(())
}
