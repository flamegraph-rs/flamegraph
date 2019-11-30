use std::path::PathBuf;

use structopt::StructOpt;

use flamegraph::Workload;

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

fn workload(opt: &Opt) -> Workload {
    if opt.trailing_arguments.is_empty() {
        eprintln!("no workload given to generate a flamegraph for!");
        std::process::exit(1);
    }

    Workload::Command(opt.trailing_arguments.clone())
}

fn main() {
    let mut opt = Opt::from_args();

    let workload = workload(&opt);

    let flamegraph_filename: PathBuf = opt
        .output
        .take()
        .unwrap_or("flamegraph.svg".into());

    flamegraph::generate_flamegraph_for_workload(
        workload,
        flamegraph_filename,
        opt.root,
    );
}
