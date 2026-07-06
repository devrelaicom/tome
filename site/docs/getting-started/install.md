---
title: Install
sidebar_position: 1
---

# Install Tome

Tome ships as a single self-contained executable. The semantic index, vector
search, and reranker runtime are compiled in. The search models are downloaded
the first time they're needed (or explicitly with `tome models download`) and
kept under `~/.tome/`. The default `medium` profile fetches a ~110 MB embedder
(`bge-base-en-v1.5`) and a ~563 MB reranker (`bge-reranker-large`); set
`[models] profile` in `~/.tome/config.toml` to select `small`, `medium`, or
`large`. No daemon, no account. Tome sends anonymous, opt-out usage telemetry (auto-disabled
under CI); turn it off with `tome telemetry off`. See
[`tome telemetry`](../reference/commands.md#tome-telemetry).

## Prerequisites

- **Binary install (Homebrew / prebuilt):** none. The binary is statically built
  for Linux and macOS.
- **`cargo install`:** Rust **1.93 or newer**, a C/C++ toolchain (for the bundled
  SQLite and `sqlite-vec` C extension), **CMake**, and a network connection (the
  ONNX Runtime used for inference is downloaded during the build).

## Homebrew (macOS)

```bash
brew install aaronbassett/tap/tome
```

## Cargo

```bash
cargo install tome-mcp
```

The crate is published as `tome-mcp`; it installs a binary named `tome`.

## From source

```bash
git clone https://github.com/devrelaicom/tome
cd tome
cargo build --release
# the binary is at target/release/tome
```

## Platform support

| Platform | x86_64 | aarch64 |
| --- | --- | --- |
| Linux | supported | supported |
| macOS | supported | supported |
| Windows | untested | untested |

Prebuilt binaries are provided for Linux and macOS on both `x86_64` and
`aarch64`. Windows is not currently tested.

## Verify the install

```bash
tome --version
```

If that prints a version, the installation succeeded — continue to the
[Quickstart](./quickstart.md).

## Shell completions

`tome completions <shell>` prints a completion script to stdout. Generating it
is a pure static operation over the command tree, so it reads no HOME, index, or
config — you can run it during shell setup before Tome is otherwise configured.
Supported shells are `bash`, `zsh`, `fish`, `powershell`, and `elvish`; an
unknown shell is a usage error (exit `2`) that lists the valid values.

Redirect the output to the file your shell loads completions from. For zsh:

```bash
tome completions zsh > ~/.zfunc/_tome
# ensure ~/.zfunc is on your fpath and compinit runs, e.g. in ~/.zshrc:
#   fpath=(~/.zfunc $fpath)
#   autoload -U compinit && compinit
```

For bash, source it directly or write it to a `bash_completion.d` file:

```bash
source <(tome completions bash)
# or: tome completions bash > /usr/local/etc/bash_completion.d/tome
```

See [`tome completions`](../reference/commands.md#tome-completions) for the
`fish`, `powershell`, and `elvish` recipes.
