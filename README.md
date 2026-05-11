# Tome

A Rust CLI (and eventually an MCP server) that makes Claude Code's plugin ecosystem work across other agentic coding harnesses — Cursor, Codex, Gemini CLI, OpenCode, and friends.

> **Status: pre-release, Phase 1.** Tome currently manages **catalogs** — Git-hosted collections of plugins. Installing plugins from catalogs into specific harnesses is part of Phase 2.

## Install (Phase 1)

```sh
git clone https://github.com/devrelaicom/tome.git
cd tome
cargo install --path .
```

Requires Rust ≥ 1.93 and a system `git` on the executable path.

## Quick example

```sh
# Register a public catalog
tome catalog add midnight/midnight-experts

# See what's in it
tome catalog show midnight-experts

# Keep it fresh
tome catalog update midnight-experts

# Or, scriptably
tome catalog list --json | jq -r '.name'
```

## Documentation

- **What Tome will do in each phase:** [`PRDs/`](./PRDs/)
- **Project principles:** [`CONSTITUTION.md`](./CONSTITUTION.md)
- **Active feature specification:** [`specs/001-phase-1-foundations/`](./specs/001-phase-1-foundations/)
- **Contributor on-ramp:** [`CONTRIBUTING.md`](./CONTRIBUTING.md)

## Licence

Dual-licensed under either of:

- Apache License, Version 2.0 ([`LICENSE-APACHE`](./LICENSE-APACHE))
- MIT license ([`LICENSE-MIT`](./LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in Tome by you, as defined in the Apache-2.0 licence, shall be dual-licensed as above, without any additional terms or conditions.
