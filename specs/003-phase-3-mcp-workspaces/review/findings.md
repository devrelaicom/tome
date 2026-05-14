# Phase 3 — Pre-release Review Findings

Four reviewers dispatched in parallel:

- **Contract audit** — compare shipped behaviour against `contracts/*.md`
- **Rust-lens code review** — correctness, ergonomics, idiomaticity
- **Test audit** — coverage gaps + test quality
- **Security audit** — new attack surfaces, trust boundaries, credential / privacy leaks

Counts: **3 blockers, ~25 majors, ~55 minors, ~17 nits.**

Triage is in `disposition.md`.

---

## Blockers (3)

| # | Source | One-line | File / cite |
|---|---|---|---|
| **B1** | log-format | `mcp.log` JSON field names diverge — emits `timestamp`/`message`, contract pins `ts`/`msg`. Every `jq` filter in the contract returns nothing. | `src/mcp/log.rs:120-125` vs `contracts/log-format.md:7-17` |
| **B2** | resolver | §Validation 1b/1c **not enforced AND not tested** — `WorkspaceMalformed` (70) only fires from `commands::workspace::info`; contract says resolver should gate. | `src/workspace/resolution.rs` + `contracts/workspace-resolution.md` §Validation |
| **B3** | doctor / drift | embedder drift → Unhealthy and reranker drift → Degraded — first-class classification reasons — **completely untested**. | `tests/doctor.rs` + `contracts/doctor.md:167-171` |

---

## Major findings (~25)

### MCP (`mcp-server.md` + `log-format.md` + `mcp-tools.md`)

| # | Title | File |
|---|---|---|
| M-MCP-1 | SIGTERM not handled (only SIGINT) | `src/mcp/mod.rs:108-135` |
| M-MCP-2 | 5-second graceful-shutdown timeout missing | `src/mcp/mod.rs:125-133` |
| M-MCP-3 | Stderr fatal-line shape doesn't match contract (one line, category + exit code) | `src/mcp/mod.rs:54-55` |
| M-MCP-4 | Index-file-missing → exit 60 (`McpStartupFailed`); should be 35 per "specific-over-generic" | `src/mcp/preflight.rs:47-54` vs `contracts/exit-codes-p3.md:22-26` |
| M-MCP-5 | `top_k` schema bounds (1..=100) not in generated JSON schema (only at runtime) | `src/mcp/tools/search_skills.rs:35-48` |
| M-MCP-6 | `unknown_plugin` + `unknown_skill` envelope codes UNTESTED | `tests/mcp_server.rs` |
| M-MCP-7 | MCP tool *output* JSON schemas not pinned in any test | `tests/mcp_server.rs` |
| M-MCP-8 | `mcp_lifecycle.rs:19-22` falsely claims drift (41) + integrity (35) are "covered" | `tests/mcp_lifecycle.rs:19-22` |
| M-MCP-9 | `OnceCell::get_or_try_init` for reranker doesn't cache failures — retries every call | `src/mcp/tools/search_skills.rs:127-140` |
| M-MCP-10 | `get_skill` returns unbounded `resources` list (no recursion cap, no count cap) | `src/mcp/tools/get_skill.rs:261-273` |
| M-MCP-11 | Tracing subscriber install fails on second `mcp::run` in-process (`McpStartupFailed`) | `src/mcp/log.rs:115-141` |

### Log format (`log-format.md`)

| # | Title | File |
|---|---|---|
| M-LOG-1 | No credential scrubbing on `workspace_path` / `error_message` fields | `src/mcp/mod.rs:77`, `src/mcp/tools/*.rs` |
| M-LOG-2 | "Hard shutdown" event never emitted | `src/mcp/mod.rs:119-123` |
| M-LOG-3 | `signal` value: only `"SIGINT"`; `"SIGTERM"` and `"stdin_closed"` literals never used | `src/mcp/mod.rs:113,129` |
| M-LOG-4 | Pre-flight check failures never emit a log event | `src/mcp/preflight.rs` |
| M-LOG-5 | `filter` field in `search_skills` log uses `?Debug`-rendering, not structured JSON object | `src/mcp/tools/search_skills.rs:232-235` |
| M-LOG-6 | Log-format **integration test coverage is zero** (schema, taxonomy, scrubbing, taxonomy) | new `tests/mcp_log.rs` (none) |

### Workspace

| # | Title | File |
|---|---|---|
| M-WKS-1 | `TOME_WORKSPACE` accepts a relative path; contract requires absolute | `src/workspace/resolution.rs:64-75` |
| M-WKS-2 | `init --force` not crash-atomic between two renames; orphan `.tome.old/` window | `src/workspace/init.rs:105-130` |
| M-WKS-3 | Registry dedupe is exact-string; case-variant entries desync with `canonicalize` | `src/workspace/inventory.rs:50-72` |
| M-WKS-4 | CLI default-path behaviour for `tome workspace init` (positional omitted → CWD) **UNTESTED** | `tests/workspace_init.rs` |

### Doctor + catalog extensions

| # | Title | File |
|---|---|---|
| M-DOC-1 | **Orphan-clone reporting missing** (contract `catalog-extensions-p3.md:77`; `check_catalogs` walks config only) | `src/doctor/checks.rs:40-59` |
| M-DOC-2 | **Workspace-registry status line missing** from doctor output (contract `catalog-extensions-p3.md:103-113`) | `src/commands/doctor.rs::emit_human` |
| M-DOC-3 | Schema-too-new path through doctor (auto_fixable: false; `--fix` skips) **UNTESTED** | `tests/doctor.rs` |
| M-DOC-4 | `doctor::fixes::apply` signature returns `Result` but never produces `Err`; misleading | `src/doctor/fixes.rs:45-71` |
| M-DOC-5 | `repair_schema` is dead code (no production code path emits `subsystem: "schema"` SuggestedFix) | `src/doctor/fixes.rs:147-166` + `src/doctor/mod.rs:143-185` |

### Schema migration + concurrency

| # | Title | File |
|---|---|---|
| M-MIG-1 | Schema-migration **concurrency / IndexBusy (50) untested** under contention | `tests/schema_migrations.rs` + `tests/concurrency.rs` |
| M-MIG-2 | `concurrent_remove_of_last_reference_is_benign` is **sequential, not concurrent** | `tests/catalog_cache_refcount.rs:292` |

### Security

| # | Title | File | Severity |
|---|---|---|---|
| S-01 | `mcp.log` created with default umask (0644 on `umask 022`); contains workspace paths + error chains | `src/mcp/log.rs:57-62` | major |
| S-02 | `get_skill` walks symlinks verbatim — hostile catalog can plant `skills/foo/creds -> ~/.ssh/id_rsa` | `src/mcp/tools/get_skill.rs:261-273` | major |
| S-03 | Workspace registry entries trusted with only `is_absolute()` — no normalisation, no size cap, no count cap | `src/workspace/inventory.rs:30-40` + `src/catalog/store.rs:72-87` | major |
| S-04 | `init` mis-classifies non-directory markers (regular file / symlink at `.tome`) | `src/workspace/init.rs:69-114` | major |

---

## Minor findings (~55)

### MCP

- m-MCP-1 `verify_embedder_artefacts` hashes the primary file with no size pre-check
- m-MCP-2 `tome_to_mcp` always allocates `e.to_string()` before logging
- m-MCP-3 `Server::tool_router` field has `#[allow(dead_code)]`; prefer `#[doc(hidden)]`
- m-MCP-4 Defensive envelope codes untested (`skill_file_missing`, `frontmatter_strip_failed`, `embedder_drift`, `index_busy`)
- m-MCP-5 `tome mcp --json` silently accepted with no warning/rejection
- m-MCP-6 `in_flight = 0` literal in log events (contract expects live count)
- m-MCP-7 `walk_dir` uses `Path::display().to_string()` (lossy on non-UTF8)
- m-MCP-8 `search_skills.query` is unbounded (semi-trusted host can send MiB strings)

### Workspace

- m-WKS-1 `walk_cwd_for_marker` ascends via `PathBuf::pop()` (textual) before `canonicalize`; symlink chase silent
- m-WKS-2 `validate_workspace_path` returns a synthetic `ScopeSource::Flag` always overwritten by caller
- m-WKS-3 `commands::workspace::init::run` falls back to `PathBuf::default()` on CWD failure
- m-WKS-4 `inventory::append_if_registry_exists` unlocked — two concurrent `init`s race
- m-WKS-5 Unix 0700 mode on `.tome/` set but never asserted in any test
- m-WKS-6 `--inherit-global` JSON `inherited: true` flag not pinned in tests
- m-WKS-7 `.tome.old/` rollback path UNTESTED
- m-WKS-8 "not yet bootstrapped" human render UNTESTED (only JSON pinned)
- m-WKS-9 `cwd_walk` + `env` `ScopeSource` JSON byte-stability not pinned
- m-WKS-10 `workspace info` exit 7 (Io) / 35 (integrity) UNTESTED
- m-WKS-11 Stale registry entry (workspace deleted by hand) silent-failure UNTESTED
- m-WKS-12 `--workspace` / `TOME_WORKSPACE` silently follows symlinks via `canonicalize`

### Doctor

- m-DOC-1 Contract synopsis (`doctor.md:3`) is `[--fix] [--json]` — missing `--verify`
- m-DOC-2 Human "not a git repo" parenthetical detail dropped
- m-DOC-3 Harness paths render via `Path::display()` (absolute), not `~/`-relative
- m-DOC-4 `DriftStatus` JSON serialisation not verified (potential wire-shape divergence)
- m-DOC-5 Glyph TTY vs ASCII fallback UNTESTED
- m-DOC-6 `--global` override "overrode /path via CWD walk" line UNTESTED
- m-DOC-7 `assemble_with_models_and_no_catalogs_reports_ok` doesn't pin `index.present == false`
- m-DOC-8 `commands::doctor::home_dir` duplicates `Paths::resolve`'s `HOME` lookup
- m-DOC-9 Doctor `--json` leaks `$HOME` absolute paths + harness matrix (privacy when shared in bug reports)
- m-DOC-10 `reference_count` silently treats malformed workspace config as "no reference" — can delete legitimately-used clone

### Catalog / paths / error

- m-CAT-1 `catalog add` "manifest missing" → `ManifestInvalid::TomlParse` (semantically wrong variant)
- m-CAT-2 `vec_ext::register_globally` called on every open (trivial cost)
- m-CAT-3 `Paths::config_file` field still reachable; new code could reach for global path under workspace scope
- m-ERR-1 `McpStartupFailed.reason` is an open `String`; contract claims taxonomy-controlled, no enum exists
- m-PATH-1 Relative `XDG_*` env vars silently filtered (typo undetected)
- m-MIG-1 Migration boundary `tracing::info` lines missing `scope` / `path` fields
- m-MIG-2 `SchemaVersionTooNew` Display lacks third "index: …" line from contract example
- m-MIG-3 `apply_pending` no-op case (`current == target`) UNTESTED
- m-MIG-4 Cross-scope migration isolation UNTESTED
- m-MIG-5 `apply_pending` does not acquire the advisory lockfile (callers do)

### Test quality

- m-TEST-1 Several CLI tests use `out.status.success()` instead of `Some(0)` (e.g. `tests/workspace_info.rs:178`)
- m-TEST-2 `status_reports_per_scope_index` uses `unwrap_or(0)` masking field-absent
- m-TEST-3 Human-substring assertions where JSON would be more durable (`tests/doctor.rs:283,301`; `tests/workspace_info.rs:193`)
- m-TEST-4 `descriptions_do_not_enumerate_fixture_identifiers` leakage list hand-maintained
- m-TEST-5 `info_workspace_scope_reads_workspace_paths` doesn't pin which path was read

---

## Nits (~17)

- n-MCP-1 `Server::tool_router` field name same as macro-generated method (confusing on first read)
- n-MCP-2 `mcp::tools::search_skills::Output` derives `Debug` unused at runtime
- n-MCP-3 `FileMakeWriter::make_writer` `.expect("mutex poisoned")` defensible (single-threaded)
- n-MCP-4 `CatalogCacheState::as_str` unused in production
- n-MCP-5 `WorkspaceInfo` derives `PartialEq + Eq` though emit-only
- n-MCP-6 `reference_count` always allocates `Vec<Scope>` even when caller only wants count
- n-MCP-7 Pre-parse `--version` hook scans every arg (future subcommand `--version` flag would trigger early exit)
- n-MCP-8 `QueryArgs::for_mcp(...)` constructor would type-enforce field-by-field MCP build
- n-MCP-9 `human_size` in doctor duplicates promoted `presentation::format::human_mb`
- n-MCP-10 `OpenOptions` doesn't set explicit mode bits (compounds with S-01)
- n-MCP-11 `walk_dir` has no depth bound (impractical exploit; document intent)
- n-MCP-12 Preflight `is_file()` then `open_with_flags` TOCTOU (cosmetic — generic error vs "index_missing")
- n-SEC-1 `init` rollback errors silently swallowed with `let _ = ...`
- n-MIG-1 `MIGRATIONS_OVERRIDE` is `#[doc(hidden)] pub static` — deliberate per CLAUDE.md F7
- n-DOC-1 "Schema version: v1" vs contract "Schema version: 1" (contract inconsistent across its own examples)
- n-DOC-2 Sync_boundary forbidden-needles could false-positive on future doc comments mentioning `tokio::`
- n-TEST-1 `cli_doctor_fix_with_manifest_invalid_exits_75` is the assertion model to copy elsewhere

---

## Cross-cutting observations

### Phase 1/2 disciplines honoured in Phase 3

- Credential scrubbing runs at `git::Git::run` (covers `doctor::fixes::repair_catalog`).
- No new system registry overrides.
- Tome-owned input strictness preserved (`#[serde(deny_unknown_fields)]` on MCP `Input` structs).
- Closed-error-set preserved — 8 new variants documented; no `Other`/`Unknown` arm.
- Sync-only outside `src/mcp/` structurally enforced by `tests/sync_boundary.rs`.
- Atomic writes for `config.toml` (0600) and registry writes (0600 via `write_atomic`).

### Phase 1/2 disciplines NOT applied to Phase 3 surfaces

- **0600 chmod** on `config.toml` is not extended to `mcp.log` (see S-01).
- **Path-traversal validation** at `catalog::manifest::validate_plugin_source` (canonicalise + starts-with) is not applied at the per-skill-file walk in `get_skill` (see S-02).

### Contracts that are FULLY honoured

- `workspace-info.md` — zero semantic findings; all output, JSON wire shape, and exit codes match.

(Every other contract has at least one finding.)
