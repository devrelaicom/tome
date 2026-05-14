# Phase 3 — Review Disposition

Per the Phase 2 polish-phase mapping (PRs #34–#40):

- **Blockers** — each gets a dedicated PR (or grouped if mechanically related)
- **Majors** — grouped logically into PRs
- **Minors** — folded into the relevant slice PR; only addressed where cheap
- **Nits** — `wontfix` unless trivial-while-nearby

Disposition for each finding:

- **fix** — apply now in Polish
- **fix-trivial** — apply now in Polish; quick win
- **defer-test** — close in the deferred-coverage PR (T218–T221)
- **defer-docs** — close in the docs PR (CHANGELOG / README / contract reconciliation)
- **defer-phase-4** — beyond Phase 3 scope; tracked in CONCERNS.md / future PRD
- **wontfix** — accept; document rationale if non-obvious

## Slice plan (PRs)

Following Phase 2's polish-phase shape:

| PR | Scope | Approx LOC |
|---|---|---|
| **PR-A** | Review findings + disposition (this commit) | docs only |
| **PR-B** | Blocker fix B1 — `mcp.log` JSON field names | ~30 src + ~50 test |
| **PR-C** | Blocker fix B3 + B2 — doctor drift coverage + resolver validation gate | ~80 src + ~150 test |
| **PR-D** | Major fixes — MCP signal handling, log scrubbing, top_k schema, get_skill bounds | ~150 src + ~150 test |
| **PR-E** | Major fixes — doctor orphan-clone + registry-status reporting + dead-code cleanup | ~120 src + ~100 test |
| **PR-F** | Security hardening — `mcp.log` 0600, registry validation, symlink guard | ~80 src + ~80 test |
| **PR-G** | Test coverage gaps (T218-T221 deferred items + new gaps) | ~250 test |
| **PR-H** | Contract drift reconciliation + minor doc-string updates | ~contracts |
| **PR-I** | README, CHANGELOG, --help text, version bump to v0.3.0 | docs |
| **PR-J** | Final closeout — codebase docs refresh + P8 retro | docs |

Soft cap per PR ~400 lines; split further if any single fix balloons.

---

## Blockers

| # | Title | Disposition | PR |
|---|---|---|---|
| B1 | `mcp.log` JSON field names diverge from contract | **fix** — add explicit field-renamer to `fmt::layer().json()`; pin via integration test | PR-B |
| B2 | Resolver §Validation 1b/1c not enforced | **fix** — add the gate to `walk_cwd_for_marker` + `validate_workspace_path`; new test | PR-C |
| B3 | Doctor drift reporting untested | **fix** — add embedder-drift → Unhealthy and reranker-drift → Degraded tests | PR-C |

---

## Majors

### MCP / log-format

| # | Disposition | PR | Notes |
|---|---|---|---|
| M-MCP-1 SIGTERM not handled | **fix** | PR-D | Add `signal::unix::signal(SignalKind::terminate())` arm |
| M-MCP-2 5s graceful-shutdown timeout | **fix** | PR-D | `tokio::time::timeout(5s, running.waiting())` |
| M-MCP-3 Stderr fatal-line shape | **fix** | PR-D | Custom format on the stderr layer (one line, category + code) |
| M-MCP-4 Index-missing → 60 instead of 35 | **fix** | PR-D | Map `!db_path.is_file()` to `IndexIntegrityCheckFailure` |
| M-MCP-5 top_k schema bounds not advertised | **fix-trivial** | PR-D | `#[schemars(range(min = 1, max = 100))]` |
| M-MCP-6 unknown_plugin / unknown_skill untested | **defer-test** | PR-G | Add to `tests/mcp_server.rs` |
| M-MCP-7 MCP tool *output* JSON schemas not pinned | **defer-test** | PR-G | Pin via `serde_json::to_value` byte-stable assertions |
| M-MCP-8 mcp_lifecycle.rs:19-22 false coverage claim | **fix-trivial** | PR-D | Edit comment; add the missing tests in PR-G |
| M-MCP-9 OnceCell::get_or_try_init doesn't cache failures | **wontfix** | — | Intentional (transient retries, success caches); document in src |
| M-MCP-10 get_skill unbounded resources list | **fix** | PR-D | Cap depth + count; surface truncation marker |
| M-MCP-11 Tracing subscriber install once-per-process | **wontfix** | — | `mcp::run` is one-shot per process by design; document |
| M-LOG-1 No credential scrubbing on workspace/error fields | **fix** | PR-D | Route through `git::scrub_credentials::scrub_to_string` at log boundary |
| M-LOG-2 "Hard shutdown" event never emitted | **fix** | PR-D | Emit `error!` on Err arm before returning |
| M-LOG-3 signal value only "SIGINT" literal | **fix** | PR-D | Folds into M-MCP-1 fix |
| M-LOG-4 Pre-flight failures never logged | **fix** | PR-D | Emit `error!` from each preflight failure branch |
| M-LOG-5 filter field Debug-rendered | **fix-trivial** | PR-D | Replace `?FilterLog` with `filter.catalog`/`filter.plugin` named fields |
| M-LOG-6 Log-format integration coverage zero | **defer-test** | PR-G | New `tests/mcp_log_format.rs` |

### Workspace

| # | Disposition | PR | Notes |
|---|---|---|---|
| M-WKS-1 TOME_WORKSPACE accepts relative | **fix** | PR-C | Folds with B2 |
| M-WKS-2 init --force not crash-atomic between renames | **fix** | PR-F | Propagate the pre-cleanup error, surface orphan via InitOutcome |
| M-WKS-3 Registry dedupe is exact-string | **fix-trivial** | PR-F | Canonicalise before dedupe |
| M-WKS-4 CLI default-path for workspace init untested | **defer-test** | PR-G | Single integration test |

### Doctor + catalog extensions

| # | Disposition | PR | Notes |
|---|---|---|---|
| M-DOC-1 Orphan-clone reporting missing | **fix** | PR-E | Walk `paths.catalogs_dir`, diff against config; surface as `CatalogCacheState::Orphan` (new variant) |
| M-DOC-2 Workspace-registry status line missing | **fix** | PR-E | Read `paths.workspace_registry`, emit human + JSON line |
| M-DOC-3 Schema-too-new through doctor untested | **defer-test** | PR-G | New test in `tests/doctor.rs` |
| M-DOC-4 doctor::fixes::apply misleading Result | **fix-trivial** | PR-E | Change signature to infallible |
| M-DOC-5 repair_schema is dead code | **fix** | PR-E | Emit `subsystem: "schema"` SuggestedFix when `current < SCHEMA_VERSION` |

### Schema migration + concurrency

| # | Disposition | PR | Notes |
|---|---|---|---|
| M-MIG-1 Migration concurrency / IndexBusy untested | **defer-test** | PR-G | New test |
| M-MIG-2 catalog_cache_refcount concurrent-test is sequential | **fix-trivial** | PR-G | `std::sync::Barrier` |

### Security

| # | Disposition | PR | Notes |
|---|---|---|---|
| S-01 mcp.log world-readable | **fix** | PR-F | `OpenOptionsExt::mode(0o600)` on Unix |
| S-02 get_skill symlink target leak | **fix** | PR-F | Reject `is_symlink()` entries or canonicalise + assert starts_with(plugin_root) |
| S-03 Workspace registry trusted | **fix** | PR-F | Size cap (1 MiB), entry cap (10k), reject `..` / NUL bytes |
| S-04 init mis-classifies non-directory markers | **fix** | PR-F | Require `marker.is_dir()` for `--force` rename; reject regular files / symlinks |

---

## Minors

Folded into the relevant slice PR where cheap. Highlights:

| # | Disposition | PR |
|---|---|---|
| m-MCP-4 defensive envelope codes untested | **defer-test** | PR-G |
| m-MCP-5 `tome mcp --json` silently accepted | **wontfix** | — (`disable-help-flag` precedent; intentional) |
| m-MCP-7 walk_dir lossy UTF-8 | **wontfix** | — (filesystem reality; document) |
| m-MCP-8 search_skills.query unbounded | **fix-trivial** | PR-D (cap at 8 KiB) |
| m-WKS-1 walk_cwd_for_marker silent symlink chase | **wontfix** | — (deliberate per resolver perf; document) |
| m-WKS-3 init falls back to PathBuf::default() on CWD failure | **fix-trivial** | PR-F |
| m-WKS-4 inventory append unlocked | **wontfix** | — (document) |
| m-WKS-5 0700 mode not asserted | **defer-test** | PR-G |
| m-WKS-6 --inherit-global JSON inherited:true not pinned | **defer-test** | PR-G |
| m-WKS-7 .tome.old/ rollback path untested | **defer-test** | PR-G |
| m-WKS-8 "not yet bootstrapped" human render untested | **defer-test** | PR-G |
| m-WKS-9 cwd_walk + env ScopeSource JSON byte-stability | **defer-test** | PR-G |
| m-WKS-10 workspace info exit 7 / 35 untested | **defer-test** | PR-G |
| m-WKS-11 Stale registry entry untested | **defer-test** | PR-G |
| m-WKS-12 --workspace symlink chase | **wontfix** | — (document in --help) |
| m-DOC-1 doctor.md synopsis missing --verify | **defer-docs** | PR-H |
| m-DOC-4 DriftStatus JSON serialisation unverified | **fix-trivial** | PR-C (alongside B3 drift tests) |
| m-DOC-5 Glyph TTY vs ASCII fallback untested | **defer-test** | PR-G |
| m-DOC-9 doctor --json leaks $HOME paths | **wontfix** | — (interactive use; doc privacy note in --help) |
| m-DOC-10 reference_count silently drops malformed-config | **fix** | PR-E |
| m-CAT-1 manifest-missing → wrong variant | **fix-trivial** | PR-E |
| m-CAT-3 Paths.config_file field still reachable | **defer-phase-4** | — |
| m-ERR-1 McpStartupFailed.reason open String | **wontfix** | — (free-form by design) |
| m-PATH-1 Relative XDG_* silently filtered | **defer-phase-4** | — |
| m-MIG-1 Migration log lines missing scope/path | **fix-trivial** | PR-D |
| m-MIG-2 SchemaVersionTooNew Display missing index: line | **fix-trivial** | PR-H (contract / Display alignment) |
| m-MIG-3 apply_pending no-op case untested | **defer-test** | PR-G |
| m-MIG-4 Cross-scope migration isolation untested | **defer-test** | PR-G |
| m-MIG-5 apply_pending does not acquire lock | **defer-docs** | PR-H (contract clarification) |
| m-TEST-1 CLI tests use success() not Some(0) | **fix-trivial** | PR-G |
| m-TEST-2 status_reports_per_scope_index unwrap_or(0) | **fix-trivial** | PR-G |
| m-TEST-3 Human-substring vs JSON assertions | **wontfix** | — (cheap brittleness, not load-bearing) |

## Nits

All `wontfix` unless trivial-while-nearby:

| # | Disposition |
|---|---|
| n-* (most) | **wontfix** |
| n-MCP-10 explicit mode bits on OpenOptions | rolled into S-01 fix |
| n-SEC-1 init rollback errors silently swallowed | rolled into M-WKS-2 fix |

## Out-of-scope deferrals

| # | Disposition | Notes |
|---|---|---|
| T088 manual SC-001 / SC-002 + T093/T094/T095 MCP tests | **defer-phase-4** | Requires real BGE models + populated index + MCP client driver |
| `Paths.config_file` field rename | **defer-phase-4** | F1's `_for(&Scope)` accessor pattern made the rename optional; shelved per CLAUDE.md |
| MCP `Input` types length caps | **defer-phase-4** | Tracked in CONCERNS.md |
| `mcp.log` rotation discards prior history on each oversized restart | **wontfix** | Designed cap (10 + 10 = 20 MiB); accept |

---

## Closing

Total disposition counts:

| Action | Count |
|---|---|
| fix | ~30 |
| fix-trivial | ~14 |
| defer-test (PR-G) | ~20 |
| defer-docs (PR-H) | ~3 |
| defer-phase-4 | ~4 |
| wontfix | ~18 |

Roughly **44 fixes** to land across PRs B–F, plus **20 deferred test items** in PR-G, **3 doc reconciliations** in PR-H, **README+CHANGELOG+version** in PR-I, and **closeout** in PR-J.
