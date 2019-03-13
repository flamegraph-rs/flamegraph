use std::{
    fs::File,
    io::{BufReader, BufWriter},
    process::Command,
};

#[cfg(target_os = "linux")]
use inferno::collapse::perf::{
    Folder, Options as CollapseOptions,
};

#[cfg(not(target_os = "linux"))]
use inferno::collapse::dtrace::{
    Folder, Options as CollapseOptions,
};

use inferno::{
    collapse::Collapse,
    flamegraph::{
        from_reader, Options as FlamegraphOptions,
    },
};

#[cfg(target_os = "linux")]
mod arch {
    use super::*;

    pub const SPAWN_ERROR: &'static str =
        "could not spawn perf";
    pub const WAIT_ERROR: &'static str =
        "unable to wait for perf \
         child command to exit";

    pub(crate) fn initial_command(
        workload: String,
    ) -> Command {
        let mut command = Command::new("perf");

        for arg in "record -F 99 -g".split_whitespace() {
            command.arg(arg);
        }

        for item in workload.split_whitespace() {
            command.arg(item);
        }

        command
    }

    pub fn output() -> Vec<u8> {
        Command::new("perf")
            .arg("script")
            .output()
            .expect("unable to call perf script")
            .stdout
    }
}

#[cfg(not(target_os = "linux"))]
mod arch {
    use super::*;

    pub const SPAWN_ERROR: &'static str =
        "could not spawn dtrace";
    pub const WAIT_ERROR: &'static str =
        "unable to wait for dtrace \
         child command to exit";

    pub(crate) fn initial_command(
        workload: String,
    ) -> Command {
        let mut command = Command::new("dtrace");

        let dtrace_script = "profile-997 /pid == $target/ { @[ustack(100)] = count(); }";

        command.arg("-x");
        command.arg("ustackframes=100");

        command.arg("-n");
        command.arg(&dtrace_script);

        command.arg("-o");
        command.arg("cargo-flamegraph.stacks");

        command.arg("-c");
        command.arg(&workload);

        command
    }

    pub fn output() -> Vec<u8> {
        let mut buf = vec![];
        let mut f = File::open("cargo-flamegraph.stacks")
            .expect("failed to open dtrace output file cargo-flamegraph.stacks");

        use std::io::Read;
        f.read_to_end(&mut buf).expect(
            "failed to read dtrace expected \
             output file cargo-flamegraph.stacks",
        );

        std::fs::remove_file("cargo-flamegraph.stacks")
            .expect(
                "unable to remove cargo-flamegraph.stacks \
                 temporary file",
            );

        buf
    }
}

pub fn generate_flamegraph_by_running_command<
    P: AsRef<std::path::Path>,
>(
    workload: String,
    flamegraph_filename: P,
) {
    let mut command = arch::initial_command(workload);

    let mut recorder =
        command.spawn().expect(arch::SPAWN_ERROR);
    let exit_status =
        recorder.wait().expect(arch::WAIT_ERROR);

    if !exit_status.success() {
        eprintln!("failed to sample program");
        std::process::exit(1);
    }

    let output = arch::output();

    let perf_reader = BufReader::new(&*output);

    let mut collapsed = vec![];

    let collapsed_writer = BufWriter::new(&mut collapsed);

    let collapse_options = CollapseOptions::default();

    Folder::from(collapse_options)
        .collapse(perf_reader, collapsed_writer)
        .expect(
            "unable to collapse generated profile data",
        );

    let collapsed_reader = BufReader::new(&*collapsed);

    println!(
        "writing flamegraph to {:?}",
        flamegraph_filename.as_ref()
    );

    let flamegraph_file = File::create(flamegraph_filename)
        .expect(
            "unable to create flamegraph.svg output file",
        );

    let flamegraph_writer = BufWriter::new(flamegraph_file);

    let flamegraph_options = FlamegraphOptions::default();

    from_reader(flamegraph_options, collapsed_reader, flamegraph_writer)
        .expect("unable to generate a flamegraph from the collapsed stack data");
}
