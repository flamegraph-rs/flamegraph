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

    trailing_arguments: Vec<String>,
}

fn workload(opt: &Opt) -> Workload {
    match opt.pid {
        Some(p) => {
            if !opt.trailing_arguments.is_empty() {
                eprintln!("only a pid or command can be specified!");
                std::process::exit(1);
            }

            Workload::Pid(p)
        }
        None => {
            if opt.trailing_arguments.is_empty() {
                eprintln!("no workload given to generate a flamegraph for!");
                std::process::exit(1);
            }

            Workload::Command(
                opt.trailing_arguments.clone(),
            )
        }
    }
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
        &flamegraph_filename,
        opt.root,
        opt.frequency,
    );

    if opt.open {
        if let Err(e) = opener::open(&flamegraph_filename) {
            eprintln!(
                "Failed to open [{}]. Error: {}",
                flamegraph_filename.display(),
                e
            );
        }
    }
}
