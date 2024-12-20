use std::{
    env,
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    path::PathBuf,
    process::{exit, Command, ExitStatus, Stdio},
    str::FromStr,
};

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

#[cfg(target_os = "linux")]
use inferno::collapse::perf::{Folder, Options as CollapseOptions};

#[cfg(not(target_os = "linux"))]
use inferno::collapse::dtrace::{Folder, Options as CollapseOptions};

#[cfg(unix)]
use signal_hook::consts::{SIGINT, SIGTERM};

use anyhow::{anyhow, Context};
use clap::{
    builder::{PossibleValuesParser, TypedValueParser},
    Args,
};
use inferno::{collapse::Collapse, flamegraph::color::Palette, flamegraph::from_reader};

pub enum Workload {
    Command(Vec<String>),
    Pid(Vec<u32>),
    ReadPerf(PathBuf),
}

#[cfg(target_os = "linux")]
mod arch {
    use std::fmt::Write;
    use std::time::Duration;

    use indicatif::{ProgressBar, ProgressStyle};

    use super::*;

    pub const SPAWN_ERROR: &str = "could not spawn perf";
    pub const WAIT_ERROR: &str = "unable to wait for perf child command to exit";

    pub(crate) fn initial_command(
        workload: Workload,
        sudo: Option<Option<&str>>,
        freq: u32,
        custom_cmd: Option<String>,
        verbose: bool,
        ignore_status: bool,
    ) -> Option<PathBuf> {
        let perf = if let Ok(path) = env::var("PERF") {
            path
        } else {
            if Command::new("perf")
                .arg("--help")
                .stderr(Stdio::null())
                .stdout(Stdio::null())
                .status()
                .is_err()
            {
                eprintln!("perf is not installed or not present in $PATH");
                exit(1);
            }

            String::from("perf")
        };
        let mut command = sudo_command(&perf, sudo);

        let args = custom_cmd.unwrap_or(format!("record -F {freq} --call-graph dwarf,16384 -g"));

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
                perf_output = Some(PathBuf::from(next_arg));
            }
        }

        let perf_output = match perf_output {
            Some(path) => path,
            None => {
                command.arg("-o");
                command.arg("perf.data");
                PathBuf::from("perf.data")
            }
        };

        match workload {
            Workload::Command(c) => {
                command.args(&c);
            }
            Workload::Pid(p) => {
                if let Some((first, pids)) = p.split_first() {
                    let mut arg = first.to_string();

                    for pid in pids {
                        write!(arg, ",{pid}").unwrap();
                    }

                    command.arg("-p");
                    command.arg(arg);
                }
            }
            Workload::ReadPerf(_) => (),
        }

        run(command, verbose, ignore_status);
        Some(perf_output)
    }

    pub fn output(
        perf_output: Option<PathBuf>,
        script_no_inline: bool,
        sudo: Option<Option<&str>>,
    ) -> anyhow::Result<Vec<u8>> {
        // We executed `perf record` with sudo, and will be executing `perf script` with sudo,
        // so that we can resolve privileged kernel symbols from /proc/kallsyms.
        let perf = env::var("PERF").unwrap_or_else(|_| "perf".to_string());
        let mut command = sudo_command(&perf, sudo);

        command.arg("script");

        // Force reading perf.data owned by another uid if it happened to be created earlier.
        command.arg("--force");

        if script_no_inline {
            command.arg("--no-inline");
        }

        if let Some(perf_output) = perf_output {
            command.arg("-i");
            command.arg(perf_output);
        }

        // perf script can take a long time to run. Notify the user that it is running
        // by using a spinner. Note that if this function exits before calling
        // spinner.finish(), then the spinner will be completely removed from the terminal.
        let spinner = ProgressBar::new_spinner().with_prefix("Running perf script");
        spinner.set_style(
            ProgressStyle::with_template("{prefix} [{elapsed}]: {spinner:.green}").unwrap(),
        );
        spinner.enable_steady_tick(Duration::from_millis(500));

        let result = command.output().context("unable to call perf script");
        spinner.finish();
        let output = result?;
        if !output.status.success() {
            anyhow::bail!(format!(
                "unable to run 'perf script': ({}) {}",
                output.status,
                std::str::from_utf8(&output.stderr)?
            ));
        }
        Ok(output.stdout)
    }
}

#[cfg(not(target_os = "linux"))]
mod arch {
    use super::*;

    pub const SPAWN_ERROR: &str = "could not spawn dtrace";
    pub const WAIT_ERROR: &str = "unable to wait for dtrace child command to exit";
    #[cfg(target_os = "windows")]
    pub const BLONDIE_ERROR: &str = "could not find dtrace and could not profile using blondie";

    #[cfg(target_os = "macos")]
    fn base_dtrace_command(sudo: Option<Option<&str>>) -> Command {
        // If DTrace is spawned from a parent process (or grandparent process etc.) running in Rosetta-emulated x86 mode
        // on an ARM mac, it will fail to trace the child process with a confusing syntax error in its stdlib .d file.
        // If the flamegraph binary, or the cargo binary, have been compiled as x86, this can cause all tracing to fail.
        // To work around that, we unconditionally wrap dtrace on MacOS in the "arch -64/-32" wrapper so it's always
        // running in the native architecture matching the bit width (32 oe 64) with which "flamegraph" was compiled.
        // NOTE that dtrace-as-x86 won't trace a deliberately-cross-compiled x86 binary running under Rosetta regardless
        // of "arch" wrapping; attempts to do that will fail with "DTrace cannot instrument translated processes".
        // NOTE that using the ARCHPREFERENCE environment variable documented here
        // (https://www.unix.com/man-page/osx/1/arch/) would be a much simpler solution to this issue, but it does not
        // seem to have any effect on dtrace when set (via Command::env, shell export, or std::env in the spawning
        // process).
        let mut command = sudo_command("arch", sudo);

        #[cfg(target_pointer_width = "64")]
        command.arg("-64".to_string());
        #[cfg(target_pointer_width = "32")]
        command.arg("-32".to_string());

        command.arg(env::var("DTRACE").unwrap_or_else(|_| "dtrace".to_string()));
        command
    }

    #[cfg(not(target_os = "macos"))]
    fn base_dtrace_command(sudo: Option<Option<&str>>) -> Command {
        let dtrace = env::var("DTRACE").unwrap_or_else(|_| "dtrace".to_string());
        sudo_command(&dtrace, sudo)
    }

    pub(crate) fn initial_command(
        workload: Workload,
        sudo: Option<Option<&str>>,
        freq: u32,
        custom_cmd: Option<String>,
        verbose: bool,
        ignore_status: bool,
    ) -> Option<PathBuf> {
        let mut command = base_dtrace_command(sudo);

        let dtrace_script = custom_cmd.unwrap_or(format!(
            "profile-{freq} /pid == $target/ \
             {{ @[ustack(100)] = count(); }}",
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
                    escaped.push_str(&arg.replace(' ', "\\ "));
                }

                command.arg("-c");
                command.arg(&escaped);

                #[cfg(target_os = "windows")]
                {
                    let mut help_test = crate::arch::base_dtrace_command(None);

                    let dtrace_found = help_test
                        .arg("--help")
                        .stderr(Stdio::null())
                        .stdout(Stdio::null())
                        .status()
                        .is_ok();
                    if !dtrace_found {
                        let mut command_builder = Command::new(&c[0]);
                        command_builder.args(&c[1..]);
                        print_command(&command_builder, verbose);

                        let trace = match blondie::trace_command(command_builder, false) {
                            Err(err) => {
                                eprintln!("{}: {:?}", BLONDIE_ERROR, err);
                                exit(1);
                            }
                            Ok(trace) => trace,
                        };

                        let f = std::fs::File::create("./cargo-flamegraph.stacks").unwrap();
                        let mut f = std::io::BufWriter::new(f);
                        trace.write_dtrace(&mut f).unwrap();

                        return None;
                    }
                }
            }
            Workload::Pid(p) => {
                for p in p {
                    command.arg("-p");
                    command.arg(p.to_string());
                }
            }
            Workload::ReadPerf(_) => (),
        }

        run(command, verbose, ignore_status);
        None
    }

    pub fn output(
        _: Option<PathBuf>,
        script_no_inline: bool,
        sudo: Option<Option<&str>>,
    ) -> anyhow::Result<Vec<u8>> {
        if script_no_inline {
            return Err(anyhow::anyhow!("--no-inline is only supported on Linux"));
        }

        // Ensure the file is readable by the current user if dtrace was run
        // with sudo.
        if sudo.is_some() {
            #[cfg(unix)]
            if let Ok(user) = env::var("USER") {
                Command::new("sudo")
                    .args(["chown", user.as_str(), "cargo-flamegraph.stacks"])
                    .spawn()
                    .expect(arch::SPAWN_ERROR)
                    .wait()
                    .expect(arch::WAIT_ERROR);
            }
        }

        let mut buf = vec![];
        let mut f = File::open("cargo-flamegraph.stacks")
            .context("failed to open dtrace output file 'cargo-flamegraph.stacks'")?;

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

fn sudo_command(command: &str, sudo: Option<Option<&str>>) -> Command {
    let sudo = match sudo {
        Some(sudo) => sudo,
        None => return Command::new(command),
    };

    let mut c = Command::new("sudo");
    if let Some(sudo_args) = sudo {
        c.arg(sudo_args);
    }
    c.arg(command);
    c
}

fn run(mut command: Command, verbose: bool, ignore_status: bool) {
    print_command(&command, verbose);
    let mut recorder = command.spawn().expect(arch::SPAWN_ERROR);
    let exit_status = recorder.wait().expect(arch::WAIT_ERROR);

    // only stop if perf exited unsuccessfully, but
    // was not killed by a signal (assuming that the
    // latter case usually means the user interrupted
    // it in some way)
    if !ignore_status && terminated_by_error(exit_status) {
        eprintln!("failed to sample program");
        exit(1);
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

fn print_command(cmd: &Command, verbose: bool) {
    if verbose {
        println!("command {:?}", cmd);
    }
}

pub fn generate_flamegraph_for_workload(workload: Workload, opts: Options) -> anyhow::Result<()> {
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

    let sudo = opts.root.as_ref().map(|inner| inner.as_deref());

    let perf_output = if let Workload::ReadPerf(perf_file) = workload {
        Some(perf_file)
    } else {
        arch::initial_command(
            workload,
            sudo,
            opts.frequency(),
            opts.custom_cmd,
            opts.verbose,
            opts.ignore_status,
        )
    };

    #[cfg(unix)]
    signal_hook::low_level::unregister(handler);

    let output = arch::output(perf_output, opts.script_no_inline, sudo)?;

    let perf_reader = BufReader::new(&*output);

    let mut collapsed = vec![];

    let collapsed_writer = BufWriter::new(&mut collapsed);

    #[allow(unused_mut)]
    let mut collapse_options = CollapseOptions::default();

    #[cfg(target_os = "linux")]
    {
        collapse_options.skip_after = opts.flamegraph_options.skip_after.clone();
    }

    Folder::from(collapse_options)
        .collapse(perf_reader, collapsed_writer)
        .context("unable to collapse generated profile data")?;

    if let Some(command) = opts.post_process {
        let command_vec = shlex::split(&command)
            .ok_or_else(|| anyhow!("unable to parse post-process command"))?;

        let mut child = Command::new(
            command_vec
                .first()
                .ok_or_else(|| anyhow!("unable to parse post-process command"))?,
        )
        .args(command_vec.get(1..).unwrap_or(&[]))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .with_context(|| format!("unable to execute {:?}", command_vec))?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("unable to capture post-process stdin"))?;

        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("unable to capture post-process stdout"))?;

        let thread_handle = std::thread::spawn(move || -> anyhow::Result<_> {
            let mut collapsed_processed = Vec::new();
            stdout.read_to_end(&mut collapsed_processed).context(
                "unable to read the processed stacks from the stdout of the post-process process",
            )?;
            Ok(collapsed_processed)
        });

        stdin
            .write_all(&collapsed)
            .context("unable to write the raw stacks to the stdin of the post-process process")?;
        drop(stdin);

        anyhow::ensure!(
            child.wait()?.success(),
            "post-process exited with a non zero exit code"
        );

        collapsed = thread_handle.join().unwrap()?;
    }

    let collapsed_reader = BufReader::new(&*collapsed);

    let flamegraph_filename = opts.output;
    println!("writing flamegraph to {:?}", flamegraph_filename);
    let flamegraph_file = File::create(&flamegraph_filename)
        .context("unable to create flamegraph.svg output file")?;

    let flamegraph_writer = BufWriter::new(flamegraph_file);

    let mut inferno_opts = opts.flamegraph_options.into_inferno();
    from_reader(&mut inferno_opts, collapsed_reader, flamegraph_writer)
        .context("unable to generate a flamegraph from the collapsed stack data")?;

    if opts.open {
        opener::open(&flamegraph_filename).context(format!(
            "failed to open '{}'",
            flamegraph_filename.display()
        ))?;
    }

    Ok(())
}

#[derive(Debug, Args)]
pub struct Options {
    /// Print extra output to help debug problems
    #[clap(short, long)]
    pub verbose: bool,

    /// Output file
    #[clap(short, long, default_value = "flamegraph.svg")]
    output: PathBuf,

    /// Open the output .svg file with default program
    #[clap(long)]
    open: bool,

    /// Run with root privileges (using `sudo`). Accepts an optional argument containing command line options which will be passed to sudo
    #[clap(long, value_name = "SUDO FLAGS")]
    pub root: Option<Option<String>>,

    /// Sampling frequency in Hz [default: 997]
    #[clap(short = 'F', long = "freq")]
    frequency: Option<u32>,

    /// Custom command for invoking perf/dtrace
    #[clap(short, long = "cmd")]
    custom_cmd: Option<String>,

    #[clap(flatten)]
    flamegraph_options: FlamegraphOptions,

    /// Ignores perf's exit code
    #[clap(long)]
    ignore_status: bool,

    /// Disable inlining for perf script because of performance issues
    #[clap(long = "no-inline")]
    script_no_inline: bool,

    /// Run a command to process the folded stacks, taking the input from stdin and outputting to
    /// stdout.
    #[clap(long)]
    post_process: Option<String>,
}

impl Options {
    pub fn check(&self) -> anyhow::Result<()> {
        // Manually checking conflict because structopts `conflicts_with` leads
        // to a panic in completion generation for zsh at the moment (see #158)
        match self.frequency.is_some() && self.custom_cmd.is_some() {
            true => Err(anyhow!(
                "Cannot pass both a custom command and a frequency."
            )),
            false => Ok(()),
        }
    }

    pub fn frequency(&self) -> u32 {
        self.frequency.unwrap_or(997)
    }
}

#[derive(Debug, Args)]
pub struct FlamegraphOptions {
    /// Set title text in SVG
    #[clap(long, value_name = "STRING")]
    pub title: Option<String>,

    /// Set second level title text in SVG
    #[clap(long, value_name = "STRING")]
    pub subtitle: Option<String>,

    /// Colors are selected such that the color of a function does not change between runs
    #[clap(long)]
    pub deterministic: bool,

    /// Plot the flame graph up-side-down
    #[clap(short, long)]
    pub inverted: bool,

    /// Generate stack-reversed flame graph
    #[clap(long)]
    pub reverse: bool,

    /// Set embedded notes in SVG
    #[clap(long, value_name = "STRING")]
    pub notes: Option<String>,

    /// Omit functions smaller than <FLOAT> pixels
    #[clap(long, default_value = "0.01", value_name = "FLOAT")]
    pub min_width: f64,

    /// Image width in pixels
    #[clap(long)]
    pub image_width: Option<usize>,

    /// Color palette
    #[clap(
        long,
        value_parser = PossibleValuesParser::new(Palette::VARIANTS).map(|s| Palette::from_str(&s).unwrap())
    )]
    pub palette: Option<Palette>,

    /// Cut off stack frames below <FUNCTION>; may be repeated
    #[cfg(target_os = "linux")]
    #[clap(long, value_name = "FUNCTION")]
    pub skip_after: Vec<String>,

    /// Produce a flame chart (sort by time, do not merge stacks)
    #[clap(long = "flamechart", conflicts_with = "reverse")]
    pub flame_chart: bool,
}

impl FlamegraphOptions {
    pub fn into_inferno(self) -> inferno::flamegraph::Options<'static> {
        let mut options = inferno::flamegraph::Options::default();
        if let Some(title) = self.title {
            options.title = title;
        }
        options.subtitle = self.subtitle;
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
        options.flame_chart = self.flame_chart;

        options
    }
}
