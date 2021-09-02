use std::{
    env,
    fs::File,
    io::{BufReader, BufWriter},
    process::{Command, ExitStatus},
};

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

#[cfg(target_os = "linux")]
use inferno::collapse::perf::{Folder, Options as CollapseOptions};

#[cfg(not(target_os = "linux"))]
use inferno::collapse::dtrace::{Folder, Options as CollapseOptions};

#[cfg(unix)]
use signal_hook::consts::{SIGINT, SIGTERM};

use anyhow::Context;
use inferno::{
    collapse::Collapse,
    flamegraph::color::Palette,
    flamegraph::{defaults, from_reader},
};

pub enum Workload {
    Command(Vec<String>),
    Pid(u32),
}

#[cfg(target_os = "linux")]
mod arch {
    use super::*;

    pub const SPAWN_ERROR: &str = "could not spawn perf";
    pub const WAIT_ERROR: &str = "unable to wait for perf child command to exit";

    pub(crate) fn initial_command(
        workload: Workload,
        sudo: bool,
        freq: Option<u32>,
        custom_cmd: Option<String>,
    ) -> (Command, Option<String>) {
        let perf = env::var("PERF").unwrap_or_else(|_| "perf".to_string());

        let mut command = if sudo {
            let mut c = Command::new("sudo");
            c.arg(perf);
            c
        } else {
            Command::new(perf)
        };

        let args = custom_cmd.unwrap_or(format!(
            "record -F {} --call-graph dwarf -g",
            freq.unwrap_or(997)
        ));

        let mut perf_output = None;
        let mut args = args.split_whitespace();
        while let Some(arg) = args.next() {
            command.arg(arg);

            // Detect if user is setting `perf record`
            // output file with `-o`. If so, save it in
            // order to correctly compute perf's output in
            // `Self::output`.
            if arg == "-o" {
                let next_arg = args.next().expect("missing '-o' argument");
                command.arg(next_arg);
                perf_output = Some(next_arg.to_string());
            }
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

        (command, perf_output)
    }

    pub fn output(perf_output: Option<String>, script_no_inline: bool) -> anyhow::Result<Vec<u8>> {
        let perf = env::var("PERF").unwrap_or_else(|_| "perf".to_string());
        let mut command = Command::new(perf);

        command.arg("script");
        if script_no_inline {
            command.arg("--no-inline");
        }

        if let Some(perf_output) = perf_output {
            command.arg("-i");
            command.arg(perf_output);
        }

        Ok(command
            .output()
            .context("unable to call perf script")?
            .stdout)
    }
}

#[cfg(not(target_os = "linux"))]
mod arch {
    use super::*;

    pub const SPAWN_ERROR: &'static str = "could not spawn dtrace";
    pub const WAIT_ERROR: &'static str = "unable to wait for dtrace child command to exit";

    pub(crate) fn initial_command(
        workload: Workload,
        sudo: bool,
        freq: Option<u32>,
        custom_cmd: Option<String>,
    ) -> (Command, Option<String>) {
        let dtrace = env::var("DTRACE").unwrap_or_else(|_| "dtrace".to_string());

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
                let mut escaped = String::new();
                for (i, arg) in c.iter().enumerate() {
                    if i > 0 {
                        escaped.push(' ');
                    }
                    escaped.push_str(&arg.replace(" ", "\\ "));
                }

                command.arg("-c");
                command.arg(&escaped);
            }
            Workload::Pid(p) => {
                command.arg("-p");
                command.arg(p.to_string());
            }
        }

        (command, None)
    }

    pub fn output(_: Option<String>, script_no_inline: bool) -> anyhow::Result<Vec<u8>> {
        if script_no_inline {
            return Err(anyhow::anyhow!("--no-inline is only supported on Linux"));
        }

        let mut buf = vec![];
        let mut f = File::open("cargo-flamegraph.stacks")
            .context("failed to open dtrace output file 'cargo-flamegraph.stacks'")?;

        use std::io::Read;
        f.read_to_end(&mut buf)
            .context("failed to read dtrace expected output file 'cargo-flamegraph.stacks'")?;

        std::fs::remove_file("cargo-flamegraph.stacks")
            .context("unable to remove temporary file 'cargo-flamegraph.stacks'")?;

        // Workaround #32 - fails parsing invalid utf8 dtrace output
        //
        // Intermittently, invalid utf-8 is found in cargo-flamegraph.stacks, which
        // causes parsing to blow up with the error:
        //
        // > unable to collapse generated profile data: Custom { kind: InvalidData, error: StringError("stream did not contain valid UTF-8") }
        //
        // So here we just lossily re-encode to hopefully work around the underlying problem
        let string = String::from_utf8_lossy(&buf);
        let reencoded_buf = string.as_bytes().to_owned();

        if reencoded_buf != buf {
            println!("Lossily converted invalid utf-8 found in cargo-flamegraph.stacks");
        }

        Ok(reencoded_buf)
    }
}

#[cfg(unix)]
fn terminated_by_error(status: ExitStatus) -> bool {
    status
        .signal() // the default needs to be true because that's the neutral element for `&&`
        .map_or(true, |code| code != SIGINT && code != SIGTERM)
        && !status.success()
}

#[cfg(not(unix))]
fn terminated_by_error(status: ExitStatus) -> bool {
    !status.success()
}

// False positive in clippy for non-exhaustive struct FlamegraphOptions:
// https://github.com/rust-lang/rust-clippy/issues/6559
#[allow(clippy::field_reassign_with_default)]
pub fn generate_flamegraph_for_workload<P: AsRef<std::path::Path>>(
    workload: Workload,
    flamegraph_filename: P,
    sudo: bool,
    script_no_inline: bool,
    freq: Option<u32>,
    custom_cmd: Option<String>,
    mut flamegraph_options: inferno::flamegraph::Options,
    verbose: bool,
) -> anyhow::Result<()> {
    // Handle SIGINT with an empty handler. This has the
    // implicit effect of allowing the signal to reach the
    // process under observation while we continue to
    // generate our flamegraph.  (ctrl+c will send the
    // SIGINT signal to all processes in the foreground
    // process group).
    #[cfg(unix)]
    let handler = unsafe {
        signal_hook::low_level::register(SIGINT, || {}).expect("cannot register signal handler")
    };

    let (mut command, perf_output) = arch::initial_command(workload, sudo, freq, custom_cmd);
    if verbose {
        println!("command {:?}", command);
    }

    let mut recorder = command.spawn().expect(arch::SPAWN_ERROR);

    let exit_status = recorder.wait().expect(arch::WAIT_ERROR);

    #[cfg(unix)]
    signal_hook::low_level::unregister(handler);

    // only stop if perf exited unsuccessfully, but
    // was not killed by a signal (assuming that the
    // latter case usually means the user interrupted
    // it in some way)
    if terminated_by_error(exit_status) {
        eprintln!("failed to sample program");
        std::process::exit(1);
    }

    let output = arch::output(perf_output, script_no_inline)?;

    let perf_reader = BufReader::new(&*output);

    let mut collapsed = vec![];

    let collapsed_writer = BufWriter::new(&mut collapsed);

    let collapse_options = CollapseOptions::default();

    Folder::from(collapse_options)
        .collapse(perf_reader, collapsed_writer)
        .context("unable to collapse generated profile data")?;

    let collapsed_reader = BufReader::new(&*collapsed);

    println!("writing flamegraph to {:?}", flamegraph_filename.as_ref());

    let flamegraph_file =
        File::create(flamegraph_filename).context("unable to create flamegraph.svg output file")?;

    let flamegraph_writer = BufWriter::new(flamegraph_file);

    from_reader(&mut flamegraph_options, collapsed_reader, flamegraph_writer)
        .context("unable to generate a flamegraph from the collapsed stack data")
}

#[derive(Debug, structopt::StructOpt)]
pub struct FlamegraphOptions {
    /// Colors are selected such that the color of a function does not change between runs
    #[structopt(long = "deterministic", conflicts_with = "hash")]
    pub deterministic: bool,

    /// Plot the flame graph up-side-down
    #[structopt(short = "i", long = "inverted")]
    pub inverted: bool,

    /// Generate stack-reversed flame graph
    #[structopt(long = "reverse", conflicts_with = "no-sort")]
    pub reverse: bool,

    /// Set embedded notes in SVG
    #[structopt(long = "notes", value_name = "STRING")]
    pub notes: Option<String>,

    /// Omit functions smaller than <FLOAT> pixels
    #[structopt(
            long = "min-width",
            default_value = &defaults::str::MIN_WIDTH,
            value_name = "FLOAT"
        )]
    pub min_width: f64,

    /// Image width in pixels
    #[structopt(long = "image-width")]
    pub image_width: Option<usize>,

    /// Color palette
    #[structopt(
        long = "palette",
        possible_values = &[
            "hot", "mem", "io", "red", "green", "blue", "aqua", "yellow",
            "purple", "orange", "wakeup", "java", "perl", "js", "rust"
        ]
    )]
    pub palette: Option<Palette>,
}

impl FlamegraphOptions {
    pub fn into_inferno(self) -> inferno::flamegraph::Options<'static> {
        let mut options = inferno::flamegraph::Options::default();
        options.deterministic = self.deterministic;
        if self.inverted {
            options.direction = inferno::flamegraph::Direction::Inverted;
        }
        options.reverse_stack_order = self.reverse;
        options.notes = self.notes.unwrap_or_default();
        options.min_width = self.min_width;
        options.image_width = self.image_width;
        if let Some(palette) = self.palette {
            options.colors = palette;
        }

        options
    }
}
