use std::{
    env::args,
    fs::File,
    io::{prelude::*, BufReader, BufWriter},
    process::{exit, Command},
};

use inferno::{
    collapse::{
        perf::{Folder, Options as CollapseOptions},
        Collapse,
    },
    flamegraph::{from_reader, Options as FlamegraphOptions},
};

fn usage() -> ! {
    println!("{} [command to run under profile]", args().nth(0).unwrap(),);
    exit(0);
}

fn main() -> Result<(), ()> {
    let cmd: String = if args().len() == 2 {
        "cargo run".into()
    } else if args().nth(1).unwrap() == "-h" {
        usage();
    } else {
        args().skip(1).collect::<Vec<_>>().join(" ")
    };

    let gen = format!("perf record -F 99 -g {}", cmd);
    println!("running: {}", gen);

    let mut gen_parts = gen.split_whitespace();

    let mut command = Command::new(gen_parts.next().unwrap());

    for arg in gen_parts {
        command.arg(arg);
    }

    let mut recorder = command.spawn().unwrap();
    recorder.wait().unwrap();

    let perf_script_output = Command::new("perf").arg("script").output().unwrap();

    let perf_reader = BufReader::new(&*perf_script_output.stdout);
    let mut collapsed = vec![];

    let collapsed_writer = BufWriter::new(&mut collapsed);

    let collapse_options = CollapseOptions::default();
    Folder::from(collapse_options).collapse(perf_reader, collapsed_writer);

    let collapsed_reader = BufReader::new(&*collapsed);

    let flamegraph_file = File::create("flamegraph.svg").unwrap();
    let flamegraph_writer = BufWriter::new(flamegraph_file);

    let flamegraph_options = FlamegraphOptions::default();
    from_reader(flamegraph_options, collapsed_reader, flamegraph_writer).unwrap();

    Ok(())
}
