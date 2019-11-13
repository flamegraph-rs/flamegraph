use std::path::PathBuf;

use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(raw(
    setting = "structopt::clap::AppSettings::TrailingVarArg"
))]
struct Opt {
    /// Output file, flamegraph.svg if not present
    #[structopt(
        parse(from_os_str),
        short = "o",
        long = "output"
    )]
    output: Option<PathBuf>,

    /// Run with root privileges (using `sudo`)
    #[structopt(long = "root")]
    root: bool,

    trailing_arguments: Vec<String>,
}

fn workload(opt: &Opt) -> Vec<String> {
    if opt.trailing_arguments.is_empty() {
        eprintln!("no workload given to generate a flamegraph for!");
        std::process::exit(1);
    }

    opt.trailing_arguments.clone()
}

fn main() {
    let mut opt = Opt::from_args();

    let workload = workload(&opt);

    let flamegraph_filename: PathBuf = opt
        .output
        .take()
        .unwrap_or("flamegraph.svg".into());

    flamegraph::generate_flamegraph_by_running_command(
        workload,
        flamegraph_filename,
        opt.root,
    );
}
