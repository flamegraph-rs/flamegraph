use anyhow::anyhow;
use structopt::StructOpt;

use flamegraph::Workload;

#[derive(Debug, StructOpt)]
#[structopt(
    setting = structopt::clap::AppSettings::TrailingVarArg
)]
struct Opt {
    /// Profile a running process by pid
    #[structopt(short = "p", long = "pid")]
    pid: Option<u32>,

    #[structopt(flatten)]
    graph: flamegraph::Options,

    trailing_arguments: Vec<String>,
}

fn main() -> anyhow::Result<()> {
    let opt = Opt::from_args();
    let workload = match (opt.pid, opt.trailing_arguments.is_empty()) {
        (Some(p), true) => Workload::Pid(p),
        (None, false) => Workload::Command(opt.trailing_arguments.clone()),
        (Some(_), false) => return Err(anyhow!("cannot pass in command with --pid")),
        (None, true) => return Err(anyhow!("no workload given to generate a flamegraph for")),
    };
    flamegraph::generate_flamegraph_for_workload(workload, opt.graph)
}
