# Tome — Claude Code Project Context

This file gives Claude Code persistent context about the Tome project. Keep it terse.

## Project

**Tome** is a Rust CLI (and eventually MCP server) that makes Claude Code's plugin ecosystem work across other agentic coding harnesses (Cursor, Codex, Gemini CLI, OpenCode, …).

- **Current phase:** Phase 1 — project foundations and catalog management.
- **PRD:** [`PRDs/phase-1.md`](./PRDs/phase-1.md)
- **Constitution:** [`CONSTITUTION.md`](./CONSTITUTION.md) (v1.0.0, ratified 2026-05-11)
- **Active spec:** [`specs/001-phase-1-foundations/spec.md`](./specs/001-phase-1-foundations/spec.md)
- **Active plan:** [`specs/001-phase-1-foundations/plan.md`](./specs/001-phase-1-foundations/plan.md)

## Active Technologies

- **Language**: Rust stable (MSRV pinned at `cargo init` time, verified in CI).
- **CLI**: `clap` (derive feature) — provides `--help` / `--version` / global flags.
- **Config / manifest**: `serde` + `serde_derive`, `toml` — every struct uses `#[serde(deny_unknown_fields)]`.
- **Errors**: `thiserror` for the closed `TomeError` enum (drives exit codes); `anyhow` for application-level context chaining.
- **Logging**: `tracing` + `tracing-subscriber` (stderr only; orthogonal to `--json`).
- **Paths**: `directories` (XDG-aware).
- **Hashing**: `sha2` (cache directory naming).
- **Atomic writes**: `tempfile` (registry + per-catalog cache atomicity).
- **Signal handling**: `ctrlc` (SIGINT cancellation; exits with code 8).
- **Colour / NO_COLOR**: `anstream` + `anstyle` (already transitive via clap 4).
- **Regex**: `regex` (credential scrubbing in `src/catalog/git.rs`).

**Not used**: `tokio`, `libgit2`/`git2`, `atty`, `colored`, `lazy_static`, `once_cell` (covered by std `OnceLock`).

## Architectural Constraints (from the constitution)

- **Sync only.** No async runtime in Phase 1.
- **Inherit `git`.** Shell out to system `git` via `std::process::Command`; never vendor a Git library.
- **Closed error set.** `TomeError` has no `Other`/`Unknown` arm. New error categories require editing the spec, PRD, and enum together.
- **Strict TOML.** Every deserialised struct carries `#[serde(deny_unknown_fields)]`; enforced by a test in `tests/manifest_strictness.rs`.
- **Atomic writes.** Registry mutations and cache mutations are atomic; verified by interruption-injection tests.
- **Credential scrubbing at the boundary.** Captured `git` stderr passes through `git::scrub_credentials` before reaching `tracing`, `anyhow::Error`, or any display path.
- **10 MB binary cap.** Dependencies that push the stripped release binary over 10 MB require a written justification.
- **Licence allowlist.** `MIT`, `Apache-2.0`, `MIT-0`, `BSD-{2,3}-Clause`, `ISC`, `Unicode-DFS-2016`, `Zlib`. GPL / AGPL / LGPL banned. Enforced by `cargo-deny`.

## Conventions

- **Commits**: Conventional Commits. Enforced locally by `cocogitto` (`cog verify`) in the lefthook `commit-msg` hook. Format: `type(scope): subject`.
- **Branching**: trunk-based; short-lived branches off `main`.
- **PRs**: small batches — ~400 lines or 2 modules max as a soft cap.
- **Comments**: explain *why*, not *what*. Reader knows Rust.
- **Modules**: capability-organised (`catalog`, `config`, `paths`, `error`, `output`, `logging`).
- **Errors**: `thiserror` inside modules; `anyhow` at the application boundary.

## Common Commands

```sh
# Build / run
cargo build                                      # debug build
cargo build --release                            # release build (used by CI binary-size check)
cargo run -- catalog list                        # run a subcommand from source

# Quality gates (also enforced by lefthook pre-commit)
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
typos

# Tests (lefthook pre-push runs the full suite)
cargo test                                       # all tests
cargo test --test catalog_add                    # one integration test file
cargo test catalog_add::                         # one test by path

# Security and dependency hygiene
cargo audit
cargo deny check

# Conventional Commits
cog verify --file <commit-msg-file>

# Lefthook
lefthook install                                 # one-time, sets up git hooks
lefthook run pre-commit                          # run the pre-commit chain manually
lefthook run pre-push                            # run the pre-push chain manually

# MSRV verification (CI uses dtolnay/rust-toolchain @ rust-version from Cargo.toml)
cargo +<MSRV> build
```

## File Structure (planned)

```
src/
├── main.rs              # entry: parse → dispatch → map errors → exit
├── cli.rs               # clap derive defs (global --json, --force, -v/-vv)
├── commands/catalog.rs  # tome catalog {add,remove,list,update,show}
├── catalog/
│   ├── manifest.rs      # tome-catalog.toml schema + parsing + path validation
│   ├── store.rs         # registry persistence (atomic writes), cache layout
│   └── git.rs           # git shell-outs + credential scrubber + signal handling
├── config.rs            # config.toml schema + load/save
├── paths.rs             # XDG-aware paths
├── output.rs            # human/--json formatter, NO_COLOR, TTY detection
├── logging.rs           # tracing-subscriber wiring (stderr-only)
└── error.rs             # closed TomeError enum + ExitCode mapping

tests/
├── catalog_*.rs         # one per subcommand
├── manifest_strictness.rs
├── path_validation.rs
├── exit_codes.rs
├── scrubbing.rs
├── atomicity.rs
└── fixtures/sample-catalog/
```

## Recent Changes

- 2026-05-11: Ratified CONSTITUTION.md v1.0.0.
- 2026-05-11: Wrote Phase 1 PRD amendments resolving the constitution-review report (added exit code 7 for IO; tightened strict parsing rules; credential scrubbing required; cache ownership and SHA-pin behaviour documented).
- 2026-05-11: Generated `/sdd:specify` artefacts on branch `001-phase-1-foundations` — spec, requirements checklist (PASS), STACK.md.
- 2026-05-11: Generated `/sdd:plan` artefacts — plan.md, research.md, data-model.md, contracts/*, quickstart.md. Constitution gates: PASS, zero violations to justify.
- 2026-05-11: Added exit code 8 (SIGINT interrupted) after Rust-lens review of the spec; spec FRs and SCs amended for atomicity, signal handling, UTF-8 output, log/--json orthogonality, --help/--version, and the closed-and-exhaustive error set.

<!-- MANUAL ADDITIONS START -->
<!-- Notes that should not be touched by automation go here. -->
<!-- MANUAL ADDITIONS END -->
