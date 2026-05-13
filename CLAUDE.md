# Tome — Claude Code Project Context

This file gives Claude Code persistent context about the Tome project. Keep it terse.

## Project

**Tome** is a Rust CLI (and eventually MCP server) that makes Claude Code's plugin ecosystem work across other agentic coding harnesses (Cursor, Codex, Gemini CLI, OpenCode, …).

- **Current phase:** Phase 2, User Story 1 — closing. Implementation + integration tests shipped (PRs #11–#14). Closeout in PR #15.
- **Phase 1 PRD (shipped):** [`PRDs/phase-1.md`](./PRDs/phase-1.md)
- **Phase 2 PRD (in progress):** [`PRDs/phase-2.md`](./PRDs/phase-2.md)
- **Constitution:** [`CONSTITUTION.md`](./CONSTITUTION.md) (v1.2.0 — binary size cap revised 10 MB → 50 MB on 2026-05-13)
- **Active spec:** [`specs/002-phase-2-plugins-index/spec.md`](./specs/002-phase-2-plugins-index/spec.md)
- **Active plan:** [`specs/002-phase-2-plugins-index/plan.md`](./specs/002-phase-2-plugins-index/plan.md)
- **Codebase docs:** [`.sdd/codebase/`](./.sdd/codebase/) — all 8 documents refreshed 2026-05-13 against US1-complete source via `/sdd:map incremental`.
- **US1 status:** `tome plugin enable | list | show` + `tome query` ship end-to-end. 187 tests pass across 25 suites. Manual SC-001 / SC-002 against real BGE models still pending — see `retro/P3.md` § "T088 manual verification".

## Active Technologies

### Phase 1 (shipped, unchanged)

- **Language**: Rust stable, MSRV pinned at `rust-version = "1.93"` (verified in CI).
- **CLI**: `clap` (derive feature) — `--help` / `--version` / global flags.
- **Config / manifest**: `serde` + `serde_derive`, `toml` — Tome-owned structs use `#[serde(deny_unknown_fields)]`.
- **Errors**: `thiserror` for the closed `TomeError` enum (drives exit codes); `anyhow` for application-level context chaining.
- **Logging**: `tracing` + `tracing-subscriber` (stderr only; orthogonal to `--json`).
- **Paths**: `directories` (XDG-aware).
- **Hashing**: `sha2`, `hex` (cache directory naming, model checksums, content hashes).
- **Atomic writes**: `tempfile` (registry, catalog cache, models dir atomicity).
- **Signal handling**: `ctrlc` (SIGINT cancellation; exits with code 8).
- **Colour / NO_COLOR**: `anstream` + `anstyle` (transitive via clap 4).
- **Regex**: `regex` (credential scrubbing in `src/catalog/git.rs`).
- **Time**: `time` (RFC 3339 timestamps).
- **Semver**: `semver`.

### Phase 2 additions

- **Embedded database**: `rusqlite` with the `bundled` feature — statically linked SQLite, no system dep.
- **Vector search**: `sqlite-vec` C extension vendored under `vendor/sqlite-vec/`, compiled in via `build.rs`.
- **Inference**: `fastembed-rs` wrapping `ort` (ONNX Runtime). CPU execution provider only; CUDA / CoreML / DirectML disabled.
- **Models** (downloaded at runtime; not bundled): `bge-small-en-v1.5` INT8 (~45 MB, MIT), `bge-reranker-base` INT8 (~280 MB, MIT). Stored under `${XDG_DATA_HOME}/tome/models/`.
- **Progress / spinners**: `indicatif`.
- **Tables**: `comfy-table`.
- **Colours**: `owo-colors` (native `NO_COLOR`).
- **Prompts**: `inquire` (Select / MultiSelect / Confirm; refuses on non-TTY).
- **HTTP**: `reqwest` with `blocking` + `rustls-tls`, `default-features = false`.

**Strictness boundary** (FR-013a): `#[serde(deny_unknown_fields)]` applies to Tome-owned inputs (config, model `manifest.json`, index `meta` rows). Third-party inputs (`plugin.json`, `SKILL.md` YAML frontmatter) parse leniently — forward-compat with upstream additions.

**Not used**: `tokio`, `libgit2`/`git2`, `atty`, `colored`, `lazy_static`, `once_cell` (std `OnceLock` covers it). Phase 2 stays synchronous.

## Architectural Constraints (from the constitution)

- **Sync only.** No async runtime. `reqwest::blocking`, `rusqlite`, and `fastembed-rs` are all sync. The MCP server is the future forcing function for async.
- **Inherit `git`.** Shell out to system `git`. Never vendor a Git library.
- **Closed error set.** `TomeError` has no `Other`/`Unknown` arm. Every Phase 2 failure class has its own enumerated variant and exit code (see `specs/002-phase-2-plugins-index/contracts/exit-codes.md`).
- **Strictness boundary.** Tome-owned declarative inputs are strict (`#[serde(deny_unknown_fields)]`); third-party inputs (`plugin.json`, SKILL.md frontmatter) are lenient — see spec FR-013a. The strict-on-Tome-owned principle is enforced by `tests/manifest_strictness.rs`.
- **Atomic writes.** Registry, cache, models directory, and index DB mutations are atomic. SQLite WAL + a Tome-owned advisory lockfile (`index.lock`) provide the index concurrency contract (FR-040).
- **Credential scrubbing at the boundary.** `git::scrub_credentials` extends to model download URLs and `reqwest` error chains.
- **50 MB binary cap** (revised from 10 MB on 2026-05-13 — see `retro/P3.md` §"Binary-size cap revision"). CI asserts `stat -c%s target/release/tome` on Linux. `ort` (CPU-only static) is the load-bearing dep; profile is `lto = "thin"`, `panic = "abort"`, `strip = "symbols"`. If breached, the plan revises — the discipline is non-waivable (NFR-001); the specific number is sized to current reality plus headroom.
- **Licence allowlist.** Unchanged. Every Phase 2 dep verified inside the allowlist. `cargo-deny` enforces. Downloaded models (BGE family, MIT) are surfaced in `tome models list`.

## Conventions

- **Commits**: Conventional Commits. Enforced locally by `cocogitto` (`cog verify`) in the `commit-msg` hook (versioned under `.githooks/`). Format: `type(scope): subject`.
- **Branching**: trunk-based; short-lived branches off `main`.
- **PRs**: small batches — ~400 lines or 2 modules max as a soft cap.
- **Comments**: explain *why*, not *what*. Reader knows Rust.
- **Modules**: capability-organised. Phase 1: `catalog`, `config`, `paths`, `error`, `output`, `logging`. Phase 2 adds: `plugin` (manifest/frontmatter/lifecycle), `index` (db/schema/migrations/vec-ext/skills/query/meta/integrity/lock), `embedding` (fastembed wrapper + stub + registry + download + runtime), `presentation` (wraps comfy-table / indicatif / owo-colors / inquire).
- **Errors**: `thiserror` inside modules; `anyhow` at the application boundary.

## Common Commands

```sh
# Build / run
cargo build                                      # debug build
cargo build --release                            # release build (used by CI binary-size check)
cargo run -- catalog list                        # run a subcommand from source

# Quality gates (also enforced by the .githooks/pre-commit hook)
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
typos

# Tests (.githooks/pre-push runs the full suite)
cargo test                                       # all tests (uses stub embedder — fast, no model files)
cargo test --test catalog_add                    # one integration test file
cargo test catalog_add::                         # one test by path
cargo test --test query                          # Phase 2 query tests
cargo test --test concurrency                    # two-process index contention
cargo test --test atomicity                      # interrupt-injection tests

# Security and dependency hygiene
cargo audit
cargo deny check

# Conventional Commits
cog verify --file <commit-msg-file>

# Git hooks (versioned under .githooks/; no external manager)
git config core.hooksPath .githooks              # one-time, per clone
.githooks/pre-commit                              # run the pre-commit chain manually
.githooks/pre-push < /dev/null                    # run pre-push manually (drain empty stdin)

# MSRV verification (CI uses dtolnay/rust-toolchain @ rust-version from Cargo.toml)
cargo +<MSRV> build
```

## File Structure

```
src/
├── main.rs                   # entry: parse → dispatch → map errors → exit
├── lib.rs                    # re-exports
├── cli.rs                    # clap derive defs + global flags
├── error.rs                  # closed TomeError enum + ExitCode mapping
├── config.rs                 # config.toml (strict)
├── paths.rs                  # XDG paths (Phase 1) + index_db, models_dir, index_lock (Phase 2)
├── output.rs                 # human/--json formatter, NO_COLOR, TTY detection
├── logging.rs                # tracing-subscriber wiring
├── catalog/                  # Phase 1
│   ├── manifest.rs           # tome-catalog.toml (strict)
│   ├── store.rs              # registry persistence (atomic) — Phase 2 hooks cascade
│   └── git.rs                # git shell-outs + scrub_credentials
├── commands/
│   ├── catalog.rs            # tome catalog {add,remove,list,update,show}
│   ├── plugin.rs             # NEW — enable/disable/list/show/interactive
│   ├── query.rs              # NEW
│   ├── models.rs             # NEW — download/list/remove
│   ├── reindex.rs            # NEW
│   └── status.rs             # NEW
├── plugin/                   # NEW
│   ├── manifest.rs           # plugin.json (lenient)
│   ├── frontmatter.rs        # SKILL.md frontmatter (lenient + fallbacks)
│   ├── components.rs         # skills/agents/commands/hooks/.mcp.json walks
│   ├── identity.rs           # <catalog>/<plugin> address parsing
│   └── lifecycle.rs          # enable/disable orchestrator (atomic per plugin)
├── index/                    # NEW
│   ├── db.rs                 # rusqlite open, WAL, busy_timeout
│   ├── schema.rs             # CREATE TABLE statements (mirror of contracts/index-schema.sql)
│   ├── migrations.rs         # forward-only migrations under advisory lock
│   ├── vec_ext.rs            # sqlite-vec extension load (build.rs compiled)
│   ├── skills.rs             # CRUD on skills table; content-hash diff
│   ├── query.rs              # KNN search + reranker invocation
│   ├── meta.rs               # drift detection
│   ├── integrity.rs          # PRAGMA integrity_check
│   └── lock.rs               # advisory lockfile
├── embedding/                # NEW
│   ├── mod.rs                # Embedder + Reranker traits
│   ├── fastembed.rs          # fastembed-rs impl
│   ├── stub.rs               # #[cfg(test)] deterministic stub
│   ├── registry.rs           # MODEL_REGISTRY (pinned URLs + checksums)
│   ├── download.rs           # reqwest::blocking + SHA-256 + atomic persist
│   └── runtime.rs            # ort Environment, CPU EP only
└── presentation/             # NEW
    ├── tables.rs             # comfy-table helpers
    ├── progress.rs           # indicatif wrappers (TTY-aware)
    ├── colour.rs             # owo-colors + NO_COLOR
    └── prompt.rs             # inquire wrappers (refuse on non-TTY)

vendor/
└── sqlite-vec/               # NEW — pinned C source + LICENSE; compiled via build.rs

tests/
├── catalog_*.rs              # Phase 1 (catalog_remove extended for cascade)
├── manifest_strictness.rs    # Phase 1 (extended: model manifest is strict)
├── path_validation.rs        # Phase 1
├── exit_codes.rs             # extended for the 18 Phase 2 codes
├── scrubbing.rs              # extended for model URL scrubbing
├── atomicity.rs              # extended for enable / model download interrupts
├── plugin_{enable,disable,list,show,interactive}.rs   # NEW
├── query.rs                  # NEW
├── models_{download,list,remove}.rs                   # NEW
├── reindex.rs                # NEW
├── status.rs                 # NEW
├── catalog_update_reindex.rs # NEW
├── catalog_remove_cascade.rs # NEW
├── concurrency.rs            # NEW — two-process index contention
├── schema_migrations.rs      # NEW
├── version_output.rs         # NEW
├── frontmatter.rs            # NEW — table-driven parser cases
└── fixtures/
    ├── sample-catalog/       # Phase 1
    └── sample-plugin/        # NEW
```

## Recent Changes

- 2026-05-13: Closed Phase 3 / User Story 1 across PRs #11–#15. Slice 1a (`plugin::lifecycle::enable` / `disable` orchestrator + pinned `MODEL_REGISTRY` SHA-256s; reranker URL moved upstream from `BAAI/bge-reranker-base` to `onnx-community/bge-reranker-base-ONNX`), slice 1b (`tome plugin enable | list | show` CLI + T074 prompt UI), slice 2 (`tome query` with reranker + `--strict`), slice 3 (5 new integration-test files + `tests/fixtures/sample-plugin-catalog/` + `StubEmbedder::with_force_fail_after`), and the resolver-bug fix folded into PR #14 (`lifecycle::resolve_plugin_dir` is now manifest-first via `tome-catalog.toml`; falls back to flat join only when the manifest is absent/unparsable). Constitution v1.2.0 — binary size cap revised 10 MB → 50 MB after slice 1b measured 29.56 MB on Linux (research §Binary size budget's ~9.2 MB worst-case projection underestimated `ort`). Test total 156 → 187 across 25 suites. T088 manual SC-001 / SC-002 verification against real BGE models is the only outstanding US1 task and lives in `retro/P3.md` for a developer pass.
- 2026-05-12: Closed Phase 2 foundational — landed slices 1–7 across PRs #2–#10. T057 (model-download integration test with hand-rolled `TcpListener` HTTP fixture) is in slice 7 rather than slice 5 where it was originally scheduled. The cleanup bug it caught (partial-dir leaking on checksum mismatch because cleanup only ran on `stream_to_partial` errors, not later pipeline errors) is fixed by wrapping the full post-stream pipeline in a closure. Codebase docs (`.sdd/codebase/STACK.md`, `STRUCTURE.md`) refreshed; retro at `specs/002-phase-2-plugins-index/retro/P2.md` extended with workarounds, package gotchas, patterns, and "for next time" entries.
- 2026-05-12: Generated Phase 2 `/sdd:plan` artefacts on `002-phase-2-plugins-index` — plan.md, research.md (15 R-decisions including binary-size strategy, SQLite concurrency model, schema migration, frontmatter strictness boundary), data-model.md, contracts/* (plugin-commands, query, models-commands, reindex, status, catalog-extensions, version-output, exit-codes, index-schema.sql), quickstart.md. Constitution gates: PASS with one justified deviation (`#[cfg(test)]` stub for the embedder/reranker traits — keeps CI fast and bounded; principle VIII boundary case).
- 2026-05-11: Generated Phase 2 `/sdd:specify` artefacts — spec (7 user stories, 60 FRs, 5 NFRs, 15 SCs) and refreshed `.sdd/codebase/*` against the Phase 1 source. Rust-lens review folded in 3 blockers + 12 majors before validation passed.
- 2026-05-11: Ratified CONSTITUTION.md v1.0.0; later patched to v1.0.1.
- 2026-05-11: Wrote Phase 1 PRD amendments resolving the constitution-review report.
- 2026-05-11: Generated `/sdd:specify` and `/sdd:plan` artefacts on `001-phase-1-foundations`. Constitution gates: PASS, zero violations.
- 2026-05-11: Added exit code 8 (SIGINT interrupted) after Rust-lens review of the Phase 1 spec.

<!-- MANUAL ADDITIONS START -->
<!-- Notes that should not be touched by automation go here. -->
<!-- MANUAL ADDITIONS END -->
