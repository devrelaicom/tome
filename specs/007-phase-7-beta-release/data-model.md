# Phase 7 Data Model — Beta Hardening and Public Release

**Branch**: `007-phase-7-beta-release` | **Date**: 2026-06-01
**Input**: [spec.md](./spec.md), [plan.md](./plan.md), [research.md](./research.md)

Phase 7 is a hardening + release phase: it changes **behaviour and structure**, not the data schema. Types are described at the data-model level (names, fields, invariants); exact Rust signatures land in the contracts and the implementation.

**The headline data-model facts:**
- **No new SQLite column or table; no migration; the schema version is unchanged.**
- **No new `TomeError` variant; no new exit code** — every new failure path reuses an existing variant (§8).
- The only *new* internal types are test-side (the in-process MCP harness, §6) and a single production helper (the symlink-safe open primitive, §4). The only *new* persisted artifacts are repo-root files (release config, licence bundle, security policy — §7).

---

## 1. Search over-fetch + widen (FR-001) — behavioural, no schema change

### `knn` query shape (`src/index/query.rs`, existing — fixed)

The function signature is unchanged (`knn(conn, workspace_name, query_vec, top_k, filters) -> Result<Vec<Candidate>, TomeError>`). The **internal control flow** changes from a single vec0 query with `k = top_k` to a bounded over-fetch + widen loop:

| Concept | Behaviour |
|---|---|
| `effective_k` | the vec0 `MATCH … AND k = ?` bound; starts at a multiple of `top_k`, grows geometrically per widen iteration |
| widen invariant | re-query with a larger `effective_k` while `post_filter_matches < min(top_k, total_matching_entries)` and `effective_k < table_row_count` |
| termination | stop when `top_k` post-filter matches are collected **or** the candidate set is exhausted (`effective_k` reaches the row count) — never an error, never global-neighbourhood leakage (spec Edge Case: widen ceiling returns the true smaller set) |
| result | exactly `min(top_k, total matching entries)`, ordered by distance — the matching entries are present (0 → present) and the count does not shrink as the corpus grows |

- Filters (`workspace`, `searchable = 1`, `--catalog`, `--plugin`) are the existing post-`JOIN`/`WHERE` predicates — unchanged; only the candidate-fetch width changes so the filters operate on a wide-enough pool.
- Callers `commands/query.rs` (incl. `--strict`) and `mcp/tools/search_skills.rs` consume the corrected `knn`; no caller-side filter logic moves.
- **No `Candidate` field change**, no embedding change, no reindex.

---

## 2. Read-only doctor (FR-002) — open-mode change, no schema change

| Concept | Behaviour |
|---|---|
| read-only path | `doctor` (no `--fix`) opens the index via `index::open_read_only` — **no `apply_pending` migration, no WAL pragma write, no advisory lock** |
| degradation | a stale **or** future schema → a *degraded* report for that subsystem (mirrors the existing `check_index` swallow), never an exit-73 abort of the whole run |
| `--fix` path | unchanged — performs the lock-held migration via the existing `repair_schema` |

No new types; the `DoctorReport` envelope is unchanged (a possibly-degraded subsystem block rather than an abort). Existing JSON wire-pins stay byte-stable.

---

## 3. Catalog cache key (FR-003) — keying change, no schema change

| Reader/writer | Keys by (today) | Keys by (fixed) |
|---|---|---|
| `cache_dir_for(url)` (writer, `catalog/add.rs`) | **raw** url | **scrubbed** url |
| `refcount_by_url(conn, url)` (writer reuse check) | **raw** url | **scrubbed** url |
| `git.clone_shallow(url)` (auth) | raw url | **raw** url (unchanged — credential lives here) |
| `show`/`update`/`remove`/`list`/doctor/sync (readers) | **scrubbed** url | **scrubbed** url (already correct) |

- Invariant after the fix: **the writer keys by the same string every reader resolves by** (the scrubbed URL). Plain `https://host/owner/repo` (raw == scrubbed) is unaffected. SSH/tokenised sources round-trip without orphaning a clone.
- No DB column change — `workspace_catalogs.url` already persists the scrubbed URL; the fix aligns the cache-dir + refcount keying to it.

---

## 4. Symlink-safe write primitive (FR-007) — one new SSOT helper

### `symlink_safe` open (`src/util/`, NEW — the single source of truth)

A single rustix-backed helper that opens a write target **refusing to traverse a symlinked intermediate directory component**, replacing the several duplicated `refuse_symlink` final-node-only checks.

| Concept | Behaviour |
|---|---|
| primary path (spike confirms) | `openat2` with `ResolveFlags::NO_SYMLINKS` (Linux); a portable per-component `openat` + `OFlags::NOFOLLOW` walk (macOS) |
| fallback path (spike fails) | final-node `O_NOFOLLOW` + the documented trust-model mitigation (NFR-004 holds either way; no new package) |
| error | the **dedicated write-guard error of the calling sink** — settings sink → `HookSettingsWriteFailed` (44); guardrails → `GuardrailsWriteFailed` (46); agents → `AgentTranslationFailed`/its existing code; rules/mcp → their existing codes. **Never** a regression to generic `Io` (7) on a dedicated sink (the Phase-6 CON-1 precedent) |
| consumers (ALL routed in ONE pass) | `harness/hooks.rs` (settings.local.json), `harness/guardrails.rs` (in-file regions + Cursor sibling), `harness/agents.rs` (agent files), `harness/rules_file.rs`, `harness/mcp_config.rs`, `util/atomic_dir.rs` |

- **Single SSOT**: the existing duplicated `refuse_symlink` copies (`util/atomic_dir.rs:249`, `harness/mcp_config.rs:92`, the `catalog/store.rs` final-node check) delegate to this primitive — closing the "fix one sink, miss its parallel" hazard structurally (the bug class the project has shipped twice).
- The final-node refusal continues to hold; this *adds* intermediate-component protection.

---

## 5. Reconciler decomposition (FR-011) — module relocation, behaviour-preserving

No data-type changes — a **file move** of already-factored functions. The shapes below move verbatim; the orchestrator's assembly of them is unchanged.

| Stays in `harness/sync.rs` (thin orchestrator) | Moves to `harness/reconcile/hooks.rs` | Moves to `harness/reconcile/guardrails.rs` | Moves to `harness/reconcile/agents.rs` |
|---|---|---|---|
| `sync_project`, `SyncDeps`, `SyncOutcome`, `SyncChange`, `SyncSubsystem`, `HarnessDecision`, `Action` | `HooksReconciliation`, `reconcile_hooks`, `compute_plugins_with_hooks_json`, `merge_hooks_for_harness`, `remove_hooks_for_harness` | `GuardrailsReconciliation`, `PreparedGuardrails`, `reconcile_guardrails`, `guardrails_target_path`, `guardrails_action_to_action` | `AgentReconciliation`, `PreparedAgent`, `reconcile_agents`, `prepare_agent`, `emit_agents_for_harness`, `cleanup_all_owned_agents`, `removed_disabled_owned`, `all_owned_in_dir`, `AgentWrite`, `write_agent_file`, `record_action` |
| shared: `HarnessSnapshot`, `collect_harness_snapshots`, `snapshot_for`, `group_by_path`, `read_workspace_settings`, `read_global_settings`, `relative_path`; rules/mcp helpers (`compute_rules_body`, `write/clean_rules_for_path`, `classify_block`, `write/clean_mcp_for_harness`) | | | |

**Preserved invariants (NFR-005, the behaviour-preservation contract):**
- Fixed sink order `first_clash → hooks → guardrails → agents`.
- `first_error` precedence (hooks 43 > guardrails 46 > agents 45) — proven unchanged by `tests/harness_sync_p6_first_error.rs`.
- The **mass-delete safeguard**: open the central DB read-only and **propagate** on an existing DB (never `.ok()`-swallow → empties the enabled set → mass-delete) — carried into each module's reconcile fn.
- The `SyncSubsystem` arm order + the `<sink>_action`-appended-last field order (so the `SyncOutcome` byte-stable JSON pin does not move a field).
- Idempotence (mtime-stable re-sync) — proven unchanged by `sync_idempotence.rs` / `harness_sync_p6_idempotence.rs`.
- **RUST-1/RUST-2 efficiency cleanups are NOT applied** (out of scope; folding them would risk behaviour change).

---

## 6. In-process MCP test harness (FR-012) — test-side types only

| Type (test-side, `tests/common/`) | Purpose |
|---|---|
| in-process server driver | constructs a real MCP server instance via the library API (no ONNX load — `StubEmbedder`), drives `prompts/list` / `prompts/get` / tool calls, and surfaces the MCP-internal exit codes |
| `tests/exit_codes_e2e_mcp.rs` | exercises GAP-1 codes **9** (`PluginDataDirWriteFailed`), **26** (`PromptArgumentMismatch`), **27** (`EntryNotFound`), **28** (`SubstitutionFailed`), **29** (`InvalidArgumentFrontmatter`) end-to-end; verifies the FR-004 fix (command+skill+`foo2` → all present + resolvable) through the prompt list/get path |

No production type changes. Built on the established `#[doc(hidden)] pub static` override-slot + RAII-guard seam pattern; reuses `EnvVarGuard`/`ENV_MUTEX` (promote to `tests/common/mod.rs` if this is the 5th consumer).

---

## 7. Release & packaging entities (US4/US5) — repo-root artifacts, no in-memory model

| Entity | Where | Notes |
|---|---|---|
| crate name | `Cargo.toml [package] name = "tome-mcp"` | the published crate (FR-017) |
| binary name | `Cargo.toml [[bin]] name = "tome"` | the user-facing command stays `tome` (FR-017) |
| discovery metadata | `Cargo.toml` `authors`, `homepage`, `documentation` (+ existing `repository`/`categories`/`keywords`) | FR-025 |
| docs.rs config | `Cargo.toml [package.metadata.docs.rs]` (or `documentation` → repo) | FR-025/§R-16 |
| crate tarball allowlist | `Cargo.toml include`/`exclude` | ships `src/**`, `build.rs`, `vendor/**`, `Cargo.toml`, `Cargo.lock`, `README.md`, `CHANGELOG.md`, `LICENSE-*`; excludes `specs/`, `.sdd/`, `review/`, `retro/`, `CLAUDE.md`, the two `*.local.json` (FR-024) |
| release pipeline config | `cargo-dist` config + generated `.github/workflows/release.yml` | Linux+macOS × x86_64+aarch64; checksums; tap push; crates.io publish; same gates as CI (FR-018/019, NFR-010) |
| `THIRD-PARTY-LICENSES` | repo root + attached to releases | `cargo-about` over the cargo graph + appended native notices (ONNX Runtime, llama.cpp, vendored sqlite-vec) (FR-019, NFR-007) |
| `SECURITY.md` | repo root | private-reporting channel; placeholder email removed (FR-022) |
| `CONSTITUTION.md` | repo root | **STAYS tracked**; amended to v1.4.0 (FR-023) |
| version | `Cargo.toml version = "0.6.0"` | first public version stays 0.6.0 (FR-025) |
| `Cargo.lock` | repo root + tarball | committed, authoritative, `--locked` builds (NFR-008) |

**Untracking (FR-024) ≠ deletion**: `git rm --cached` + `.gitignore` the internal process dirs; **local copies retained** so the SDD/closeout workflow keeps reading them from the working tree. This is the **final step**, after all hardening PRs merge.

---

## 8. Errors (`src/error.rs`, existing enum — **no new variant, no new exit code**)

| Failure (Phase 7) | Reused variant | Exit code | Note |
|---|---|---|---|
| Malformed `~/.tome/config.toml` (FR-014) | `ManifestInvalid::TomlParse { file, message }` | **5** | replaces today's generic `Internal` (exit 1) collapse |
| Non-array `hooks` event value (FR-015) | `HookSettingsWriteFailed { path }` | **44** | fail closed; do **not** coerce to `[]` |
| Meta row indicating corruption (FR-015) | existing variants (a diagnostic *distinction*, e.g. `IndexIntegrityCheckFailure` vs. fresh-DB bootstrap) | (existing) | distinguish corruption from a fresh DB; **no new code** |
| Symlink intermediate-component refusal (FR-007) | the **calling sink's** existing write-guard variant (`HookSettingsWriteFailed` 44 / `GuardrailsWriteFailed` 46 / agents / rules / mcp) | (existing) | never generic `Io` (7) on a dedicated sink |
| Bounded-read overflow (FR-006) | the read's existing per-class parse/size error (`ManifestInvalid` / frontmatter parse) naming the file | (existing) | per-class cap, named error |

Closed-set discipline (§II NON-NEGOTIABLE) preserved: no `Other`/`Unknown` arm; the occupied code set (`1–9, 13–37, 40–46, 50–54, 60–61, 70, 73–75`) gains nothing. Verified against `src/error.rs::exit_code` (the canonical truth).

---

## 9. Dependency graph delta

| Change | Kind | Graph effect |
|---|---|---|
| `rustix` | transitive → **direct** (`features = ["fs"]`) | **no new package** — `rustix v1.1.4` already resolved via `tempfile`/`crossterm`; `fs` already enabled transitively. Licence `Apache-2.0 OR MIT` on the allowlist. Binary-size delta ≈ 0. |
| `cargo-dist`, `cargo-about` | CI tools | **not** `[dependencies]`; zero effect on the binary or `cargo-deny` graph |

MSRV unchanged (NFR-009). `ort` stays pinned at the RC; `llama-cpp-2` stays exact-pinned (`=0.1.146`). `Cargo.lock` regenerated only for the crate-rename `name` field (run `cargo check` before committing so the gate doesn't dirty the lock).
