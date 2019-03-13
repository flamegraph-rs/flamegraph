# cargo-flamegraph

<p align="center">
  <img alt="example flamegraph image" src="https://raw.githubusercontent.com/ferrous-systems/cargo-flamegraph/master/example.svg" width="20%" height="auto" />
</p>

A simple cargo plugin that generates a flamegraph
for a given workload. It can be used to profile anything,
not just Rust projects! No perl or pipes required <3

Uses perf on linux and dtrace otherwise.

Windows is getting [dtrace support](https://techcommunity.microsoft.com/t5/Windows-Kernel-Internals/DTrace-on-Windows/ba-p/362902), so if you try this out please let us know how it goes :D

## Installation

`flamegraph` not `cargo-flamegraph`! (`cargo-flamegraph` is an inactive crate as of March 2019)

```
cargo install flamegraph
```

This will make the `cargo-flamegraph` binary
available in your cargo binary directory.
On most systems this is usually something
like `~/.cargo/bin`.

## Examples

```
# defaults to profiling cargo run, which will
# also profile the cargo compilation process
# unless you've previously issued `cargo build`
cargo flamegraph

# if you'd like to profile your release build:
cargo flamegraph --release

# if you'd like to profile a specific binary:
cargo flamegraph --bin=stress2

# if you want to pass arguments, as you would with cargo run:
cargo flamegraph -- my-command --my-arg my-value -m -f 

# if you'd like to profile an arbitrary executable:
cargo flamegraph --exec="/path/to/my/binary --some-arg 5"
```

## Usage

```
USAGE:
    cargo flamegraph [FLAGS] [OPTIONS] -- [[ARGS_FOR_YOUR_BINARY]]

FLAGS:
    -h, --help       Prints help information
    -r, --release    Activate release mode
    -V, --version    Prints version information

OPTIONS:
    -b, --bin <bin>              Binary to run
    -e, --exec <exec>            Other command to run
    -f, --features <features>    Build features to enable
    -o, --output <output>        Output file, flamegraph.svg if not present
```

## Enabling perf for use by unprivileged users

To enable perf without running as root, you may
lower the `perf_event_paranoid` value in proc
to an appropriate level for your environment.
The most permissive value is `-1` but may not
be acceptable for your security needs etc...

```bash
echo -1 | sudo tee /proc/sys/kernel/perf_event_paranoid
```

## Improving output when running with `--release`

Due to optimizations etc... sometimes the quality
of the information presented in the flamegraph will
suffer when profiling release builds. To counter this
to some extent, you may set the following in your 
`Cargo.toml` file:

```
[profile.release]
debug = true
```
