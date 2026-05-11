# Stack — Tome

This document records the *intended* technology stack derived from the Phase 1 PRD and constitution. No source code has been written yet; this file will be expanded once the first commit of Rust source lands.

## Languages

- **Rust** (stable channel) — the sole implementation language.
  - **Edition**: latest stable; pinned in `Cargo.toml`.
  - **MSRV**: the current stable release at project start, pinned in `Cargo.toml` and verified in CI.
  - **Toolchain pinning**: `rust-toolchain.toml` pins the stable channel and the `rustfmt` and `clippy` components.

## Runtime

- **Synchronous**. No async runtime in Phase 1. `tokio` is deliberately not a dependency; it is pulled in only when async genuinely appears (expected with the MCP server in a later phase).

## Build / packaging

- **Cargo** workspace-of-one. Single binary crate `tome`. Workspace splitting is deferred until justified by code size.
- **`cargo install --path .`** is the Phase 1 install path. Cross-platform release tooling is deferred.

## Direct dependencies (Phase 1)

| Crate | Purpose |
|---|---|
| `clap` (`derive` feature) | CLI argument parsing |
| `serde` + `serde_derive` | Configuration and manifest (de)serialisation |
| `toml` | Configuration and manifest format |
| `anyhow` | Application-level error handling |
| `thiserror` | Typed errors for library-shaped modules |
| `tracing` + `tracing-subscriber` | Structured logging |
| `directories` | XDG / platform-aware paths |
| `sha2` | URL hashing for cache directory naming |

Constraints from the constitution (§Operational Constraints):

- Each new dependency must justify itself.
- The release binary must remain under 10 MB stripped.
- Licences are constrained by `cargo-deny`: allow `MIT`, `Apache-2.0`, `MIT-0`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `Unicode-DFS-2016`, `Zlib`; deny GPL / AGPL / LGPL family.

## External tools (run by Tome at runtime)

- **`git`** — shelled out via `std::process::Command` for clone, checkout, fetch, and reset operations. `libgit2` is explicitly rejected (constitution principle XII).

## Quality tooling

- **`cargo fmt`** (rustfmt) — formatting.
- **`cargo clippy`** with `-D warnings` — lints promoted to errors.
- **`typos`** — typo detection in source and docs.
- **`cocogitto`** (`cog`) — Conventional Commits validation.
- **`cargo-audit`** — RustSec advisory database checks (weekly + PR).
- **`cargo-deny`** — licence allowlist, advisory, source allowlist, duplicate-version warnings.
- **`cargo-llvm-cov`** + Codecov — coverage (nice-to-have, not Phase 1 blocking).

## Local automation

- **Lefthook** — pre-commit (fmt, clippy, typos in parallel), commit-msg (cocogitto), pre-push (`cargo test --workspace`).

## CI

- **GitHub Actions** with two workflows:
  - `ci.yml` — runs on every PR and push to `main`. Matrix `{macos-latest, ubuntu-latest} × {stable, MSRV}`. Steps: checkout, `dtolnay/rust-toolchain`, `Swatinem/rust-cache`, fmt check, clippy, build, test.
  - `security.yml` — runs weekly and on PR. Steps: `cargo-audit`, `cargo-deny check`.
- **Renovate** (configured via `renovate.json`) — auto-PR for patch updates, weekly schedule for minor/major.

## Persistence

- **Configuration**: `${XDG_CONFIG_HOME:-~/.config}/tome/config.toml`. TOML, strictly parsed.
- **Catalog cache**: `${XDG_DATA_HOME:-~/.local/share}/tome/catalogs/<sha256-of-source-url>/`. Tool-owned.

## Licensing

- **MIT OR Apache-2.0** dual licence (`LICENSE-MIT` + `LICENSE-APACHE` at repo root).

## Notes

- This is a greenfield project. The above is the *plan*, not an observation. Once `cargo new` runs and the first commit lands, this file should be re-derived from `Cargo.toml`, `Cargo.lock`, and CI configuration.
- The Phase 2 PRD will introduce additional dependencies (vector store, embedding model). Those choices must be justified in writing per the constitution's binary-size and dependency constraints.
