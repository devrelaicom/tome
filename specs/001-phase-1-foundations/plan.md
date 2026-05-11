# Implementation Plan: Phase 1 — Project Foundations and Catalog Management

**Branch**: `001-phase-1-foundations` | **Date**: 2026-05-11 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `specs/001-phase-1-foundations/spec.md`
**Source PRD** (HOW reference): [PRDs/phase-1.md](../../PRDs/phase-1.md)
**Constitution**: [CONSTITUTION.md](../../CONSTITUTION.md) — v1.0.0

## Summary

Phase 1 of Tome delivers two interlocking pieces of work, exposed to users through a single Rust CLI binary:

1. **A contributor-ready Rust project scaffold.** Cargo crate, dual MIT/Apache licence, conventional-commits hook, lefthook-driven local quality gates (fmt, clippy `-D warnings`, typos, tests), and a CI matrix (`{macos-latest, ubuntu-latest} × {stable, MSRV}`) plus weekly security scans (`cargo-audit`, `cargo-deny`).
2. **The catalog management CLI surface.** `tome catalog add | remove | list | update | show`, with strict TOML manifest parsing, atomic on-disk state, scriptable behaviour (`--json`, non-interactive flag equivalents), and signal-safe Git shell-outs.

The technical approach is deliberately boring: synchronous Rust, shell out to system `git`, parse TOML strictly with `serde(deny_unknown_fields)`, map a closed `thiserror` enum to documented exit codes, write registry mutations through `tempfile::persist` for atomicity, and instrument with `tracing` only after credential scrubbing at the process-output boundary. No async runtime, no in-process Git library, no embedded model, no MCP server — those are explicitly deferred to Phase 2 or later.

## Technical Context

**Language/Version**: Rust stable. MSRV pinned in `Cargo.toml` at the current stable release at `cargo init` time and verified in CI. `rust-toolchain.toml` pins channel + `rustfmt` + `clippy`.

**Primary Dependencies**: `clap` (with `derive` feature) for CLI parsing; `serde` + `serde_derive` and `toml` for manifest/config (de)serialisation; `anyhow` for application-level error handling; `thiserror` for the closed error enum that drives exit codes; `tracing` + `tracing-subscriber` for structured logging; `directories` for XDG-aware paths; `sha2` for content-addressing the catalog cache. Additional small-footprint utilities resolved in Phase 0 research: TTY detection, atomic file write, and signal handling.

**Storage**: Filesystem only.
- Configuration: `${XDG_CONFIG_HOME:-~/.config}/tome/config.toml` (TOML, strict-parsed).
- Catalog cache: `${XDG_DATA_HOME:-~/.local/share}/tome/catalogs/<sha256-of-source-url>/` (Tool-owned working copy per catalog).

**Testing**: `cargo test` for both unit and integration. Integration tests in `tests/` exercise the CLI binary against real Git fixtures (a local file-URL "catalog" repository created with `tempfile`). No mocking of Git, the filesystem, or stdio — real `git` binary, real `tempfile::TempDir`, real subprocess invocation. Property-style coverage for the manifest path validator (a small input table with table-driven assertions).

**Target Platform**: macOS arm64 and Linux x86_64. Both must be green on every PR.

**Project Type**: Single. One binary crate `tome` (no workspace splitting yet).

**Performance Goals**: No quantitative target. All Phase 1 operations are interactive; "fast enough for a human waiting" is the bar. Refresh-all of N catalogs is sequential and bounded by upstream network latency, not by Tome.

**Constraints**:
- Release binary < 10 MB stripped (success criterion SC-010, constitution §Operational Constraints).
- Synchronous only — no `tokio`, no `async` (constitution principle VI, §Operational Constraints).
- Closed-and-exhaustive error category set (spec FR-022); no `Other`/`Unknown` arms in the typed error enum.
- Credential scrubbing happens at the boundary of process-output capture, not at the display site (spec FR-024).
- Every persisted state mutation is atomic (spec FR-017a, FR-017b).
- Same `--force` flag name across every command (spec FR-021).

**Scale/Scope**: Single-developer CLI, low-throughput usage. ~10 user-visible commands across 5 subcommands. ~39 functional requirements. Expected catalog count per user: 1–20 in practice. Cache footprint per catalog: bounded by upstream repo size (shallow clones).

## Constitution Check

*GATE: must pass before Phase 0. Re-checked after Phase 1.*

| # | Principle | Status | How this plan satisfies it |
|---|---|---|---|
| I | Unix Philosophy | ✓ | `clap` gives `--help`/`--version`/exit codes for free. Stdout = command output, stderr = errors and diagnostic logs (FR-019, FR-019b). `--json` is a global flag on every output-producing command. `NO_COLOR` is honoured via `anstream`/`anstyle` (resolved in research). Each subcommand has one purpose. |
| II | Predictable Exit Codes (NON-NEGOTIABLE) | ✓ | A single `enum TomeError` in `src/error.rs` enumerates every named failure (closed set per FR-022). `impl ExitCode for TomeError` is the only place codes are mapped. Integration tests assert exit code per category. |
| III | Scriptable by Default | ✓ | `--force` on `remove` (the only Phase 1 prompt). `is_terminal()` check on stdin in every interactive code path; non-TTY triggers a clear error, never a hang. Same flag name everywhere (FR-021). |
| IV | Strict Schemas, Helpful Errors | ✓ | Every TOML-deserialised struct carries `#[serde(deny_unknown_fields)]`. A test asserts every public struct in the manifest and config modules has the attribute (compile-time enforced via a small macro or runtime check on test). |
| V | Fail Fast, Fail Clear | ✓ | `anyhow` `.context()` chaining around every fallible boundary call. Error display formats include "what / where / next" per FR-023. No silent fallbacks. |
| VI | KISS / YAGNI | ✓ | Sync only. No workspace. No async runtime. Dependencies limited to the eight justified in STACK.md plus the small additions resolved in research. No premature trait abstractions. |
| VII | Modular by Boundary | ✓ | Modules organised by capability (`cli`, `commands/catalog`, `catalog/manifest`, `catalog/store`, `catalog/git`, `config`, `paths`, `error`). `thiserror` inside modules; `anyhow` at the application boundary. No circular deps. |
| VIII | Test What Matters | ✓ | Integration test per CLI command using real `git` against a local file-URL catalog. Unit tests for the path validator, the credential scrubber, the SHA detector, and the error→exit-code map. No mocks of the filesystem or Git. |
| IX | Conventional Commits | ✓ | `cocogitto` (`cog`) in lefthook `commit-msg` hook. Phase 2 of this plan installs the hook. |
| X | CI Gates Every Merge | ✓ | `ci.yml` (`{macos-latest, ubuntu-latest} × {stable, MSRV}`) and `security.yml` (`cargo-audit`, `cargo-deny check`) — both scaffolded in Phase 2. |
| XI | Documentation Is Part of the Change | ✓ | `quickstart.md` written alongside the implementation plan. README, CONTRIBUTING, command help-text, and changelog all update in the same PR as the behaviour change. |
| XII | Inherit, Don't Reimplement | ✓ | Shell out to `git` via `std::process::Command`. No `libgit2`. Credential management inherited from the user's existing `git` config. |
| XIII | Never Log Secrets | ✓ | Credential scrubber lives at the boundary in `src/catalog/git.rs`. Every captured stderr/stdout passes through it before reaching tracing, error chains, or display. Unit tests with table-driven cases (HTTPS-with-token, helper output, SSH host-key prompts). |

**Operational Constraints check**:
- Lints (`clippy -D warnings`, `fmt`, `typos`) — enforced in pre-commit and CI.
- Dependencies — the eight from STACK.md plus the additions resolved in research; each within the licence allowlist; `cargo-deny` enforces.
- Async — sync only.
- Binary size — verified in release CI by a `du -sh target/release/tome` assertion.
- Paths — XDG-aware via `directories`.
- Licensing — MIT OR Apache-2.0, both files at repo root.

**Result: PASS.** No complexity violations to track. No deviations from the constitution to justify.

## Project Structure

### Documentation (this feature)

```text
specs/001-phase-1-foundations/
├── plan.md              # This file (/sdd:plan output)
├── spec.md              # Feature specification (/sdd:specify output)
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output — CLI command contracts (not HTTP)
│   ├── catalog-add.md
│   ├── catalog-remove.md
│   ├── catalog-list.md
│   ├── catalog-update.md
│   ├── catalog-show.md
│   └── catalog-manifest.schema.toml
├── checklists/
│   └── requirements.md  # Spec quality checklist (PASS)
└── tasks.md             # Phase 2 output of /sdd:tasks (NOT created here)
```

### Source code (repository root)

```text
tome/                                # repo root
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml
├── lefthook.yml
├── deny.toml
├── renovate.json
├── rustfmt.toml
├── clippy.toml
├── _typos.toml
├── LICENSE-MIT
├── LICENSE-APACHE
├── README.md
├── CONTRIBUTING.md
├── CHANGELOG.md
├── CODE_OF_CONDUCT.md
├── .editorconfig
├── .gitignore
├── .github/
│   └── workflows/
│       ├── ci.yml
│       └── security.yml
├── src/
│   ├── main.rs              # entry point — parses args, dispatches, maps errors to exit codes
│   ├── cli.rs               # clap derive definitions (top-level + global flags: --json, --force, -v/-vv)
│   ├── commands/
│   │   ├── mod.rs
│   │   └── catalog.rs       # tome catalog {add,remove,list,update,show} implementations
│   ├── catalog/
│   │   ├── mod.rs           # public API of the catalog module
│   │   ├── manifest.rs      # tome-catalog.toml schema + strict parsing + path validation
│   │   ├── store.rs         # registry persistence (atomic writes), cache layout
│   │   └── git.rs           # git clone/fetch/reset shell-outs + credential scrubber + signal handling
│   ├── config.rs            # config.toml schema (strict) + load/save
│   ├── paths.rs             # XDG-aware path resolution
│   ├── output.rs            # human vs --json output, NO_COLOR, TTY detection
│   ├── logging.rs           # tracing-subscriber wiring (stderr only; verbosity orthogonal to --json)
│   └── error.rs             # closed `TomeError` enum + ExitCode mapping
├── tests/
│   ├── catalog_add.rs
│   ├── catalog_remove.rs
│   ├── catalog_list.rs
│   ├── catalog_update.rs
│   ├── catalog_show.rs
│   ├── manifest_strictness.rs    # corpus of malformed manifests asserting strict rejection
│   ├── path_validation.rs        # plugins[].source rejection cases
│   ├── exit_codes.rs             # asserts every error category maps to its documented code
│   ├── scrubbing.rs              # credential-bearing stderr does not leak
│   ├── atomicity.rs              # interrupted writes leave registry/cache recoverable
│   └── fixtures/
│       └── sample-catalog/       # a local file-URL catalog used by every test
└── docs/
    └── (anything beyond README that lands in Phase 1)
```

**Structure Decision**: Single binary crate, capability-organised modules. Mirrors the PRD's proposed layout with three small additions resolved in this plan: `output.rs` (cross-cutting human/`--json` formatter to keep individual commands tidy), `logging.rs` (tracing-subscriber wiring kept out of `main.rs` for testability), and `tests/atomicity.rs` + `tests/scrubbing.rs` to satisfy the post-Rust-lens success criteria (SC-006, SC-011, SC-012).

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|---|---|---|
| Single Phase 1 PR exceeds the constitution's ~400-line / 2-module PR-size soft cap (constitution §Development Workflow → PRs) | Phase 1 is the *bootstrap*: it creates the repository structure, the CI matrix, the closed error enum, the manifest parser, and the five catalog subcommands that everything else builds on. Splitting Phase 1 into ≤2-module PRs would either (a) merge half-implementations to `main` (e.g. an `error` module without any caller, a `cli` module that dispatches to `unimplemented!()`) which breaks the constitution's "CI Gates Every Merge" intent of every merge being shippable, or (b) require a long-lived integration branch which contradicts trunk-based development. The phase structure inside this PR (Setup → Foundational → US1 → US2 → US3 → Polish) keeps the diff reviewable in checkpointed slices even though it merges in one drop. **The soft cap re-engages from Phase 2 onward**: every subsequent phase's PR will obey the ~400-line / 2-module rule. | Per-phase PRs to `main`: rejected because the early phases ship no user-visible behaviour and would land partial scaffolding on `main`. Long-lived integration branch: rejected because it inverts trunk-based development. |

## Phase 0 — Research

See [research.md](./research.md) for resolutions of the open questions identified above (TTY detection crate, atomic file write crate, signal handling crate, colour-output strategy, structured-logging configuration, MSRV pinning approach, manifest-strictness compile-time check).

## Phase 1 — Design & Contracts

- **Data model**: [data-model.md](./data-model.md) — Rust types for `CatalogEntry`, `CatalogManifest`, `Owner`, `PluginDeclaration`, `Config`, `TomeError`, and the on-disk shapes (registry TOML, cache layout).
- **Contracts**: [contracts/](./contracts/) — one document per CLI subcommand specifying the contract (args, flags, stdout shape, stderr shape, exit codes, atomicity guarantees), plus the canonical `tome-catalog.toml` schema as a TOML file.
- **Quickstart**: [quickstart.md](./quickstart.md) — clone → `lefthook install` → `cargo build` → run the test suite; documents the 10-minute new-contributor path required by SC-002.
- **Agent context**: `CLAUDE.md` updated with the active tech stack and the most-useful Cargo + project-specific commands.

## Phase 2 — Local Development Environment

Phase 2 of this plan scaffolds the actual Rust project on disk and wires up the tooling described in the spec and constitution. This is **distinct from** `/sdd:tasks`, which will later break the implementation work itself into ordered tasks. Phase 2 here is the tooling foundation that all of those tasks depend on.

Actions:

1. `cargo init --name tome --edition 2024` (or current stable edition) at the repo root.
2. Pin MSRV in `Cargo.toml` to the version of `rustc` available at project start, recorded in research.md.
3. Commit `rust-toolchain.toml` (stable channel + `rustfmt`, `clippy` components).
4. Commit `lefthook.yml` (pre-commit: fmt, clippy `-D warnings`, typos; commit-msg: `cog verify`; pre-push: `cargo test --workspace`).
5. Commit `rustfmt.toml`, `clippy.toml`, `_typos.toml` with project defaults.
6. Commit `deny.toml` (licence allowlist + advisory + source allowlist).
7. Commit `renovate.json`.
8. Commit `.github/workflows/ci.yml` and `.github/workflows/security.yml`.
9. Commit `LICENSE-MIT`, `LICENSE-APACHE`, `README.md`, `CONTRIBUTING.md`, `CHANGELOG.md`, `CODE_OF_CONDUCT.md`, `.editorconfig`, `.gitignore`.
10. Run `cargo fmt`, `cargo clippy -- -D warnings`, and `cargo test` (the test suite is empty at Phase 2 boundary; this validates the toolchain is wired up).
11. Update CLAUDE.md "Active Technologies" and "Recent Changes" sections.
12. Record any deferred issues in a "Tech Debt" section in plan.md (currently expected to be empty).

This phase is the pre-condition for `/sdd:tasks`, which can then assume the toolchain is in place and break the spec's user stories into ordered implementation tasks.

## Phase 2 Scope Note

`/sdd:tasks` will (per the SDD workflow) produce a `tasks.md` that turns the spec's user stories into implementation tasks. That file is **not** created by `/sdd:plan` — this command stops at the end of Phase 2 (local-dev environment setup) per the plan template's outline.
