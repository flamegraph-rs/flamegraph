use std::{
    env,
    fs::File,
    io::{BufReader, BufWriter},
    process::{Command, ExitStatus},
};

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

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

#[cfg(unix)]
use signal_hook;

pub enum Workload {
    Command(Vec<String>),
    Pid(u32),
}

#[cfg(target_os = "linux")]
mod arch {
    use super::*;

    pub const SPAWN_ERROR: &'static str =
        "could not spawn perf";
    pub const WAIT_ERROR: &'static str =
        "unable to wait for perf \
         child command to exit";

    pub(crate) fn initial_command(
        workload: Workload,
        sudo: bool,
        freq: Option<u32>,
        custom_cmd: Option<String>,
    ) -> Command {
        let perf = env::var("PERF")
            .unwrap_or_else(|_| "perf".to_string());

        let mut command = if sudo {
            let mut c = Command::new("sudo");
            c.arg(perf);
            c
        } else {
            Command::new(perf)
        };

        let args = custom_cmd.unwrap_or(format!(
            "record -F {} --call-graph dwarf -g",
            freq.unwrap_or(99)
        ));

        for arg in args.split_whitespace() {
            command.arg(arg);
        }

        match workload {
            Workload::Command(c) => {
                command.args(&c);
            }
            Workload::Pid(p) => {
                command.arg("-p");
                command.arg(p.to_string());
            }
        }

        command
    }

    pub fn output() -> Vec<u8> {
        let perf = env::var("PERF")
            .unwrap_or_else(|_| "perf".to_string());
        Command::new(perf)
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
        workload: Workload,
        sudo: bool,
        freq: Option<u32>,
        custom_cmd: Option<String>,
    ) -> Command {
        let dtrace = env::var("DTRACE")
            .unwrap_or_else(|_| "dtrace".to_string());

        let mut command = if sudo {
            let mut c = Command::new("sudo");
            c.arg(dtrace);
            c
        } else {
            Command::new(dtrace)
        };

        let dtrace_script = custom_cmd.unwrap_or(format!(
            "profile-{} /pid == $target/ \
             {{ @[ustack(100)] = count(); }}",
            freq.unwrap_or(997)
        ));

        command.arg("-x");
        command.arg("ustackframes=100");

        command.arg("-n");
        command.arg(&dtrace_script);

        command.arg("-o");
        command.arg("cargo-flamegraph.stacks");

        match workload {
            Workload::Command(c) => {
                command.arg("-c");
                command.args(&c);
            }
            Workload::Pid(p) => {
                command.arg("-p");
                command.arg(p.to_string());
            }
        }

        command
    }

    pub fn output() -> Vec<u8> {
        let mut buf = vec![];
        let mut f = File::open("cargo-flamegraph.stacks")
            .expect(
                "failed to open dtrace output \
                 file cargo-flamegraph.stacks",
            );

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

#[cfg(unix)]
fn terminated_by_error(status: ExitStatus) -> bool {
    status
        .signal() // the default needs to be true because that's the neutral element for `&&`
        .map_or(true, |code| {
            code != signal_hook::SIGINT
                && code != signal_hook::SIGTERM
        })
        && !status.success()
}

#[cfg(not(unix))]
fn terminated_by_error(status: ExitStatus) -> bool {
    !status.success()
}

pub fn generate_flamegraph_for_workload<
    P: AsRef<std::path::Path>,
>(
    workload: Workload,
    flamegraph_filename: P,
    sudo: bool,
    freq: Option<u32>,
    custom_cmd: Option<String>,
) {
    // Handle SIGINT with an empty handler. This has the
    // implicit effect of allowing the signal to reach the
    // process under observation while we continue to
    // generate our flamegraph.  (ctrl+c will send the
    // SIGINT signal to all processes in the foreground
    // process group).
    #[cfg(unix)]
    let handler = unsafe {
        signal_hook::register(signal_hook::SIGINT, || {})
            .expect("cannot register signal handler")
    };

    let mut command = arch::initial_command(
        workload, sudo, freq, custom_cmd,
    );

    let mut recorder =
        command.spawn().expect(arch::SPAWN_ERROR);

    let exit_status =
        recorder.wait().expect(arch::WAIT_ERROR);

    #[cfg(unix)]
    signal_hook::unregister(handler);

    // only stop if perf exited unsuccessfully, but
    // was not killed by a signal (assuming that the
    // latter case usually means the user interrupted
    // it in some way)
    if terminated_by_error(exit_status) {
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

    let mut flamegraph_options =
        FlamegraphOptions::default();

    from_reader(
        &mut flamegraph_options,
        collapsed_reader,
        flamegraph_writer,
    )
    .expect(
        "unable to generate a flamegraph \
         from the collapsed stack data",
    );
}
