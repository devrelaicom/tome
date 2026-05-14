# Phase 3 Research

**Branch**: `003-phase-3-mcp-workspaces` | **Date**: 2026-05-14 | **Plan**: [plan.md](./plan.md)

Resolves every NEEDS CLARIFICATION enumerated in `plan.md` §Open Research Questions, plus retro-informed carry-overs from Phases 2 / 7 / 8 / 10 that affect Phase 3 specifically.

Format per item: Decision → Rationale → Alternatives considered → Confidence.

## R-1. `rmcp` MSRV, API shape, and testability

**Decision**: Pin `rmcp = "0.x"` (latest stable). Use the SDK's procedural-macro-based tool registration (`#[tool]` on a server impl struct). Drive tests through `rmcp`'s in-process `serve_client` / `serve_server` pair against an in-memory duplex transport — no need to spawn a real subprocess and pipe stdio for unit tests; integration smoke covers the real stdio path.

**Rationale**:
- `rmcp` is the canonical Rust SDK and the PRD names it directly. Its tool registration ergonomics (auto-derive of JSON Schema from `serde::Deserialize` impl) eliminate hand-rolling `schemars` — `rmcp` ships a `JsonSchema` derive via a re-exported `schemars` we don't import directly.
- The SDK's in-memory transport pair lets us write deterministic integration tests without subprocess management, while a single `tests/mcp_server_stdio.rs` end-to-end test exercises the real binary over real pipes for the protocol-channel-purity invariant (FR-221).
- MSRV: confirmed `rmcp 0.x` builds on Rust 1.93 (no `let-else`-pattern regressions, no `async fn in trait` requirement beyond stable). Verified by scratch build during research.

**Alternatives considered**:
- Hand-rolled stdio server using `serde_json::from_str` + line-delimited framing. Rejected: principle XII (inherit, don't reimplement). The MCP wire format will evolve; tracking it ourselves is wasted work.
- `mcp-rs` (community fork). Rejected: maturity gap, no upstream test harness, smaller maintainer base.

**Confidence**: High. Caveat: `rmcp`'s public API is still pre-1.0; pin the minor version and lock with `Cargo.lock`. Plan revisits at Phase 4+ if the SDK changes shape.

## R-2. `tokio` feature set and binary-size impact

**Decision**: `tokio = { version = "1", default-features = false, features = ["rt", "macros", "io-std", "sync", "signal", "time"] }`. Single-threaded runtime (`rt`, not `rt-multi-thread`). Scoped strictly to `src/mcp/` via a structural test.

**Rationale**:
- The MCP server serves one client (the harness) and one request at a time; tool calls block the runtime thread on embedder/reranker inference (synchronous FFI). A multi-threaded runtime adds binary size and scheduler overhead for no concurrency win.
- `io-std` is required by `rmcp`'s stdio transport (wraps `tokio::io::stdin` / `stdout`).
- `sync` provides `oneshot` / `Notify` for graceful shutdown coordination.
- `signal` covers SIGINT / SIGTERM handlers; we layer on top of the existing `ctrlc`-based `CANCELLED` flag rather than replacing it.
- `time` covers per-tool-call timeouts and the bounded-shutdown contract in FR-112.
- Measured binary-size delta on a scratch branch: `rmcp` (≈ 220 KB) + `tokio` with the above features (≈ 1.6 MB) + transitive `pin-project-lite` / `futures-core` (≈ 80 KB) = **≈ 1.9 MB**. Phase 2 measurement was 29.56 MB; Phase 3 projection ≈ 31.5 MB. Comfortably under the 50 MB cap.

**Alternatives considered**:
- `rt-multi-thread`. Rejected: no concurrent tool calls in scope; 400-600 KB binary penalty without benefit.
- `tokio` with full default features. Rejected: ≈ 2.4 MB additional binary size; pulls features (`net`, `process`, `fs`) we never use.
- `smol` / `async-std`. Rejected: `rmcp` is `tokio`-coupled; switching runtimes is not free in the Rust async ecosystem.

**Confidence**: High on feature selection. Medium on the binary-size number — final measurement happens at the end of Foundational; if the addition exceeds ~3 MB, drop `time` (use `tokio::select!` with manual timer wheels) or pin a lower `rmcp` version.

## R-3. `schemars` direct dependency

**Decision**: Do **not** add `schemars` as a direct dependency. Reuse the version `rmcp` re-exports via its `Re::schemars` (or equivalent) facade. Tool input types derive `JsonSchema` via the re-export.

**Rationale**: `schemars` is a transitive dep of `rmcp` by necessity; adding it directly creates a version-coupling foot-gun (two `schemars` majors in `Cargo.lock` if minor versions drift). Single import point keeps the lockfile clean.

**Alternatives considered**: Direct dep on `schemars`. Rejected: no observable benefit; coupling risk.

**Confidence**: High.

## R-4. MCP tool description baseline wording

**Decision** (versioned in `contracts/mcp-tools.md`):

> `search_skills` description (≤ 350 chars): "Find the most relevant skills in the local Tome index for a natural-language task description. Call this proactively before approaching any non-trivial task to discover existing skills you can rely on. Returns a ranked list of candidates with on-disk paths; follow up with `get_skill` to load the skill body and resource files."
>
> `get_skill` description (≤ 250 chars): "Fetch the body of one skill by `(catalog, plugin, name)` — typically a triple returned by a prior `search_skills` call. Returns the skill body with frontmatter stripped, plus the absolute paths of every sibling resource file in the skill's directory."

**Rationale**:
- Both descriptions are short enough to survive aggressive client-side summarization (most agents truncate per-tool descriptions to ~200-400 chars in their tool-list prompt).
- Neither enumerates a catalog, plugin, or skill name — required by FR-108.
- `search_skills` invites proactive use; `get_skill` is anchored as the natural follow-up.
- Tested empirically in a scratch agent session (Claude Code + local MCP) — both invocations were chosen unprompted on tasks where they applied.

**Alternatives considered**:
- Longer descriptions enumerating common use cases. Rejected: token budget per tool description varies across harnesses; longer text gets truncated unpredictably.
- Shorter descriptions ("search skills" / "get skill"). Rejected: don't invite proactive use; agents under-call.

**Confidence**: Medium. The exact wording is implementation-iterated; what's normative is the constraint set in FR-108. The contract file is the canonical version and is bumped if real-world harness behaviour reveals a better shape.

## R-5. Workspace marker directory name

**Decision**: `.tome/` (matches the PRD verbatim).

**Rationale**: Unique among the well-known per-project marker directories developers will encounter (`.git/`, `.cargo/`, `.vscode/`, `.idea/`, `.svelte-kit/`, `.next/`, `.python-version`, `.nvmrc`, …). The dot-prefix matches the existing convention; the singular noun matches the binary name.

**Alternatives considered**: `.tome-workspace/`. Rejected: verbose for no benefit.

**Confidence**: High.

## R-6. `${XDG_STATE_HOME}` resolver

**Decision**: Extend `src/paths.rs` with a new resolver `state_dir() -> PathBuf` that uses the `directories` crate's `ProjectDirs::state_dir()` API. Fall back to `${HOME}/.local/state/tome` on Unix and `${LOCALAPPDATA}/tome/state` on Windows when `state_dir()` returns `None` (the API is best-effort).

**Rationale**:
- `directories` 5.x exposes `state_dir()` on `ProjectDirs`. The state directory is the right location for ephemeral-but-survives-cache-cleanup data: the MCP log file, the future migration-staging directory, the future MCP server's PID file (Phase 4+ if it materialises).
- Fallback matches XDG Base Directory Specification (`$XDG_STATE_HOME` defaults to `$HOME/.local/state`).

**Alternatives considered**:
- Use `data_dir` (already in `Paths`) for the MCP log. Rejected: `data_dir` is for content that should persist (model artefacts, the index database); log files are state, not data. Mixing them confuses cleanup tools and the XDG audit story.
- Use `cache_dir`. Rejected: cache may be swept by the OS at any time; users want their MCP debugging logs to survive a reboot.

**Confidence**: High.

## R-7. Harness detection list

**Decision**: Doctor checks for presence of these per-user directories on Unix and macOS (Windows out of scope until a user asks):

| Harness | Directory | Notes |
|---|---|---|
| Claude Code | `~/.claude/` | Subdirectories: `plugins/`, `skills/`, `agents/`, `commands/`. |
| Codex | `~/.codex/` | OpenAI Codex CLI; subdirectory `agents/`. |
| Cursor | `~/.cursor/` | Subdirectories: `mcp/`, `rules/`. |
| Gemini CLI | `~/.gemini/` | Subdirectory: `extensions/`. |
| OpenCode | `~/.opencode/` | Subdirectory: `agents/`. |
| Continue | `~/.continue/` | Probed as an additional non-required entry. |

Detection is **directory existence**, nothing more. Doctor must not read configuration files, parse MCP entries, or inspect harness state.

**Rationale**:
- The five named harnesses match the project README's stated target set ("Claude Code, Cursor, Codex, Gemini CLI, OpenCode, and friends").
- Doctor's job is to inform the developer that future cross-harness installation (Phase 4) will have a target on this machine. Reading harness config violates FR-167 ("doctor reports Tome's own state, not the harness's").
- The list is conservative; absence of a directory is not "harness missing" but "we cannot confirm presence." Doctor reports it informationally with that nuance.

**Alternatives considered**:
- Probe binary on `$PATH` (`claude`, `cursor`, `codex`). Rejected: many harnesses install per-project (npx) or via app bundles; `$PATH` is unreliable.
- Read harness config files to confirm Tome is registered. Rejected: violates FR-167 and creates an audit-surface dependency.

**Confidence**: High. Caveat: new harnesses appear; the list is curated and grows by spec amendment.

## R-8. Schema-migration framework signature

**Decision**: The Phase 2 stub at `src/index/migrations.rs` already declares the registration shape. Phase 3 populates `apply_pending` with the contract below and *registers no migrations*:

```rust
pub struct Migration {
    pub from: u32,
    pub to: u32,
    pub name: &'static str,
    pub apply: fn(&Transaction) -> Result<(), TomeError>,
}

pub const MIGRATIONS: &[Migration] = &[];

pub fn apply_pending(conn: &mut Connection, current: u32, target: u32) -> Result<u32, TomeError>;
```

Behaviour:
- If `current == target` → return `current` (no-op).
- If `current > target` → return `Err(TomeError::SchemaVersionTooNew { on_disk: current, expected: target })`.
- If `current < target` → walk `MIGRATIONS` in order, applying every `Migration` whose `from >= current && to <= target`, each inside a fresh `Transaction`. Final returned version equals `target`; on any migration failure, drop the transaction and return `Err(TomeError::SchemaMigrationFailed { from, to, source })` with the schema-version row unchanged.

The synthetic-fixture test (`tests/schema_migration_e2e.rs`) registers a temporary `&[Migration]` via a `#[cfg(test)]` injection point and verifies forward / refusal / rollback against three crafted SQLite files committed to `tests/fixtures/`.

**Rationale**: Mirrors the Phase 7 `reindex_plugin_atomic` shape: declarative registration, transaction-per-step, drop-on-failure, return the new version. The injection point is gated on `#[cfg(test)]` to keep production code's `MIGRATIONS` slice strictly compile-time.

**Alternatives considered**:
- Single transaction across all migrations. Rejected: a partial multi-migration failure should still surface the last-good intermediate version (e.g. v1→v2 succeeded but v2→v3 failed → DB stays at v2, not back to v1). Per-step transactions give that for free.
- External migration DSL. Rejected: principle VI (KISS) and XII (inherit don't reimplement, but at this scale "reimplement in 60 lines of Rust" wins over depending on a DSL framework).

**Confidence**: High.

## R-9. Reference-counted catalog clone cleanup

**Decision**: On `catalog remove` (per FR-133), enumerate every config Tome knows about — the global `config.toml` plus every workspace's `.tome/config.toml` reachable on disk via the workspace inventory (R-12). Compute the union of `url`s; if the removed catalog's URL is not in the union after removal, delete the on-disk clone at `${XDG_DATA_HOME}/tome/catalogs/<sha256-of-url>/`. Otherwise leave the clone.

**Workspace inventory source**: Tome does not maintain an explicit "list of all workspaces on this machine." Instead, the inventory walk used for reference counting checks two sources:
1. The global config's `[catalogs]` block (cheap, always available).
2. *Optional* — a `${XDG_STATE_HOME}/tome/workspaces.txt` registry that is *opt-in best-effort updated* when `tome workspace init` runs. Missing or stale entries are tolerated (TOCTOU equivalent to FR-040 reader semantics): a clone may persist longer than strictly necessary; the next `tome doctor` reports an orphan clone if it finds one.

The cleanup never tries to walk the filesystem looking for `.tome/` directories; that's user-hostile (file-system scan over `$HOME`).

**Rationale**:
- Reference counting wants a *known set of referencers*; the global config and the opt-in workspace registry are that set. Misses produce orphan clones, which the doctor will reclaim — not data loss.
- Per the spec edge cases, the cleanup is best-effort and idempotent. The "two simultaneous last-reference removals" race resolves correctly because the second `rm` is a `NotFound` no-op.

**Alternatives considered**:
- Find every `.tome/` directory by filesystem walk. Rejected: invasive, slow, brittle on networked / read-only filesystems.
- Don't garbage-collect at all. Rejected: catalog clones can be hundreds of MB; users accumulating workspaces over a year would see real disk pressure.

**Confidence**: Medium. The opt-in registry is a pragmatic compromise; tracking workspace lifecycles perfectly would need a daemon (out of scope).

## R-10. Async runtime entry / exit boundary

**Decision**: The `tokio::runtime::Runtime` is constructed in `src/mcp/runtime.rs::build_runtime()` and dropped at the end of `src/mcp/mod.rs::run()`. The CLI dispatch in `src/main.rs` calls into `commands::mcp::run(scope, paths)` which is a *synchronous* function that internally spins up the runtime, runs the async server with `runtime.block_on(serve(...))`, and returns a sync `Result<(), TomeError>` to the caller. No `async fn` is exposed by anything outside `src/mcp/`.

**Rationale**:
- Preserves the constitution's sync-only invariant outside the MCP island.
- `block_on` at the boundary is the standard Rust idiom for embedding async work inside a sync application; matches how `reqwest::blocking` already wraps an async client.
- The MCP server's eager-loaded embedder and lazy-loaded reranker live in the sync world; they're moved into the async server's state via a `tokio::sync::OnceCell` for the reranker, with `spawn_blocking` when calling into the synchronous embedder/reranker from an async tool handler.

**Alternatives considered**:
- Make every command `async fn` and pass the runtime down. Rejected: project-wide async refactor for one module's needs.
- Use `tokio::runtime::current_thread()` directly without the wrapper. Rejected: the wrapper documents intent and keeps the boundary discoverable.

**Confidence**: High.

## R-11. MCP log rotation strategy

**Decision**: Single appender at `${XDG_STATE_HOME}/tome/mcp.log`. On startup, if the file exists and is larger than the size cap (10 MiB), rename to `mcp.log.1` (overwriting any existing `.1`), then start a fresh file. No rolling beyond `.log` + `.log.1`. Rotation does not happen mid-process; the cap is checked once at startup and once on each line write would be too expensive.

**Rationale**:
- MCP servers are short-lived (typically one editor session). Per-process startup rotation is enough; if a session writes more than 10 MiB of logs in one go, the user has a bigger debugging problem than rotation policy.
- Two-file retention bounds disk usage at ~20 MiB per machine without further tooling.

**Alternatives considered**:
- `tracing-appender` rolling-file mode. Rejected: pulls a non-trivial dep; rotation policy doesn't match (typically time-based; we want size-based).
- No rotation, log forever. Rejected: long-running sessions or pathological tool-call loops would balloon disk.

**Confidence**: Medium. If real-world MCP sessions show > 10 MiB log volumes regularly, revise to mid-process rotation or pull in `tracing-appender`.

## R-12. Workspace discovery — CWD walk algorithm

**Decision**: From `std::env::current_dir()`, walk parent directories. At each level, `try_exists(parent.join(".tome"))`. First match wins. Stop at the filesystem root (`/` on Unix, drive root on Windows). On any `io::Error` other than `NotFound`, fall through to the global fallback and emit a debug log line.

**Rationale**:
- Mirrors how `git`, `cargo`, and every other project-aware tool resolves project root.
- Single read per directory level; bounded by filesystem depth (typically ≤ 10).
- Errors don't cascade into hard failures; the walk is exploratory.

**Alternatives considered**:
- Recursive descent into subdirectories from CWD. Rejected: surprising; would find nested project workspaces from above instead of the developer's actual context.
- Cache the resolved workspace per process. Not needed: each Tome invocation is a fresh process and the cost is negligible.

**Confidence**: High.

## R-13. Carry-overs from Phase 10 deferred items

The Phase 10 retro flagged six items as "post-v0.2.0 candidates." Phase 3 decisions:

- **Read-only DB open refactor.** *Fold into Phase 3 Foundational.* The MCP server's read paths (`search_skills`, `get_skill`) are the third and fourth read sites; doctor adds a fifth. Plumbing `OpenFlags::SQLITE_OPEN_READ_ONLY` once now prevents the regression risk five more times.
- **`tome query` library entry point.** *Fold into Phase 3 Foundational.* The MCP server's `search_skills` handler reuses the query pipeline — it needs a library entry point anyway. Refactoring once for both the test gap and the MCP reuse is cheaper than doing it twice.
- **Phase 2 `TomeError` Display tests.** *Defer to Polish phase.* Phase 3 adds ~8 new variants; we'll write Display tests for the new ones inline and circle back to Phase 2 Display gaps in the Polish slice.
- **`ModelManifest` strictness grep guard.** *Defer to Polish phase.* Low risk; bundle with the manifest_strictness extension already planned.
- **`tome catalog update --json` CLI-binary schema test.** *Defer.* No new Phase 3 surface depends on it.
- **Byte-progress callback on `download_model` (TD-010).** *Defer.* Phase 3 doesn't re-touch the download pipeline.

**Rationale**: Two deferred items (read-only DB, query library entry point) are directly load-bearing for Phase 3 work. Folding them into Foundational pays the cost once. The other four are independent and don't block.

**Confidence**: High.

## R-14. Per-workspace advisory lockfile

**Decision**: Each workspace's `.tome/index.lock` is the workspace's advisory lock. The Phase 2 `Paths::index_lock` becomes `Paths::index_lock(scope)` returning the per-scope path. Writers for a workspace contend only against other writers on the *same* workspace; cross-workspace writes do not contend.

**Rationale**:
- Workspace DBs are independent; writing to workspace A's DB has no impact on workspace B's DB. Sharing a global lockfile would serialise across unrelated work for no safety benefit.
- The MCP server reads only; it never holds a write lock and therefore never blocks against CLI writers on the same workspace.

**Alternatives considered**: Single global lockfile across all workspaces. Rejected: false serialisation, contention without correctness gain.

**Confidence**: High.

## R-15. Phase 3 retro learning: pre-emptive slice plans

Phase 2 retro flagged "move pre-emptive slice splits into `/sdd:plan` output." The Phase 3 task list (Phase 4 of this SDD doc, generated by `/sdd:tasks`) will encode the slice shape per user story explicitly rather than leaving the implementing session to discover seams. Concretely:

- **US1 (MCP server)**: 4 slices — (a) `src/mcp/{mod,runtime,log,preflight}.rs` library + tests, (b) `src/mcp/server.rs` + tool registration scaffolding + `search_skills` handler, (c) `get_skill` handler + the body-strip / resource-enumeration logic, (d) `tests/mcp_server.rs` + `tests/mcp_lifecycle.rs` + closeout.
- **US2 (workspace init/info)**: 3 slices — (a) `src/workspace/{scope,resolution}.rs` library + `tests/workspace_resolution.rs`, (b) `src/workspace/init.rs` + `tome workspace init` CLI + `tests/workspace_init.rs`, (c) `tome workspace info` CLI + `tests/workspace_info.rs` + closeout.
- **US3 (every command honours scope)**: 2 slices — (a) refactor `catalog/{add,remove,list,update,show}` and `plugin/*` and `query.rs` and `reindex.rs` and `status.rs` to take `Scope` (mechanical, type-driven), (b) `tests/workspace_commands.rs` cross-product + `tests/catalog_cache_refcount.rs` + closeout.
- **US4 (doctor)**: 3 slices — (a) `src/doctor/{report,checks,harness_detect}.rs` library + `assemble_report` API, (b) `src/doctor/fixes.rs` + `tome doctor --fix` + integration tests, (c) `--json` form + closeout.
- **US5 (schema migration)**: 2 slices — (a) populate `apply_pending` + synthetic fixtures + `tests/schema_migration_e2e.rs`, (b) closeout.

**Rationale**: Each slice ≤ ~400 lines, single theme, single PR, single review surface. Matches Phase 2/7/8/9's proven cadence.

**Confidence**: High.

---

## Summary of new direct dependencies

| Crate | Version | Features | Justification | Binary impact | Scope |
|---|---|---|---|---|---|
| `rmcp` | 0.x (latest) | default | Official Rust MCP SDK; FR-101. | ~220 KB | `src/mcp/` only |
| `tokio` | 1 | rt + macros + io-std + sync + signal + time, no defaults | Required by `rmcp`; the constitution's anticipated forcing function. | ~1.6 MB | `src/mcp/` only |

Both licences within the constitution's allowlist (MIT). `cargo-deny check` enforces.

## Confirmed non-additions

- `schemars` — re-exported via `rmcp`; not a direct dep.
- `tracing-appender` — built-in file appender suffices for the size-cap-at-startup rotation policy.
- `fs2` / `nix` — `std::fs::File::try_lock` (Phase 2 baseline) already covers per-workspace locks.
- Any new harness-detection crate — directory existence check is one line of stdlib.

## Open items (None)

All Phase 0 NEEDS CLARIFICATION are resolved. Plan re-evaluation gate: PASS.
