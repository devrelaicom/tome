---
title: Install
sidebar_position: 1
---

# Install Tome

Tome ships as a single self-contained executable. The semantic index, vector
search, and reranker runtime are compiled in. The two search models — a 32 MB
embedder and a 266 MB reranker — are downloaded the first time they're needed
(or explicitly with `tome models download`) and kept under `~/.tome/`. No
daemon, no account, no telemetry.

## Prerequisites

- **Binary install (Homebrew / prebuilt):** none. The binary is statically built
  for Linux and macOS.
- **`cargo install`:** Rust **1.93 or newer**, a C/C++ toolchain (for the bundled
  SQLite and `sqlite-vec` C extension), **CMake**, and a network connection (the
  ONNX Runtime used for inference is downloaded during the build).

## Homebrew (macOS)

```bash
brew install aaronbassett/homebrew-tap/tome
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
