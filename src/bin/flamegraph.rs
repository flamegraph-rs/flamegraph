use std::path::PathBuf;

use anyhow::anyhow;
use clap::{CommandFactory, Parser};
use clap_complete::Shell;

use flamegraph::Workload;

#[derive(Debug, Parser)]
#[clap(version)]
struct Opt {
    /// Profile a running process by pid
    #[clap(short, long)]
    pid: Option<u32>,

    /// Generate shell completions for the given shell.
    #[clap(long, value_name = "SHELL", exclusive(true))]
    completions: Option<Shell>,

    #[clap(flatten)]
    graph: flamegraph::Options,

    /// Read perf data from the given file.
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
        let path = perf_file.to_str().unwrap();
        Workload::ReadPerf(path.to_string())
    } else {
        match (opt.pid, opt.trailing_arguments.is_empty()) {
            (Some(p), true) => Workload::Pid(p),
            (None, false) => Workload::Command(opt.trailing_arguments.clone()),
            (Some(_), false) => return Err(anyhow!("cannot pass in command with --pid")),
            (None, true) => return Err(anyhow!("no workload given to generate a flamegraph for")),
        }
    };
    flamegraph::generate_flamegraph_for_workload(workload, opt.graph)
}
