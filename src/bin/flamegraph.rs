use std::path::PathBuf;

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

    /// Generate shell completions for the given shell.
    #[structopt(long = "completions", conflicts_with = "pid")]
    completions: Option<structopt::clap::Shell>,

    #[structopt(flatten)]
    graph: flamegraph::Options,

    #[structopt(parse(from_os_str), long = "perfdata", conflicts_with = "pid")]
    perf_file: Option<PathBuf>,

    trailing_arguments: Vec<String>,
}

fn main() -> anyhow::Result<()> {
    let opt = Opt::from_args();

    if let Some(shell) = opt.completions {
        return match opt.trailing_arguments.is_empty() {
            true => {
                Opt::clap().gen_completions_to("flamegraph", shell, &mut std::io::stdout().lock());
                Ok(())
            }
            false => {
                return Err(anyhow!(
                    "command arguments cannot be used with --completions <completions>"
                ))
            }
        };
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
