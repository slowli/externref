# externref CLI

`externref` CLI app allows transforming WASM modules that use `externref` shims to use real `externref` types.
This is a mandatory step to get the module to work correctly.

## Usage

The executable provides the same functionality as the WASM [`processor`]
from the `externref` crate. See its docs and the output of `externref --help`
for a detailed description of available options.

> [!WARNING]
>
> The processor should run before WASM optimization tools such as `wasm-opt` from binaryen.

The terminal capture below demonstrates transforming a test WASM module.
The capture includes the tracing output, which was switched on
by setting the `RUST_LOG` env variable. Tracing info includes each transformed function
and some other information that could be useful for debugging.

![Output with tracing](../assets/with-tracing.svg)

## Installation options

- [Use a pre-built binary](#downloads) for popular targets (x86_64 for Linux / macOS / Windows
  and AArch64 for macOS) from the `master` branch.
- Use a pre-built binary for popular targets from [GitHub Releases](https://github.com/slowli/externref/releases).
- [Use the app Docker image](docker.md).
- [Build from sources](build.md) using Rust / `cargo`.

## Downloads

> [!IMPORTANT]
>
> The binaries are updated on each push to the git repo branch. Hence, they may contain more bugs
> than the release binaries mentioned above.

| Platform | Architecture | Download link                                                                                             |
|:---------|:-------------|:----------------------------------------------------------------------------------------------------------|
| Linux    | x86_64, GNU  | [<i class="fa-solid fa-download"></i> Download](../assets/externref-main-x86_64-unknown-linux-gnu.tar.gz) |
| macOS    | x86_64       | [<i class="fa-solid fa-download"></i> Download](../assets/externref-main-x86_64-apple-darwin.tar.gz)      |
| macOS    | arm64        | [<i class="fa-solid fa-download"></i> Download](../assets/externref-main-aarch64-apple-darwin.tar.gz)     |
| Windows  | x86_64       | [<i class="fa-solid fa-download"></i> Download](../assets/externref-main-x86_64-pc-windows-msvc.zip)      |
