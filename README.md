# cargo-flamegraph

A simple cargo plugin that generates a flamegraph
for a workload.

Currently only linux is supported via perf, but
this is going to change as inferno gains support
for others.

## Installation

```
cargo install flamegraph
```

This will make the `cargo-flamegraph` binary
available in your cargo binary directory.
On linux systems this is usually something
like `~/.cargo/bin`.

## Usage

```
# defaults to profiling cargo run, which will
# also profile the cargo compilation process
# unless you've previously issued `cargo build`
cargo flamegraph

# if you'd like to profile your release build:
cargo flamegraph --release

# if you'd like to profile a specific binary:
cargo flamegraph --bin=stress2

# if you'd like to profile an arbitrary executable:
cargo flamegraph --exec="sleep 10"
```

## Enabling perf for use by unpriviledged users

To enable perf without running as root, you may
lower the `perf_event_paranoid` value in proc
to an appropriate level for your environment.
The most permissive value is `-1` but may not
be acceptable for your security needs etc...

```bash
echo -1 | sudo tee /proc/sys/kernel/perf_event_paranoid
```
