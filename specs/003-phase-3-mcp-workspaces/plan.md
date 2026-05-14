# Implementation Plan: Phase 3 — MCP Server, Workspaces, and Doctor

**Branch**: `003-phase-3-mcp-workspaces` | **Date**: 2026-05-14 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `specs/003-phase-3-mcp-workspaces/spec.md`
**Source PRD** (HOW reference): [PRDs/phase-3.md](../../PRDs/phase-3.md)
**Constitution**: [CONSTITUTION.md](../../CONSTITUTION.md) — v1.2.0

## Summary

Phase 3 layers three new capabilities on top of the Phase 1 + Phase 2 CLI:

1. **`tome mcp` — a stdio MCP server.** A long-lived child process the harness (Claude Code, Codex, Cursor, Gemini CLI, OpenCode, …) launches once per agent session. It speaks the Model Context Protocol over stdio and exposes two tools: `search_skills` (natural-language search over enabled skills, with optional catalog/plugin filters) and `get_skill` (fetch one skill's body and resource paths by `(catalog, plugin, name)`). The server is backed by the Phase 2 index, eager-loads the embedder at startup, lazy-loads the reranker on the first search call, and writes diagnostic logs to a file because stdout is the protocol channel.
2. **Workspaces.** Any project directory containing a `.tome/` marker is a workspace. The workspace owns its own catalog list and its own index database; shared globally are the on-disk Git clones of catalog URLs (reference-counted by URL hash) and the model artefacts. Workspace resolution is deterministic: `--workspace <path>` flag > `--global` flag > `TOME_WORKSPACE` env > CWD walk > global fallback. Every existing Phase 1 + Phase 2 command resolves a workspace and operates on its state — no per-command opt-in.
3. **`tome doctor`.** A read-only diagnostic command that reports model state, index state, catalog-cache state, workspace context, and which agentic-coding harnesses are detected on the local machine. `--fix` performs the three safe automatic repairs (re-download missing/corrupt models, re-clone broken catalog caches, run pending forward DB migrations). Destructive repairs are never automatic; they are surfaced as suggested commands. `tome status` remains the lock-free narrow pre-flight; `tome doctor` is the broad slower sibling.

Schema-migration plumbing lands in Phase 3 as well. No schema actually changes between Phase 2 and Phase 3 — but every workspace database carries its own `meta.schema_version` row, every DB open runs the forward migrator inside one atomic-write boundary, and a newer-on-disk schema causes the open to refuse rather than corrupt the file. The framework is exercised end-to-end against a synthetic older-version fixture so Phase 4+ DB bumps land on tested rails.

The most consequential design decisions for this plan are:

1. **Async boundary for the MCP server.** The constitution names the MCP server as the expected forcing function for an async runtime ("Sync only until there's a concrete reason otherwise (the MCP server is the expected forcing function)"). `rmcp`, the official Rust MCP SDK named in the PRD, is `tokio`-based. Phase 3 introduces `tokio` strictly inside `src/mcp/`; every other module — including every existing Phase 1/2 command and the new workspace and doctor surfaces — stays synchronous. The MCP server is the only place an `async fn` appears.
2. **Workspace resolution as a global, cross-cutting concern.** Every command must resolve a workspace before deciding which catalog list, which config file, and which index database to open. This is a single Phase 3 change touching the boundary of every Phase 1 + Phase 2 command, executed with the same discipline that produced the closed `TomeError` enum: one resolver, one `Paths`-builder, exhaustive call-sites enforced by the type system.
3. **Shared-on-disk + reference-counted catalog clones.** Catalog cache directories already live at `${XDG_DATA_HOME}/tome/catalogs/<sha256-of-url>/` — they are already URL-hashed and global. What changes in Phase 3 is the *bookkeeping*: a clone's reference set spans the global config and every workspace config. Removing a catalog from a workspace looks up the URL's other references; the on-disk clone is removed only when the last reference goes.
4. **Doctor is built on top of status, not parallel to it.** `tome status` is the Phase 2 lock-free pre-flight. `tome doctor` reuses every status check as a building block, adds workspace context, catalog-cache integrity, and harness detection, formats them through a richer report with suggested-fixes, and offers `--fix`. The two commands' health classifications agree by construction.

## Technical Context

**Language/Version**: Rust stable. MSRV unchanged at `rust-version = "1.93"`. `rmcp`'s and `tokio`'s MSRVs must be verified in research to stay ≤ 1.93; if either is tighter, the plan revises by either bumping MSRV (and CI matrix) or pinning to the last MSRV-compatible version.

**Primary Dependencies** (additions on top of Phase 1 + Phase 2):

- **`rmcp`** — the official Rust MCP SDK (named in the PRD). Provides the protocol implementation, transport adapters (stdio for Phase 3), and tool-registration ergonomics. Likely brings `tokio` and `serde_json` (already a Phase 1 dep).
- **`tokio`** — async runtime, *scoped to `src/mcp/` only*. Features: `rt-multi-thread`, `macros`, `io-std`, `signal`, `sync`. No `tokio` import is permitted outside `src/mcp/`; enforced by a structural test in the same style as `tests/manifest_strictness.rs`.
- **`schemars`** *(possibly)* — JSON Schema generation for MCP tool input schemas. Whether `rmcp` requires it directly or accepts hand-written `serde_json::Value` is a Phase 0 research item.
- *No new presentation, parsing, or storage deps.* `tome workspace`, `tome doctor`, and the workspace-aware refactor of existing commands are pure Rust on existing dependencies (`comfy-table`, `owo-colors`, `inquire`, `rusqlite`, `directories`).

**Storage**:

- **Per-user state** (unchanged): catalog cache at `${XDG_DATA_HOME}/tome/catalogs/<sha256-of-url>/`, models at `${XDG_DATA_HOME}/tome/models/{embedder|reranker}/`, global config at `${XDG_CONFIG_HOME}/tome/config.toml`, global index at `${XDG_DATA_HOME}/tome/index.db`, global advisory lockfile at `${XDG_DATA_HOME}/tome/index.lock`.
- **New** — per-workspace state at `<workspace>/.tome/`:
  - `config.toml` — workspace catalog list, same TOML schema as the global config (strict, `#[serde(deny_unknown_fields)]`).
  - `index.db` — workspace skill index. Same schema as the global DB, same WAL/PRAGMA/lock conventions. Each workspace DB carries its own `meta.schema_version`, `meta.embedder_*`, `meta.reranker_*`.
  - `index.lock` — workspace-scoped advisory lockfile. Workspace writers contend only against other writers on the same workspace; the global lockfile remains separate.
- **New** — MCP server log at `${XDG_STATE_HOME:-~/.local/state}/tome/mcp.log` with size-based rotation (cap ~10 MB, keep the previous one). Standard error reserved for fatal startup-only diagnostics that occur before the file is open. State directory, not data or cache — survives OS cache cleanup but is not user-precious.
- **No new shared on-disk surfaces.** Catalog clones and model artefacts continue to live where Phase 2 put them.

**Testing**: `cargo test`, extending the Phase 1/2 test discipline:

- **New integration suites**:
  - `tests/mcp_server.rs` — drives the MCP server through `rmcp`'s in-process client harness (or a hand-rolled stdio harness if `rmcp` doesn't ship one). Asserts: startup pre-condition failures exit with the dedicated codes; tool list contains exactly two tools; `search_skills` returns ranked hits against a fixture index; `get_skill` returns body + resources for a known triple; `get_skill` returns a structured error for an unknown triple; tool descriptions invite proactive use and do not enumerate plugin or skill names by substring match.
  - `tests/mcp_lifecycle.rs` — exit codes on startup failure (missing DB, schema mismatch, embedder identity mismatch, missing model files, model checksum mismatch). Each failure mode tested by mutating the fixture and asserting the dedicated exit code.
  - `tests/workspace_init.rs` — `tome workspace init` happy path, `--force` overwrite, `--inherit-global` seeds the catalog list but not enablement, refuse-when-exists.
  - `tests/workspace_resolution.rs` — priority order over all five sources (flag, env, CWD walk, global fallback). Includes nested-workspace-wins, malformed-marker-fails-loudly, `--global`-from-inside-workspace.
  - `tests/workspace_info.rs` — `tome workspace info` reports correct workspace identity and resolution method for each priority case.
  - `tests/workspace_commands.rs` — every existing Phase 1 + Phase 2 command, run from inside a workspace without overrides, mutates workspace state and not global. Cross-product: catalog add/remove, plugin enable/disable, reindex, query, status.
  - `tests/catalog_cache_refcount.rs` — adding the same URL in two workspaces produces one on-disk clone; removing from one workspace leaves the clone in place; removing from both workspaces (and global, if referenced) removes the clone.
  - `tests/doctor.rs` — happy report exits 0, every supported failure class triggers the right marker and the right suggested fix, `--fix` actually repairs each of the three safe classes, structured form mirrors human form, harness detection finds well-known directories.
  - `tests/schema_migration_e2e.rs` — synthetic older-version fixture DB is forward-migrated end-to-end; newer-version fixture is refused with the dedicated exit code; migration failure injection rolls back to the pre-migration schema version.
- **Extended Phase 2 suites**:
  - `tests/exit_codes.rs` — eight new variants per FR-201.
  - `tests/scrubbing.rs` — workspace paths and MCP log lines.
  - `tests/atomicity.rs` — interrupt during forward migration leaves the original schema version intact.
- **MCP server end-to-end with a real harness**: out of scope for CI matrix (every harness has its own install requirements). One developer-machine pass against Claude Code and at least one non-Claude-Code harness verifies SC-101; recorded as a manual gate in the retro doc.

**Target Platform**: macOS arm64 and Linux x86_64 — unchanged CI matrix from Phase 2.

**Project Type**: Single binary crate `tome`. Workspace splitting still deferred. Phase 3 adds an estimated 2–3 kLOC of Rust on top of the Phase 2 ~7 kLOC; no friction yet.

**Performance Goals** (from spec SCs):

- MCP server startup to first-message-ready: < 1 s on a recent laptop (SC-102 / NFR-103). Eager-loaded embedder is included in this budget.
- `search_skills` end-to-end with reranker on, ~100 skills indexed: p50 < 300 ms, p99 < 600 ms (SC-103 / NFR-104).
- Workspace resolution per command invocation: dominated by the CWD walk; expected ≤ a handful of `stat(2)` calls. Negligible vs the existing command overhead.
- `tome doctor` happy-path: ≤ ~500 ms in the absence of `--verify` rehashing (no SC, but a Unix-tool expectation).
- `tome doctor --fix` for the three repair classes: bounded by the underlying operation (network for re-download, git for re-clone, transaction for migration). No new ceiling beyond Phase 1/2 expectations.

**Constraints**:

- Release binary stripped: ≤ 50 MB. NFR-101 carries forward Phase 2's revised cap. `rmcp` + `tokio` together must measure under the 20.4 MB current headroom. If they don't, the plan revises (see Open Research Questions).
- **Async scoped to `src/mcp/`**. No `async` keyword, no `.await`, no `tokio::` import outside `src/mcp/`. A structural grep test enforces this at compile time.
- Closed-error-set principle holds. Eight new variants per FR-201; no generic `Other`.
- Atomic state mutations extend to: workspace marker creation (created via `tempfile::TempDir::persist` or equivalent), workspace `config.toml` writes (same `write_atomic` as global), workspace `index.db` writes (same SQLite WAL + advisory lockfile, but per-workspace), MCP log rotation (rename-based).
- Credential scrubbing discipline extends to: workspace paths in MCP logs, catalog URLs in doctor reports, `reqwest` errors during `tome doctor --fix`'s model re-download.
- All Phase 1/2 quality gates apply (NFR-107).

**Scale/Scope**:

- Workspaces per user: typically 1–10 (one per active project); pathological case ≤ ~50.
- MCP server processes: one per agent session; typically 1–3 concurrent on a developer machine.
- Each workspace's enabled-plugin and skill counts: same Phase 2 distribution (1–50 plugins, 5–30 skills per plugin).
- Catalog clones on disk: bounded by the union of all workspace catalog lists plus the global one. Reference-counting prevents duplication.
- MCP log file: capped at ~10 MB by rotation; keep the previous one. Two log files per machine, regardless of how many MCP servers have run.

## Constitution Check

*GATE: must pass before Phase 0. Re-checked after Phase 1.*

| # | Principle | Status | How this plan satisfies it |
|---|---|---|---|
| I | Unix Philosophy | ✓ | Every new CLI surface keeps the convention: human form on stdout (or *no* stdout, for the MCP server), errors on stderr, `--json` global flag. `tome mcp` does one thing (serve MCP over stdio); `tome workspace init` / `tome workspace info` / `tome doctor` each do one thing. The MCP server's stdout-is-protocol invariant is hard-enforced (FR-221, FR-222) — Tome logs land in a file, not on stdout. |
| II | Predictable Exit Codes (NON-NEGOTIABLE) | ✓ | FR-201 enumerates eight new failure classes; each gets its own variant in the closed `TomeError` enum and its own dedicated exit code. Phase 1 + Phase 2 codes are untouched (FR-200). Tests assert exit code per category. The MCP server's startup pre-condition failures each map to their own pre-existing code (model missing, schema mismatch, etc.) rather than a generic "MCP failed"; the new MCP-specific variants cover only what is *new* (stdio I/O failure, MCP startup composite). |
| III | Scriptable by Default | ✓ | Every new interactive surface has a non-interactive equivalent. `tome workspace init` is non-interactive; `tome workspace info` is non-interactive; `tome doctor` is non-interactive; the MCP server is intrinsically non-interactive. `tome doctor --fix` has the same `--force`-vs-prompt discipline as Phase 2 destructive commands: re-downloads and re-clones do not prompt (they're recoverable); the three "destructive" repairs the doctor *won't* perform are surfaced as suggested commands the developer must run explicitly. No silent auto-confirmation. |
| IV | Strict Schemas, Helpful Errors | ✓ | Workspace `config.toml` parses strictly (Tome-owned input, FR-013a). MCP tool input schemas are strict via `serde` deserialisation — unknown parameters in a tool call return a structured error rather than being silently ignored. Workspace-malformed and schema-too-new each have their own dedicated error variant naming the offending file. The "lenient for third-party" boundary continues to apply to `plugin.json` and `SKILL.md` frontmatter inside any workspace; nothing changes there. |
| V | Fail Fast, Fail Clear | ✓ | MCP server startup pre-conditions are checked *before* the server accepts the first message (FR-110); failures exit with a clear log line in both the file log and stderr. Workspace malformed (FR-140) points at `tome doctor`. Schema-too-new (FR-182) points at upgrading Tome. Doctor reports name what failed, where, and what to run (FR-163). No silent fallback, no spinners pretending all is well. |
| VI | KISS / YAGNI | ✓ | One MCP transport (stdio). One workspace marker shape (`.tome/`). One discovery algorithm (priority list). One DB schema, shared across global and every workspace. Migration framework lands with *zero* concrete migrations — only the synthetic-fixture test exercises it. No multi-tenant MCP, no auth, no HTTP transport, no migration tooling between global and workspace, no validation of harness MCP config files. All listed in Out of Scope of the spec and enforced here. |
| VII | Modular by Boundary | ✓ | New modules organised by capability: `src/mcp/` (the async-only island: server, tool handlers, transport, logging-to-file), `src/workspace/` (resolution, init, info, paths-with-workspace-context), `src/commands/mcp.rs`, `src/commands/workspace/{init,info}.rs`, `src/commands/doctor.rs`. Schema migrations consolidate into `src/index/migrations.rs` (already present from Phase 2 plumbing) with a populated, tested apply-pending path. Cross-module access goes through explicit public surfaces; no backdoors. `thiserror` inside modules; `anyhow` at the application boundary. |
| VIII | Test What Matters | ✓ | Integration tests per new CLI command and per MCP tool surface. Workspace resolution exhaustively tested across the priority order. Doctor exhaustively tested per failure class and per fix class. Schema migration tested end-to-end against a synthetic older-version fixture. The Phase 2 `StubEmbedder` continues to keep CI fast; no real-model load in CI. The MCP server's stdio harness uses real I/O over pipes (same "no mocks for the things they hide" rule). Harness detection tests use temp dirs to simulate the well-known per-user directories. |
| IX | Conventional Commits | ✓ | Unchanged. |
| X | CI Gates Every Merge | ✓ | `ci.yml` extends to install nothing new (the `rmcp` + `tokio` stack is pure Rust). Binary-size step asserts ≤ 50 MB. `security.yml` unchanged. Renovate continues. |
| XI | Documentation Is Part of the Change | ✓ | `quickstart.md` updated for Phase 3 commands. README gets a Phase 3 section. CHANGELOG entries. Command help-text for every new subcommand. MCP tool descriptions are themselves user-facing documentation and are versioned in the contract under `contracts/mcp-server.md`. |
| XII | Inherit, Don't Reimplement | ✓ | We use `rmcp`, the official Rust MCP SDK, rather than writing our own protocol implementation. We use `tokio`, the canonical Rust async runtime, rather than rolling a custom executor for one module. Catalog Git clones still shell out to system Git. Schema migrations are an in-process Rust framework — no external migration tool. Harness detection reads directory existence; we never parse a harness's own config files. |
| XIII | Never Log Secrets | ✓ | Phase 1 credential scrubber applies to: every MCP log line (workspace paths and catalog URLs flow through it before reaching the file), every doctor report line (catalog URLs and `reqwest` error chains for `--fix`'s re-download). Unit tests cover signed-URL query-string scrubbing in the workspace and MCP code paths. Workspace paths that contain segments like `~/.aws/credentials` are still just paths — Tome treats them opaquely and never reads their contents. |

**Operational Constraints check**:

- Lints unchanged. New code must pass `clippy -D warnings`.
- **Dependencies** — three new direct (assuming `schemars` is required by `rmcp`'s tool macros; verified in research):

  | Crate | Justification | Licence | Binary impact (estimated) |
  |---|---|---|---|
  | `rmcp` | Official Rust MCP SDK named in the PRD. Provides protocol implementation, tool registration, stdio transport. Required by FR-101. | MIT (verify in research) | ~250 KB (pure Rust) |
  | `tokio` | Async runtime required by `rmcp`. Forcing function explicitly anticipated by the constitution. Scoped to `src/mcp/` only. Required by FR-101. | MIT | ~1.5–2 MB |
  | `schemars` *(possibly)* | JSON Schema generation for MCP tool input schemas. May or may not be required directly depending on `rmcp`'s ergonomics; verified in research. | MIT | ~150 KB if needed |

  Every crate's licence to be confirmed within the constitution's allowlist (MIT / Apache-2.0 / BSD / ISC / Zlib / Unicode-DFS-2016). `cargo-deny check` enforces. Renovate-managed.

- **Async** — *now in play*. The constitution names the MCP server as the expected forcing function ("Sync only until there's a concrete reason otherwise (the MCP server is the expected forcing function)"). Phase 3 introduces `tokio` *only* inside `src/mcp/`. Every existing module, every new workspace module, every new doctor code path stays sync. A structural test (`tests/sync_boundary.rs`) grep-asserts that no `.rs` file outside `src/mcp/` contains `tokio::`, `async fn`, or `.await`. This is the only acceptable shape for the forcing function: a contained island, not a project-wide rewrite.
- **Binary size** — load-bearing concern again. `tokio` is the biggest single addition since `ort`. Current Phase 2 measurement: 29.56 MB on `ubuntu-latest`. Estimated Phase 3 addition: 2–3 MB. Comfortably under the 50 MB cap. CI binary-size step continues to assert ≤ 50 MB. If breached, the plan revises (likely by dropping unused `tokio` features).
- **Paths** — XDG-aware via `directories`. New per-user state directory (`${XDG_STATE_HOME}`) for the MCP log file; new per-workspace `.tome/` for workspace state. Both reuse the same resolver, parameterised over a `Scope::Global | Scope::Workspace(PathBuf)`.
- **Licensing** — MIT OR Apache-2.0 unchanged.

**Result: PASS.** One deviation needs justification in Complexity Tracking: the introduction of `tokio` inside `src/mcp/`. The constitution explicitly anticipates this forcing function, so it is a *documented* deviation from the sync-only default rather than an undocumented one — recorded below for the same reason every other deviation is recorded, so a future reader doesn't lose the rationale.

## Project Structure

### Documentation (this feature)

```text
specs/003-phase-3-mcp-workspaces/
├── plan.md              # This file (/sdd:plan output)
├── spec.md              # Feature specification (/sdd:specify output)
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output — CLI + MCP contracts (no HTTP)
│   ├── mcp-server.md            # tome mcp invocation, startup pre-conditions, lifecycle, log file
│   ├── mcp-tools.md             # search_skills + get_skill schemas, exact descriptions, error shapes
│   ├── workspace-init.md        # tome workspace init
│   ├── workspace-info.md        # tome workspace info
│   ├── workspace-resolution.md  # priority order, error cases, --workspace / --global global flags
│   ├── doctor.md                # tome doctor [--fix] [--json]
│   ├── schema-migration.md      # forward-only contract, atomic boundary, refusal of newer
│   ├── catalog-extensions-p3.md # reference-counted catalog clones across workspaces
│   ├── exit-codes-p3.md         # eight new variants
│   └── log-format.md            # MCP file log line shape; rotation policy
├── checklists/
│   └── requirements.md  # Spec quality checklist (PASS)
└── tasks.md             # Phase 2 output of /sdd:tasks (NOT created here)
```

### Source code (repository root)

New modules in **bold**; Phase 1 + Phase 2 modules left intact where untouched; modified-in-place modules marked as `extended`.

```text
tome/                                # repo root
├── Cargo.toml                       # extended: rmcp, tokio (feature-gated minimal set), optional schemars
├── Cargo.lock
├── build.rs                         # unchanged
├── vendor/sqlite-vec/               # unchanged
├── deny.toml                        # extended: rmcp + tokio + transitives licence rows
├── .github/workflows/
│   ├── ci.yml                       # extended: binary-size step continues to assert ≤ 50 MB; new sync-boundary test runs
│   └── security.yml                 # unchanged
├── src/
│   ├── main.rs                      # extended: dispatch arms for mcp / workspace / doctor; global --workspace / --global flag parsing precedes existing command dispatch
│   ├── lib.rs                       # extended: re-exports
│   ├── cli.rs                       # extended: new subcommands; global --workspace <path> and --global flags carried through to every command
│   ├── config.rs                    # extended: Config now also represents a workspace config (same TOML shape); add Scope::{Global, Workspace(PathBuf)} typed wrapper
│   ├── paths.rs                     # extended: Paths::resolve takes a Scope; new helpers state_dir(), mcp_log_path(), workspace_marker_dir(workspace), workspace_config_path(workspace), workspace_index_path(workspace), workspace_lock_path(workspace)
│   ├── output.rs                    # unchanged
│   ├── logging.rs                   # extended: file-backed appender for MCP mode (rotation, size cap); stderr appender continues for CLI mode
│   ├── error.rs                     # extended: 8 new TomeError variants + ExitCode mapping (FR-201)
│   ├── catalog/                     # Phase 1
│   │   ├── manifest.rs              # unchanged
│   │   ├── store.rs                 # extended: Scope-aware load/save (workspace vs global); reference-counted catalog-clone cleanup uses union of all known scopes
│   │   └── git.rs                   # unchanged (credential scrubbing already covers what doctor needs)
│   ├── commands/
│   │   ├── catalog/                 # Phase 1 — extended to honour Scope
│   │   │   ├── add.rs               # extended: writes workspace config when inside a workspace
│   │   │   ├── remove.rs            # extended: reference-counts the URL across scopes before removing the on-disk clone
│   │   │   ├── list.rs              # extended: lists for the resolved scope
│   │   │   ├── show.rs              # extended
│   │   │   ├── update.rs            # extended
│   │   │   └── source.rs            # unchanged
│   │   ├── plugin/                  # Phase 3–5 — extended to honour Scope (every read/write goes against the resolved DB)
│   │   ├── models/                  # Phase 6 — unchanged (model artefacts are global)
│   │   ├── query.rs                 # Phase 3 — extended: opens the resolved workspace DB
│   │   ├── reindex.rs               # Phase 7 — extended: operates on the resolved scope's enabled plugins
│   │   ├── status.rs                # Phase 8 — extended: reports the resolved scope's DB; --global flag honoured
│   │   ├── workspace/               # NEW
│   │   │   ├── mod.rs               # Dispatcher
│   │   │   ├── init.rs              # `tome workspace init [<path>] [--inherit-global] [--force]`
│   │   │   └── info.rs              # `tome workspace info [--json]`
│   │   ├── doctor.rs                # NEW
│   │   └── mcp.rs                   # NEW — thin CLI wrapper; constructs the server, hands off to src/mcp/
│   ├── workspace/                   # NEW capability module
│   │   ├── mod.rs                   # public surface
│   │   ├── scope.rs                 # Scope::{Global, Workspace(PathBuf)} typed wrapper + Display
│   │   ├── resolution.rs            # priority-ordered workspace resolver; pure function over (flag, env, cwd, fs); deterministic and unit-testable
│   │   ├── init.rs                  # workspace init logic: create marker, write empty config (or inherited), bootstrap empty index DB
│   │   └── inventory.rs             # iterate every workspace known to a global config + every workspace marker reachable on disk; used by reference-counted catalog cleanup
│   ├── plugin/                      # Phase 3+ — unchanged
│   ├── index/                       # Phase 2+ — extended
│   │   ├── mod.rs                   # unchanged public surface
│   │   ├── db.rs                    # extended: takes a path argument from Paths::index_db(scope); WAL/PRAGMA logic unchanged
│   │   ├── schema.rs                # unchanged
│   │   ├── migrations.rs            # extended: populated apply_pending() that actually runs registered migrations under one tx; same forward-only refusal of newer-on-disk; zero registered migrations in Phase 3 (synthetic test fixture exercises the framework)
│   │   ├── vec_ext.rs               # unchanged
│   │   ├── skills.rs                # unchanged
│   │   ├── query.rs                 # unchanged
│   │   ├── meta.rs                  # unchanged
│   │   ├── integrity.rs             # unchanged
│   │   └── lock.rs                  # unchanged
│   ├── embedding/                   # Phase 2 — unchanged
│   ├── presentation/                # Phase 2 — extended
│   │   ├── tables.rs                # extended: doctor report rendering
│   │   ├── progress.rs              # unchanged
│   │   ├── colour.rs                # unchanged
│   │   └── prompt.rs                # unchanged
│   ├── doctor/                      # NEW capability module
│   │   ├── mod.rs                   # public surface
│   │   ├── report.rs                # DoctorReport: collection of per-subsystem findings + suggested fixes + overall classification
│   │   ├── checks.rs                # per-subsystem check functions; each returns a SubsystemFinding
│   │   ├── harness_detect.rs        # well-known per-user directory probes (~/.claude/, ~/.codex/, ~/.cursor/, ~/.gemini/, …)
│   │   └── fixes.rs                 # apply_fix(SubsystemFinding) -> Result<(), TomeError>; only the three safe classes are implemented; everything else returns DoctorFixNotSafe
│   └── mcp/                         # NEW capability module — THE async island
│       ├── mod.rs                   # public surface; pub fn run(scope: Scope, paths: &Paths) -> Result<(), TomeError>; entry point that spins up tokio runtime and serves
│       ├── runtime.rs               # tokio runtime construction (minimum features)
│       ├── server.rs                # rmcp server scaffolding; tool registration; lifecycle (startup pre-conditions, signal handling, graceful shutdown)
│       ├── tools/
│       │   ├── mod.rs
│       │   ├── search_skills.rs     # tool handler: embed query (lazy reranker load), KNN, optional rerank, filter, format result
│       │   └── get_skill.rs         # tool handler: resolve (catalog, plugin, name); read SKILL.md body, strip frontmatter, enumerate sibling files
│       ├── preflight.rs             # startup pre-condition checks: DB present + schema match + embedder identity match + model files present + checksums
│       └── log.rs                   # file-backed tracing layer with rotation
└── tests/
    ├── (all Phase 1 + Phase 2 suites carry forward)
    ├── exit_codes.rs                # extended: 8 new variants
    ├── scrubbing.rs                 # extended: workspace path and MCP log scrubbing
    ├── atomicity.rs                 # extended: interrupt during forward migration
    ├── mcp_server.rs                # NEW — tool list, search_skills, get_skill via rmcp client harness or stdio pipes
    ├── mcp_lifecycle.rs             # NEW — startup pre-condition failure modes per FR-110
    ├── workspace_init.rs            # NEW
    ├── workspace_resolution.rs      # NEW — flag > env > CWD walk > global fallback, nested-workspace wins, malformed-marker errors
    ├── workspace_info.rs            # NEW
    ├── workspace_commands.rs        # NEW — every existing command honours the resolved workspace
    ├── catalog_cache_refcount.rs    # NEW
    ├── doctor.rs                    # NEW
    ├── schema_migration_e2e.rs      # NEW — synthetic older-version fixture; newer-version refusal; rollback on injected failure
    ├── sync_boundary.rs             # NEW — structural grep: no tokio / async / .await outside src/mcp/
    └── fixtures/
        ├── (Phase 1 + Phase 2 fixtures carry forward)
        ├── workspace-fixture/       # NEW — a sample project directory with a .tome/ marker, sample config, sample index seeded for tests
        ├── older-schema.db          # NEW — synthetic SQLite file recording an older schema_version; lets schema_migration_e2e.rs verify the forward-migration path runs end-to-end
        └── newer-schema.db          # NEW — synthetic SQLite file recording a not-yet-supported schema_version
```

**Structure Decision**: Same single binary crate. Phase 3 adds four new capability modules (`workspace`, `doctor`, `mcp`, `commands/workspace`) and *extends* the boundary of every existing command module so it honours `Scope::{Global, Workspace}`. The `Scope` type lives in `src/workspace/scope.rs` and is passed (by reference) into the existing `Paths` builder and the existing config / catalog / lifecycle entry points; no command parses the `--workspace` / `--global` flags itself. Async lives strictly inside `src/mcp/`; a structural test makes the boundary an invariant. No workspace split — still one crate, still ~9–10 kLOC total after Phase 3, well below the "code size justifies the split" threshold.

## Open Research Questions

These are the Phase 0 questions the plan defers to `research.md`:

1. **`rmcp` MSRV and exact API shape.** Does `rmcp` pin to a Rust version > 1.93? Does it require `schemars` for tool input schemas, or does it accept hand-written `serde_json::Value`? Does it ship an in-process client harness for testing, or do we have to drive its server through real pipes?
2. **`tokio` feature set.** Minimum subset needed by `rmcp` over stdio: almost certainly `rt-multi-thread` *or* `rt` (single-threaded is sufficient given Phase 3 serves one request at a time and the embedder/reranker block the thread anyway), `macros`, `io-std`, `signal`. Picking `rt` over `rt-multi-thread` saves binary size if `rmcp` accepts it.
3. **Binary-size measurement.** Build the minimal `rmcp` + `tokio` integration in a scratch branch, measure stripped size on `ubuntu-latest`. If the addition exceeds ~5 MB, revise the `tokio` feature set or pin `rmcp` to a lower-overhead version. Hard floor: total ≤ 50 MB.
4. **MCP tool description wording.** The constraints in FR-108 are normative; the exact text is iterated against real-harness behaviour. Phase 0 collects baseline wording for both tools and the descriptions are versioned in `contracts/mcp-tools.md`.
5. **Workspace marker name.** Spec uses `.tome/` (matches PRD). Phase 0 confirms no clash with existing tools (`.git/`, `.cargo/`, `.vscode/`, etc.; `.tome/` is unique).
6. **`${XDG_STATE_HOME}` resolver.** `directories` (Phase 1 dep) covers config, data, cache. Does it cover state? If not, hand-roll using the same fallback discipline (`~/.local/state` on Unix).
7. **Harness detection list.** Phase 0 confirms the canonical per-user directory for every harness Tome plans to support: Claude Code (`~/.claude/`), Codex (`~/.codex/`), Cursor (`~/.cursor/`), Gemini CLI (`~/.gemini/`), OpenCode (`~/.opencode/`?). Doctor's list is keyed off this set.
8. **Schema migration framework shape.** The current `src/index/migrations.rs` exposes `apply_pending(conn, from_version, to_version)`. Phase 0 confirms its exact signature and registration mechanism so the synthetic-fixture test can drive it without a real migration.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|---|---|---|
| `tokio` introduced inside `src/mcp/` (the constitution's sync-only default) | `rmcp`, the official Rust MCP SDK named in the PRD, is `tokio`-based. The MCP server reads from stdin and writes to stdout while concurrently servicing tool calls — the protocol model is intrinsically async. The constitution explicitly anticipates this forcing function ("Sync only until there's a concrete reason otherwise (the MCP server is the expected forcing function)"). Phase 3 satisfies that condition. | A hand-rolled sync MCP server rejected: reimplementing the protocol is the kind of "inherit, don't reimplement" violation the constitution explicitly forbids. A non-`rmcp` SDK rejected: there isn't a comparable mature alternative. A `block_on`-everywhere wrapper rejected: collapses async semantics and re-introduces every coordination problem the runtime exists to solve. The contained-island approach — `tokio` lives in `src/mcp/`, every other module stays sync — preserves the spirit of the principle: the rest of the codebase remains synchronous, the boundary is a single module, and a structural test (`tests/sync_boundary.rs`) makes the boundary an invariant. |
| Schema-migration framework introduced with zero registered migrations | FR-184 requires the framework be present and exercised in Phase 3 even though no schema actually changes. The cost of writing the framework once, against an empty migration list, is the same as writing it inline at the first migration site — except the framework version means Phase 4 doesn't have to invent it under time pressure. | Defer-until-needed rejected: the first real migration would either land its framework alongside it (doubling the review surface for the first migration PR), or sneak in without one (creating the foreseeable data-loss incident the constitution's `tempfile::persist` pattern exists to prevent). The synthetic-fixture test is what justifies adding the framework now: the code path is real, the test is real, the future migration only adds an entry to the registered list. |

## Plan history

| Date | Event |
|---|---|
| 2026-05-14 | Initial plan written (this commit). |
