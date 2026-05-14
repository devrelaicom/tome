# Known Concerns

> **Purpose**: Document technical debt, known risks, bugs, fragile areas, and improvement opportunities.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-14 (Phase 3 / US5 forward schema migrations shipped)

## Technical Debt

### High Priority

Items that should be addressed in current or near-term phases:

| ID | Area | Description | Impact | Effort | Notes |
|----|------|-------------|--------|--------|-------|
| TD-001 | `src/index/` (Phase 2) | Advisory locking for concurrent catalog/index access | Concurrency safety | High | Phase 2 MCP server exposes concurrent harness access; advisory lockfile (FR-040) is designed but T088 (real BGE testing) is pending |
| TD-003 | Binary size (Phase 2/3) | SQLite + ONNX pushed binary to 29.56 MB; cap revised to 50 MB (CONSTITUTION v1.2.0) | Headroom management | Medium | 20.4 MB headroom remains; discipline holds, number changed. Justify any further additions |

### Medium Priority

Items to address when working in the area:

| ID | Area | Description | Impact | Effort | Mitigation |
|----|------|-------------|--------|--------|-----------|
| TD-010 | `src/embedding/download.rs` | No byte-progress callback for model downloads | UX | Low | Currently wrapped in indeterminate spinner in both `plugin enable` and `models download`; enhancement for polish pass. Also applies to `tome reindex` progress visibility (Phase 7) |
| TD-011 | `src/index/migrations.rs` (Phase 3 F7 + US5) | Schema-migration framework implementation complete; zero registered migrations shipped | Testing coverage | Low | Framework landed in Phase 3 Foundational F7 + US5. Phase 4+ adds first real `Migration` rows + e2e test via `MIGRATIONS_OVERRIDE` injection |
| TD-012 | `src/mcp/preflight.rs` (Phase 3 F8) | MCP startup pre-flight runs SHA-256 over primary embedder (~66 MB) at every startup | Startup latency | Low | Acceptable for long-running server; cold-cache startup may see latency. Consider `--verify` flag on `tome status` to skip SHA-256 on non-suspect runs (similar to Phase 6 pattern). Defer unless profiling shows impact |
| TD-013 | Phase 3 US1 testing | T088 manual verification still pending: real BGE models + live harness for SC-001/SC-002 coverage | Integration testing | High | Three categories: happy-path search_skills/get_skill returns (T092 partial), MCP protocol purity (T093), latency budget (T094 p50<300ms, p99<600ms), SIGINT graceful shutdown (T095). Tracked in `retro/P3.md` |
| TD-014 | `src/mcp/state.rs` (Phase 3 F8) | McpState embedder/reranker seed exposure for test integration | Test isolation | Medium | Handlers derive seeds from `state.embedder_entry.name/version`, hard-coded to MODEL_REGISTRY entries. Tests can't bootstrap index with stub seeds + use handlers without tripping drift detection. Refactor `McpState` to carry `embedder_seed` / `reranker_seed` directly. Est. 1 hour, defer to post-US1 |
| TD-015 | `contracts/mcp-server.md` (Phase 3 F8) | Contract drift on startup failure codes | Documentation | Low | Contract lists exit 35 for "Index DB missing" but production code surfaces exit 60 (`McpStartupFailed { reason: "index_missing" }`). 35 maps to `VectorExtensionInitFailure`; neither fits. 60-with-reason approach is more accurate; amend contract in polish pass |
| TD-016 | `src/workspace/init.rs` (Phase 3 US2) | `.tome.old/` orphan cleanup on crash between rename-aside and rename-in | Recovery cleanup | Low | If `--force` rename-in fails after moving old `.tome/` to `.tome.old/`, rollback restores the old state. But if a crash occurs between rename-aside and rename-in (before rollback logic), `.tome.old/` is left orphan. Doctor (future Phase, out of scope for US2) should surface and offer cleanup. Currently documented in `contracts/workspace-init.md` §Side effects as a known limitation. |
| TD-017 | `src/catalog/store.rs::reference_count` (Phase 3 US3) | Catalog cache TOCTOU window between pre-check and `remove_dir_all` | Concurrency safety | Low | Two processes racing `tome catalog remove` may both observe empty refs and both call `remove_dir_all` (benign: one wins, one no-ops). Worse: process A observes empty, process B adds URL before A deletes clone → dangling reference (recovered by `tome catalog update` re-clone). Documented design; same profile as Phase 9 cascade pre-check. No lock taken because readers shouldn't block writers. |
| TD-018 | `src/doctor/harness_detect.rs` (Phase 3 US4) | Harness-detected list is privacy-sensitive; if a future feature transmits doctor reports, this list leaks which tools are installed | Privacy | Low | Presently local-only (never transmitted); document explicitly. Review at design time if any downstream feature proposes report transmission (e.g., crash reporting, bug-filing UI). Recommend opt-in privacy gate before enabling network transmission. |
| TD-020 | Error categorisation | All Phase 1 + Phase 2 codes are enumerated; no catch-all variants | Debuggability | Low | Current approach is sound; closed set enforces completeness |
| TD-040 | Logging verbosity | Current `-v` / `-vv` mapping is fine; `TOME_LOG` env filter is undocumented | UX | Low | — |

### Low Priority

Nice-to-have improvements:

| ID | Area | Description | Impact | Effort |
|----|------|-------------|--------|--------|
| TD-050 | Presentation module exports | `comfy_table::Cell` + `CellAlignment` imported directly by consumers | API convenience | Low |

## Security Concerns

### High Risk

| ID | Area | Description | Risk Level | Mitigation | Status |
|----|------|-------------|------------|-----------|--------|
| SEC-001 | Phase 2 BGE testing (T088) | Vector search correctness not yet measured against real BGE models (bge-small-en-v1.5, bge-reranker-base) | High | Complete developer-machine pass with real models; validate SC-001 / SC-002 | Pending Phase 3 |

### Medium Risk

| ID | Area | Description | Risk Level | Mitigation | Status |
|----|------|-------------|------------|-----------|--------|
| SEC-002 | Phase 3+ model-download UX | User declines model-download prompt (e.g., in `tome plugin enable`) → returns exit 8 (reused from Interrupted); no dedicated exit code | Medium | Lock down user-decline vs. system-interrupt distinction in future iteration | Design debt |
| SEC-003 | Phase 5 disable confirmation | User declining disable confirmation returns 0 (no error); semantically different from model-download decline (also 0) and system interrupt (exit 8) | Low | Currently consistent with interactive flow semantics; monitor for UX confusion | Documented |
| SEC-010 | Credential scrubber (Phase 1) | Regex-based scrubbing is pattern-based, not semantic; exotic credential formats may leak (e.g., GitLab private tokens with special delimiters) | Medium | Current rules (R-8) cover common patterns. Add integration tests against real Git helper output. Monitor GitHub issues | Ongoing |

### Low Risk

| ID | Area | Description | Risk Level | Mitigation | Status |
|----|------|-------------|------------|-----------|--------|
| SEC-011 | Plugin identity validation (Phase 2) | Shape validation prevents directory traversal (`..`, `.`, `/`, etc.), but doesn't constrain character set; Unicode or non-ASCII plugin names are accepted | Low | Lenient on purpose (forward-compat); real-world risk is low. Monitor for exploit reports | Documented |
| SEC-020 | MSRV drift | Dependency updates may require MSRV bump; current MSRV is pinned (1.93) but not validated in a separate CI job | Low | MSRV CI job exists and passes; keep Renovate PRs reviewed for MSRV compatibility | CI gate in place |

## Known Bugs

Active bugs that haven't been fixed:

| ID | Description | Workaround | Severity | Status |
|----|-------------|------------|----------|--------|
| (none documented) | — | — | — | All known issues tracked in GitHub issues; no unfixed bugs blocking shipped phases |

## Fragile Areas

Code areas that are brittle or risky to modify:

| Area | Why Fragile | Precautions |
|------|-------------|-------------|
| `src/catalog/git.rs::scrub_credentials` | Regex patterns are order-dependent; adding a rule can change ordering semantics | Add test case to `tests/scrubbing.rs` for every rule addition; verify no overlaps with existing rules |
| `src/catalog/manifest.rs::validate_source` | Path canonicalization behavior differs across platforms (symlinks, case sensitivity); one test failure can indicate subtle cross-platform issue | Test on both Linux and macOS (CI covers both); `tests/path_validation.rs` has Unix-specific symlink tests |
| `src/catalog/store.rs::write_atomic` | Atomic rename only works on same filesystem; moving across mounts silently falls back to non-atomic copy | Document assumption in code; consider detecting mount boundary and erroring explicitly |
| `src/embedding/download.rs::download_model` | HTTP stream and checksum verification are separate; cleanup closure ensures both failure paths clean `.partial/` (lines 77–87) | Pipeline closure must wrap full download→verify→rename chain; any new step must be inside closure to maintain atomicity guarantee |
| `src/presentation/prompt.rs::require_terminal()` | TTY check runs on both stdin and stdout; must catch non-TTY in both dimensions to prevent prompt corruption via piped output | Always call `require_terminal()` at flow entry before any prompt; test with `Command::new()` (no pty) to verify short-circuit |
| `src/commands/plugin/{enable,disable,interactive}.rs` | Non-TTY pointer-message-then-error pattern appears at 3 sites (`enable`, `disable`, `interactive`) | Pattern consolidation would yield cleaner code; worth folding in when 4th occurrence appears |
| `src/index/skills.rs::upsert_skill` | `sqlite-vec` virtual tables do NOT support `INSERT OR REPLACE` or `ON CONFLICT` (Phase 7, PR #25 latent bug fix). Uses `DELETE`-then-`INSERT` which is idempotent | Verify this pattern on any future upsert-like operation involving virtual tables; do not attempt `INSERT OR REPLACE` on `skill_embeddings` |
| `src/main.rs::--version pre-parse` | Early arg scanning before clap dispatch is custom; any change to pre-parse logic could break `--version` routing | Test both `tome --version` and `tome -V` in CLI integration tests; verify `--json` flag is also detected; check that non-matching args pass through to clap normally |
| `src/plugin/lifecycle.rs::cascade_disable_for_catalog` | Single lock acquisition per cascade; each plugin's deletion is its own transaction. TOCTOU window between pre-check (without lock) and cascade (under lock): another process may enable a plugin between check and delete, causing its rows to be dropped too | This is intentional (readers never block writers). The pre-check reports a stale but valid list; the cascade acts on what's actually there and is correct either way. Document the TOCTOU window and its benign semantics |
| `src/catalog/store.rs::reference_count` (Phase 3 US3) | Reference-count read is NOT taken under advisory lock. Two processes racing `tome catalog remove` may both observe empty refs and call `remove_dir_all`; one succeeds, one no-ops. Worse race: process A observes empty, process B adds URL before A deletes clone → dangling reference (recovered by next `update`). | This is intentional (readers never block writers). Design mirrors Phase 9's cascade pre-check. TOCTOU window is documented and benign: clone persists until no references, delete is best-effort, dangling reference is recovered on next `update`. Opt-in workspace registry deduplicates scope set; without it (default), reference-count is global-only and removal deletes clone even if workspace references it. |
| `src/mcp/log.rs::FileMakeWriter` | Mutex serialises every JSON log emit; LockedFile guard holds lock for duration of write. MCP server is single-threaded, so contention is theoretical. Test harness shares handle across threads → must trust mutex semantics | Test isolation: don't share FileMakeWriter between concurrent test threads; use separate temp log files. Production: single-threaded by design (R-2), so no contention risk |
| `src/index/migrations.rs::MIGRATIONS_OVERRIDE` + `apply_pending` (Phase 3 US5) | Public static (not `#[cfg(test)]`) so integration tests outside crate can inject synthetic migrations. Per-migration atomicity via SQLite transactions under advisory lock. Forward-only boundary enforced—no down-migration path exists. | Documented as test-only via doc comment. Only read by production `apply_pending` (write path already under advisory lock). Each migration runs in own transaction; failure rolls back that migration + subsequent steps don't run. Test injection: ensure tests clear `MIGRATIONS_OVERRIDE` after use (idempotence expectation). Monitor: no production code should ever manually manipulate `MIGRATIONS_OVERRIDE`. |
| `src/mcp/preflight.rs::verify_embedder_artefacts` | Runs full SHA-256 over primary embedder (~66 MB) at every startup; no caching or early exit | By design for long-running server correctness (FR-110). Cold-cache startup latency visible to harness. Defer `--verify` skip flag to Phase 4+ unless profiling shows impact (TD-012). In test, use `StubEmbedder` to avoid real hash cost |
| `src/mcp/tools/{search_skills,get_skill}.rs::tome_to_mcp` | Error translation from TomeError to structured MCP codes must be exhaustive; missing variants leak as generic `internal_error` | Test assertion in `tests/mcp_server.rs` that all tool error paths translate to specific codes; audit on every new TomeError variant. No generic fallback (FR-108) |
| `src/workspace/init.rs::init` | Staging directory created inside workspace root to ensure same-filesystem rename atomicity (tempfile within the target directory). If workspace root is not on the intended filesystem, stage-rename could silently cross mount boundary (not atomic). Crash between rename-aside and rename-in leaves `.tome.old/` orphan. Rollback logic must restore `.tome/` from `.tome.old/` on final-rename failure. | Atomic staging pattern is sound: create in workspace root to guarantee same filesystem. `.tome.old/` orphans are recorded as TD-016; doctor (future) will clean up. Test interruption scenarios thoroughly before shipping US3 (Phase 10+). Test rollback path on rename failure (pre-existing .tome/` with --force). |
| `src/doctor/harness_detect.rs::KNOWN_HARNESSES` | Fixed compile-time list of harness directories; adding a harness requires code change + contract sync (JSON enum) | Before adding: verify harness is widely deployed + stable. Update contract in `specs/003-phase-3-mcp-workspaces/contracts/doctor.md`. No runtime discovery — by design to avoid `$HOME` scanning |
| `src/doctor/fixes.rs` | Three auto-fixable repair classes (model re-download, catalog re-clone, schema forward-migration); each re-uses existing config-derived URLs and atomicity patterns | Each repair failure is logged and doesn't block subsequent repairs. After all repairs run, affected subsystems are re-checked and re-classified. Destructive repairs (drift, schema-too-new, manifest-invalid, orphan clone) are never auto-applied; they surface as suggested commands with `auto_fixable: false`. Monitor for edge cases where a repair might need to handle partially-failed state. |

## Deprecated Code

Code marked for removal:

| Area | Deprecation Reason | Removal Target | Replacement |
|------|-------------------|----------------|-------------|
| (none) | — | — | — |

All Phase 1, Phase 2, Phase 3 Foundational, Phase 4, Phase 5, Phase 6, Phase 7, Phase 8, Phase 9, and Phase 3 US1–US5 code is current; no legacy to remove yet.

## Performance Concerns

Known performance issues:

| ID | Area | Description | Impact | Mitigation |
|----|------|-------------|--------|------------|
| PERF-001 | Catalog refresh | Each `git fetch` is sequential; large catalogs block the command | Slow UX for multiple catalogs | Phase 1 spec requires sequential; parallelize in Phase 2 with async |
| PERF-010 | Cache validation | Manifest is re-parsed on every `show` command; no caching layer | Negligible impact (small files) | Cache not needed in Phase 1; revisit if Phase 2 manifests grow large |
| PERF-020 | Model download progress | Download wrapped in indeterminate spinner, not byte-progress bar | Poor visibility on large files | Enhancement for Phase 3 polish (TD-010). Phase 7 `tome reindex` also lacks per-skill progress visibility |
| PERF-030 | MCP pre-flight timing | SHA-256 over ~66 MB primary embedder file on every startup | Visible startup latency in cold cache | Acceptable for daemon; defer `--verify` optimization to Phase 4+ unless profiling shows impact (TD-012) |
| PERF-040 | Doctor command latency | Catalog enumeration + harness probing on every run (non-cached) | Slower than status; expected for comprehensive diagnosis | By design: status is the narrow fast path (~200 ms); doctor is the broad slower path for troubleshooting |

## TODO Items

Active TODO comments in codebase:

| Location | TODO | Priority | Status |
|----------|------|----------|--------|
| (none found) | — | — | Code is TODOs-clean; all planned work tracked in spec and PRs |

## External Dependency Risks

Dependencies that may need attention:

| Package | Version | Concern | Action Needed | Status |
|---------|---------|---------|---------------|--------|
| `clap` | 4.x | Actively maintained; track for 5.x breaking changes | Monitor releases; plan migration before major version bump | Stable |
| `serde` | 1.x | Stable; ecosystem standard | None | Stable |
| `rusqlite` | 0.31.x (Phase 2) | Bundled SQLite; monitor for platform-specific build issues | Test across CI matrix | Stable |
| `sqlite-vec` | vendored (Phase 2) | Custom C extension vendored under `vendor/sqlite-vec/`; compiled in via `build.rs` | Compiled as part of build; no separate update cadence | Pinned |
| `fastembed-rs` | (Phase 2) | Wraps ONNX Runtime; size-critical dependency | Monitor for updates; test binary size on bump | Active |
| `ort` (transitive) | (Phase 2) | ONNX Runtime via fastembed; intrinsically large (~25 MB contribution) | Size budget already accounted for; no waivable constraint | Locked by use case |
| `tempfile` | 3.x | Actively maintained; used for critical atomicity | Upgrade within 3.x when available; test after upgrade | Stable |
| `ctrlc` | 3.x | Small, stable crate; signal handling is straightforward | None in Phase 1; review if signal handling becomes more complex | Stable |
| `regex` | 1.x | Actively maintained; no known security issues | None | Stable |
| `reqwest` | 0.11.x (Phase 2) | HTTP client; used for model downloads | Monitor for TLS/security updates | Active |
| `indicatif` | (Phase 2) | Progress bar library; non-critical | Routine updates | Stable |
| `inquire` | (Phase 4+) | Interactive prompts library; used in non-TTY-refusable flows | Monitor for prompt-injection or TTY-related bugs; keep up to date | Stable |
| `rmcp` | (Phase 3) | MCP protocol implementation; required for MCP server (US1) | Monitor for spec-alignment updates; test integration with harness | Active |
| `tokio` | (Phase 3, scoped) | Async runtime; used only in `src/mcp/` (structural test enforces boundary) | Constitution gate: verify tokio stays out of Phase 1–2 code paths; test async boundary quarterly | Active |
| `tracing-subscriber` | (Phase 3) | Structured logging framework; used in MCP server only | Monitor for JSON formatter updates and file I/O edge cases | Stable |
| `schemars` | (Phase 3) | JSON schema generation for MCP tool inputs; used at compile-time | Monitor for schema correctness issues on MCP tool definitions | Active |

**No unmaintained or vulnerable dependencies detected.** `cargo-audit` weekly + PR checks.

## Improvement Opportunities

Areas that could benefit from enhancement:

| Area | Current State | Desired State | Benefit |
|------|---------------|---------------|---------|
| Config file validation | Parsed strictly, but no schema documentation | Add inline documentation or separate schema doc | Easier for users to edit config manually |
| Manifest validation errors | Clear but sometimes verbose | Add `--json` error detail with machine-readable remediation suggestions | Better tooling integration |
| Cancellation messaging | Silent on Ctrl-C (clean, but terse) | Optional verbose exit message for debugging | Better UX in automation contexts |
| Symlink security testing | Unix-only symlink escape test | Windows junction/hardlink escape tests | Cross-platform parity |
| Model download progress (TD-010) | Indeterminate spinner during download | Byte-progress bar with estimated completion | Better UX for large models |
| Non-TTY pointer pattern consolidation | `require_terminal()` check + pointer message duplicated in 3 command modules | Extract to shared helper | Cleaner code |
| Per-plugin reindex progress (Phase 7) | No per-skill progress or summary until completion | Stream progress per skill or plugin | Visibility into long multi-plugin reindex operations |
| Status command caching | Report is fully recomputed on every invocation | Cache computed parts (index metadata, drift) per query | Faster repeated health checks |
| MCP startup verbosity | Pre-flight SHA-256 silent unless it fails | Optional `--verbose` startup for diagnosing slow cold-cache initialization | Better observability |
| McpState test seed isolation (TD-014) | Hard-coded to MODEL_REGISTRY; can't test with stub seeds | Refactor McpState to carry explicit `embedder_seed` / `reranker_seed` | Better test isolation for MCP tool handlers |
| Workspace doctor command | No doctor or orphan-cleanup facility | Add `tome doctor` command to detect `.tome.old/` orphans and offer cleanup | Recovery aid for crashed `--force` init |
| Catalog cache `--force` re-add | Not yet implemented; users must manually remove cached clones | Implement `tome catalog add --force` to bypass cache re-use on URL re-add | Users concerned about URL hijacking can force fresh clone |
| Doctor report transmission | Report currently local-only; no transmission path implemented | If adding a transmission feature (crash reporting, bug-filing), implement privacy gate for harness-detected list | Respect user privacy; harness list is sensitive (leaks installed tools) |

## Monitoring Gaps

Areas lacking proper observability:

| Area | Missing | Impact | Priority |
|------|---------|--------|----------|
| Git operation timing | No latency metrics | Can't detect slow clones/fetches in automation | Low (single-user CLI; Phase 2 MCP may need metrics) |
| Index database health | No validation of persisted state on startup (Phase 2) | Corrupted index undetected until query | Low (atomicity guarantees + integrity_check PRAGMA should prevent corruption) |
| Model download errors | Network failures not distinguished from checksum failures | Harder to diagnose transient vs. persistent issues | Low (both map to Io/ModelChecksumMismatch; rare in practice) |
| Catalog size statistics | No cache size tracking | Can't warn on large catalogs | Low (Phase 2 may add quota management) |
| MCP pre-flight timing | SHA-256 verify of large embedder file not instrumented | Cold-cache startup latency not observable | Low (acceptable trade-off; defer unless profiling shows impact per TD-012) |
| Workspace registry state | No metrics on registry size or dedupe ratio | Can't detect growth or churn patterns | Low (opt-in registry; steady-state expected to be small) |
| Catalog cache TOCTOU races | No instrumentation of concurrent `remove` races or dangling reference recovery | Can't detect real-world contention or re-clone frequency | Low (opt-in registry centralizes scope set; default install has only global scope) |
| Doctor repair outcomes | No metrics on which repairs succeeded/failed or how frequently | Can't track real-world doctor --fix usage or effectiveness | Low (reports include success/failure per repair; log analysis sufficient for early phase) |
| Schema migration execution | No metrics on migration latency or per-step execution time | Can't diagnose slow migrations or detect regressions | Low (Phase 4+ first real migrations; defer instrumentation until production data available) |

## Design Tradeoffs

Intentional design decisions with known limitations:

| Decision | Area | Rationale | Consequence | Notes |
|----------|------|-----------|-------------|-------|
| **Per-plugin atomicity** (Phase 7) | `src/index/skills.rs::reindex_plugin_atomic` | Simpler transaction model; each plugin reindex commits independently | Multi-plugin `tome catalog update` or `tome reindex` may leave earlier plugins committed if interrupted between plugins | Safe state always (no partial rows); index is always valid. By design, not a bug. Advisory lock per-plugin at entry to reindex, released at commit. |
| **Status lock-free** (Phase 8) | `src/commands/status.rs::run` (no advisory lock taken) | Allows health check to run even when a writer is running; supports use as a non-invasive doctor command (FR-056) | Status report is a point-in-time snapshot; may be stale if another process is concurrently writing | Acceptable trade-off for pre-flight non-blocking diagnosis. Caller should understand the snapshot may be moments old. |
| **Cascade disable under single lock** (Phase 9) | `src/plugin/lifecycle.rs::cascade_disable_for_catalog` | Batch operation atomicity: all plugins disabled and rows dropped within one lock window; simpler than per-plugin acquisitions (Phase 7 pattern) | Each plugin's deletion is its own transaction (not atomic across plugins), so SIGINT between plugins leaves earlier plugins dropped + later plugins intact. Index is always valid; partial state is well-defined | By design. Index WAL + transaction isolation ensures each deletion is durable and correct. Pre-check (enabled-plugin query) runs WITHOUT lock, accepting TOCTOU risk of stale enabled list — acceptable because cascade acts on actual state (still correct) and reader-never-blocks-writer is the locking principle. |
| **Catalog cache ref-counting without lock** (Phase 3 US3) | `src/catalog/store.rs::reference_count` | Allows concurrent `tome catalog remove` without serialization; readers never block writers | Two processes racing may both try to delete the same clone (benign: one wins, one no-ops). Worse race: process A deletes after B adds reference → dangling clone (recovered by next `update`). Default (global-only refs) deletes clone even if workspace still references it; workspace must re-clone. | Intentional. Same TOCTOU profile as Phase 9 cascade pre-check. Opt-in workspace registry centralizes scope set to reduce collision likelihood but doesn't eliminate TOCTOU. Trade-off: no lock contention vs. possible extra re-clone (one round-trip). Benign for catalog operations (not on critical path). |
| **MCP startup SHA-256 verification** (Phase 3 F8) | `src/mcp/preflight.rs::verify_embedder_artefacts` | Long-running server: paying full hash once at startup is right trade-off vs. long-running process correctness | Cold-cache startup latency visible to harness (potential second-range delay for ~66 MB file). Acceptable for daemon; defer `--verify` skip flag to Phase 4+ unless profiling shows impact (TD-012) | Pre-flight gates before MCP protocol handshake, so harness sees startup delay before first RPC. Not user-facing command latency. Consider optional startup flag for repeated local dev testing (mitigates cold-cache cost, trade-off: skip verification). |
| **No MCP tool description enumeration** (Phase 3 F8, FR-108) | `src/mcp/server.rs` tool descriptions | Tool descriptions must NOT reference specific catalog/plugin/skill identifiers in fixture or test scope | Test `tests/mcp_server.rs::descriptions_do_not_enumerate_fixture_identifiers` fails if wording includes identifiers, preventing accidental info leakage | By design. Descriptions are generic guidance; discovery happens via `search_skills` / `get_skill` at runtime. |
| **Structured error codes for MCP tools** (Phase 3 F8) | `src/mcp/tools/{search_skills,get_skill}.rs::tome_to_mcp` | Tool errors map to contract-defined structured codes (`unknown_catalog`, `unknown_plugin`, etc.), never exposing internal TomeError variants | MCP harness sees opaque error codes; no domain-error info leakage | By design. Security + clarity: harness cannot infer internal structure or state from error messages. |
| **Forward-only schema migrations** (Phase 3 F7 + US5) | `src/index/migrations.rs` + `TomeError::SchemaVersionTooNew` (exit 73) | Simpler DB evolution: v2.1 patch adds one migration row; older Tome refuses newer DBs. No down-migration complexity. Per-migration atomicity via SQLite transactions under advisory lock. | Users on older Tome version cannot open DBs created/modified by newer version. Each migration step atomic; failure rolls back + stops chain. | Acceptable: users upgrade Tome regularly. Phase 1 is shipped and stable; Phase 2+ are synchronized. Old-version downgrade is not a supported use case. Atomicity per-step (not across-step) sufficient for correctness + recovery. |
| **Workspace staging inside root** (Phase 3 US2) | `src/workspace/init.rs::init` | Ensures same-filesystem rename atomicity; staging directory created via `tempfile::Builder::tempdir_in(&absolute)` | If workspace root spans a mount boundary, the atomic-rename assumption holds true ONLY for files under workspace root (tempfile stays in workspace root). Crash between rename-aside and rename-in leaves `.tome.old/` orphan (recovered by doctor in future phase). Rollback on final-rename failure restores old `.tome/` from `.tome.old/`. | By design. Atomicity is guaranteed for common case (workspace on single mount). TD-016 tracks `.tome.old/` orphan recovery. Test all error paths thoroughly before Phase 10+. |
| **Doctor harness detection existence-only** (Phase 3 US4, FR-167) | `src/doctor/harness_detect.rs::probe` | Detection is fixed compile-time list of directories; directory-existence check only. No `$HOME` scanning, no config parsing. | Harness list is exclusive; new harnesses require code change + contract sync. Trade-off: precise known list vs. discovery flexibility. | By design for privacy + simplicity. No runtime scanning of user home. Detected-harness list is local-only and privacy-sensitive; never transmitted unless a future feature explicitly adds network transmission (which should include privacy gate per TD-018). |
| **Doctor `--fix` network gate** (Phase 3 US4) | `src/commands/doctor.rs`, `src/doctor/fixes.rs` | All network access (model re-download, catalog re-clone) gated behind `--fix` flag. Read-only diagnostic is default. | Users get a non-invasive report before deciding to attempt repairs. Trade-off: two invocations (report + fix) vs. automatic repairs. | By design for transparency + user control. Report surfaces suggested fixes; user decides whether to run `--fix`. No privilege escalation (all URLs/paths come from user's config). |

## Risk Summary by Phase

### Phase 1 (Complete)

**Status**: All critical security controls implemented and tested.

- ✓ Credential scrubbing at capture boundary
- ✓ Path traversal prevention (six-step validation)
- ✓ Atomic writes for registry and cache
- ✓ Signal handling with clean child cleanup
- ✓ Closed error set with documented exit codes
- ✓ Licence and vulnerability scanning in CI

**Open items**: None blocking Phase 1 completion.

### Phase 2 (Foundational Complete, T088 Pending)

**Expected introducing**: Async runtime (deferred), concurrent harness access, SQLite integration, vector search.

**Completed**:
- ✓ Index database with advisory locking (FR-040)
- ✓ Vector search interface (Embedder trait)
- ✓ Model download with integrity verification
- ✓ Plugin enable/disable lifecycle
- ✓ Query command infrastructure

**Key risks to plan / monitor**:
1. TD-001: Concurrent access via advisory lockfile (designed, tested on fixtures)
2. SEC-001: Real BGE model testing (SC-001/SC-002) — T088 pending developer-machine pass
3. SEC-002: User-decline exit code distinct from system interrupt (design debt, low priority)

### Phase 3 (All User Stories Complete: US1–US5)

**US1 (MCP Server)**:
- ✓ MCP server startup pre-flight validation (FR-110)
- ✓ Embedder SHA-256 verification at startup + drift detection (41/42 codes)
- ✓ MCP protocol purity: stdout reserved for protocol, errors to stderr (FR-221/FR-222)
- ✓ Structured JSON-lines logging with 10 MiB rotation cap (`src/mcp/log.rs`)
- ⚠ T088 manual BGE testing still pending (SC-001/SC-002, happy-path, protocol purity, latency, SIGINT)

**US2 (Workspace Info & Init)**:
- ✓ `tome workspace info` read-only query command
- ✓ `tome workspace init` atomic `.tome/` creation with permission lock (chmod 0700 on staging before content)
- ✓ Workspace config written atomically via `catalog::store::write_atomic` (chmod 0600 on Unix)
- ✓ Opt-in workspace registry (`workspaces.txt`) append-only with dedupe by exact path

**US3 (Catalog-Clone Ref-Counting)**:
- ✓ Reference-counting across global + workspace scopes (via opt-in registry)
- ✓ Catalog cache re-used on same URL (no re-fetch); deleted when no references remain
- ✓ TOCTOU documented: no lock on reference-count read; readers never block writers

**US4 (Doctor Command)**:
- ✓ Read-only diagnostic by default; network access gated entirely behind `--fix` flag
- ✓ Harness detection via fixed compile-time list (directory existence only, FR-167)
- ✓ Three auto-fixable repairs: model re-download, catalog re-clone, schema forward-migration

**US5 (Forward Schema Migrations)**:
- ✓ Schema migration framework with forward-only policy (zero migrations shipped)
- ✓ Per-migration transaction atomicity under advisory lock
- ✓ `SchemaVersionTooNew` (exit 73) + `SchemaMigrationFailed` (exit 74) exit codes
- ✓ `MIGRATIONS_OVERRIDE` test-injection point for e2e tests (Phase 4+)

**Key risks to monitor**:
- SEC-001: BGE model testing still pending (T088)
- SEC-002: User-decline vs. interrupt exit code (design debt)
- TD-010: Model download progress UX (polish pass)
- TD-011: Schema migration e2e tests pending (Phase 4+)
- TD-012: MCP startup SHA-256 latency on cold cache (acceptable; defer unless profiling shows impact)
- TD-013: Phase 3 US1 manual testing incomplete (T088)
- TD-014: McpState seed exposure blocks full integration test isolation (est. 1 hour refactor)
- TD-015: Contract drift on exit codes (60 vs. 35 for index missing; amend contract)
- TD-016: `.tome.old/` orphan cleanup (doctor, Phase 10+)
- TD-017: Catalog cache TOCTOU on `reference_count` (documented and benign; same profile as cascade pre-check)
- TD-018: Harness-detected list privacy gate for future transmission features (review at design time)

### Phase 4 (Complete)

**Completed (US2)**:
- ✓ Interactive `tome plugin` browse flow with catalog/plugin/action selectors
- ✓ TTY enforcement at flow entry via `require_terminal()` — exit 54 if no TTY (FR-051)
- ✓ Prompt functions (`select`, `multiselect`, `confirm`) with non-TTY short-circuits
- ✓ Post-action redraw and navigation (Back, Quit)

**Key security additions**:
- Interactive flows refuse to run in non-TTY contexts (exit 54), preventing prompt-injection and mangled input

### Phase 5 (Complete)

**Completed (US3)**:
- ✓ `tome plugin disable` command with confirmation prompt (FR-005 / FR-007)
- ✓ Non-TTY refusal with `--force` bypass (FR-051)
- ✓ Cheap re-enable via retained embeddings (FR-006)

**Key security characteristics**:
- Disable reads catalog config + writes to index DB under existing advisory lock + transaction discipline

### Phase 6 (Complete)

**Completed (US4)**:
- ✓ `tome models download | list | remove` CLI surface
- ✓ Surfaces existing security primitives: pinned MODEL_REGISTRY SHA-256s, atomic-rename + checksum-verify pipeline
- ✓ New `embedding::download::sha256_file` public helper for re-verifying installed artefacts via `--verify` flag

### Phase 7 (Complete)

**Completed (US5)**:
- ✓ `src/index/skills.rs::reindex_plugin_atomic` + `auto_disable_orphan` for multi-plugin index reconciliation
- ✓ `tome catalog update` + `tome reindex` CLI commands with lazy-embedder loading
- ✓ Fixed latent `sqlite-vec` virtual table bug: `upsert_skill` now uses `DELETE`-then-`INSERT`

### Phase 8 (Complete)

**Completed (US6)**:
- ✓ `tome status [--verify] [--json]` read-only health check command (FR-056)
- ✓ Extended `tome --version` / `tome -V` to include embedder + reranker identities
- ✓ Drift detection + classification (Ok, Degraded, Unhealthy) to guide bug reporting

### Phase 9 (Complete)

**Completed (US7)**:
- ✓ `tome catalog remove --force` cascade disable + row drop for enabled plugins (FR-045)
- ✓ Pre-check query `enabled_plugins_for_catalog` runs without lock (readers don't block writers)
- ✓ Cascade itself runs under single lock acquisition; each plugin's deletion is its own transaction

### Phase 3 US3 (Complete)

**Completed (Catalog Cache Ref-Counting)**:
- ✓ `src/catalog/store.rs::reference_count` enumerates all scopes (global + workspace registry)
- ✓ Catalog clone persists until no scopes reference the URL; deleted on last removal
- ✓ Re-adding same URL re-uses existing clone (content-addressed by `sha256(url)`)

### Phase 3 US4 (Complete)

**Completed (Doctor Command)**:
- ✓ `tome doctor [--fix] [--json]` read-only diagnostic + optional automatic repairs
- ✓ Harness detection: fixed compile-time `KNOWN_HARNESSES` list; directory-existence only (FR-167)
- ✓ Three auto-fixable repairs: model re-download, catalog re-clone, schema forward-migration
- ✓ Four un-fixable repairs surfaced as suggested commands (drift, schema-too-new, manifest-invalid, orphan clones)

---

## Concern Severity Guide

| Level | Definition | Response Time |
|-------|------------|----------------|
| Critical | Production impact, security breach, data loss | Immediate (within sprint) |
| High | Degraded functionality, significant risk, blocking future phases | This sprint |
| Medium | Developer experience, minor risk, addressable when working in area | Next sprint |
| Low | Nice to have, cosmetic, low impact | Backlog |

---

## What Does NOT Belong Here

- Active implementation tasks → GitHub issues + project board
- Security controls (what we do right) → SECURITY.md
- Architecture decisions → ARCHITECTURE.md
- Code conventions → CONVENTIONS.md

---

*This document tracks what needs attention. Update when concerns are resolved or discovered.*
