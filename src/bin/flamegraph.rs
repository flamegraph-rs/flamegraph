use std::path::PathBuf;

use anyhow::anyhow;
use clap::{CommandFactory, Parser};
use clap_complete::Shell;

use flamegraph::Workload;

#[derive(Debug, Parser)]
#[clap(version)]
struct Opt {
    /// Profile a running process by pid (comma separated list)
    #[clap(short, long, value_delimiter(','))]
    pid: Vec<u32>,

    /// Generate shell completions for the given shell.
    #[clap(long, value_name = "SHELL", exclusive(true))]
    completions: Option<Shell>,

    #[clap(flatten)]
    graph: flamegraph::Options,

    #[clap(long = "perfdata", conflicts_with = "pid")]
    perf_file: Option<PathBuf>,

    #[clap(last = true)]
    trailing_arguments: Vec<String>,
}

fn main() -> anyhow::Result<()> {
    let opt = Opt::parse();

    if let Some(shell) = opt.completions {
        clap_complete::generate(
            shell,
            &mut Opt::command(),
            "flamegraph",
            &mut std::io::stdout(),
        );
        return Ok(());
    }

    opt.graph.check()?;

    let workload = if let Some(perf_file) = opt.perf_file {
        Workload::ReadPerf(perf_file)
    } else {
        match (opt.pid.is_empty(), opt.trailing_arguments.is_empty()) {
            (false, true) => Workload::Pid(opt.pid),
            (true, false) => Workload::Command(opt.trailing_arguments.clone()),
            (false, false) => return Err(anyhow!("cannot pass in command with --pid")),
            (true, true) => return Err(anyhow!("no workload given to generate a flamegraph for")),
        }
    };
    flamegraph::generate_flamegraph_for_workload(workload, opt.graph)
}
