# Known Concerns

> **Purpose**: Document technical debt, known risks, bugs, fragile areas, and improvement opportunities.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-25 (Phase 4 / US1 + US2 completed; security audit findings applied)

## Technical Debt

### High Priority

Items that should be addressed in current or near-term phases:

| ID | Area | Description | Impact | Effort | Notes |
|----|------|-------------|--------|--------|-------|
| TD-001 | `src/index/` (Phase 2–4) | Advisory locking for concurrent catalog/index access | Concurrency safety | High | Phase 3 MCP server + Phase 4 central DB exposes concurrent access; advisory lockfile (FR-040, F11b FR-366) is implemented; T088 (real BGE testing) is the verification gap |
| TD-003 | Binary size (Phase 2–4) | SQLite + ONNX + llama.cpp pushed binary to ~30 MB; cap is 50 MB | Headroom management | Medium | Phase 4 projection: ~28.4 MB macOS arm64, ~34 MB Linux x86_64 (research §R-4); discipline holds with 16+ MB headroom |

### Medium Priority

Items to address when working in the area:

| ID | Area | Description | Impact | Effort | Mitigation | Status |
|----|------|-------------|--------|--------|-----------|--------|
| TD-010 | `src/embedding/download.rs` | No byte-progress callback for model downloads | UX | Low | Currently wrapped in indeterminate spinner in both `plugin enable` and `models download`; enhancement for polish pass. Phase 4 Qwen download (~400 MB) makes this more urgent; tracked in F6 | Phase 4 F6 uses indeterminate spinner; upgrade in polish or if time permits |
| TD-011 | `src/index/migrations.rs` (Phase 3 F7 + US5) | Schema-migration framework implementation complete; zero registered migrations shipped | Testing coverage | Low | Framework landed in Phase 3 Foundational F7 + US5. Phase 4 US adds first real `Migration` rows (v1→v2 structural migration); e2e test via `MIGRATIONS_OVERRIDE` injection verified in Phase 3 US5 | Phase 4 (in progress) |
| TD-012 | `src/mcp/preflight.rs` (Phase 3 F8) | MCP startup pre-flight runs SHA-256 over primary embedder (~66 MB) at every startup | Startup latency | Low | Acceptable for long-running server; cold-cache startup may see latency. Consider `--verify` flag on `tome status` to skip SHA-256 on non-suspect runs. Defer unless profiling shows impact | Acceptable design |
| TD-013 | Phase 3 US1 testing (T088, T093–T095) | Manual verification pending: real BGE models + live harness for SC-001/SC-002 coverage | Integration testing | High | Three categories: (1) happy-path search_skills/get_skill returns (T092 partial via `mcp_server.rs`), (2) MCP protocol purity (T093), (3) latency budget (T094 p50<300ms, p99<600ms), (4) SIGINT graceful shutdown (T095). Tracked in `retro/P3.md`. **Status**: T088 deferred pending developer access to real BGE models for live container/harness testing | Phase 5+ / Developer pass |
| TD-014 | `src/mcp/state.rs` (Phase 3 F8) | McpState embedder/reranker seed exposure for test integration | Test isolation | Medium | Handlers derive seeds from `state.embedder_entry.name/version`, hard-coded to MODEL_REGISTRY entries. Tests can't bootstrap index with stub seeds + use handlers without tripping drift detection. Refactor `McpState` to carry `embedder_seed` / `reranker_seed` directly. Est. 1 hour, defer to post-Phase 4 | Post-Phase 4 |
| TD-015 | Error code documentation drift | Contract vs. production code discrepancy on "Index DB missing" | Documentation | Low | Contract listed exit 35 for "Index DB missing" but production surfaces exit 60 (`McpStartupFailed`). Resolved in Phase 3 Polish PR #54; updated contract | Resolved Phase 3 |
| TD-016 | `src/workspace/init.rs` (Phase 3 US2) | `.tome.old/` orphan cleanup on crash between rename-aside and rename-in | Recovery cleanup | Low | If `--force` rename-in fails after moving old `.tome/` to `.tome.old/`, rollback restores the old state. But if a crash occurs between rename-aside and rename-in (before rollback logic), `.tome.old/` is left orphan. Phase 4 doctor extensions (US5) should surface and offer cleanup. Currently documented in contract as a known limitation (FR-M-WKS-2) | Phase 4 US5 doctor --fix |
| TD-017 | `src/catalog/store.rs::reference_count` (Phase 3 US3) | Catalog cache TOCTOU window between pre-check and `remove_dir_all` | Concurrency safety | Low | Two processes racing `tome catalog remove` may both observe empty refs and both call `remove_dir_all` (benign: one wins, one no-ops). Worse: process A observes empty, process B adds URL before A deletes clone → dangling reference (recovered by `tome catalog update` re-clone). Documented design; same profile as Phase 9 cascade pre-check. Phase 4: refcounting moved to DB table; TOCTOU semantics unchanged but now under advisory lock (FR-366) | Phase 4 F11b (DB table + lock) |
| TD-018 | `src/doctor/harness_detect.rs` (Phase 3 US4) | Harness-detected list is privacy-sensitive | Privacy | Low | Presently local-only (never transmitted); document explicitly. Review at design time if any downstream feature proposes report transmission (e.g., crash reporting, bug-filing UI). Recommend opt-in privacy gate before enabling network transmission | Local-only; monitor |
| TD-019 | Phase 4 Config struct legacy field | `Config::catalogs: BTreeMap<String, CatalogEntry>` unused post-F11b | Code cleanup | Low | F11b moved catalog enrolment to central DB; the field is never written. Excision is a follow-up cleanup PR (low urgency) | Phase 4 Polish |
| TD-020 | Error categorisation | All Phase 1 + Phase 2 + Phase 3 + Phase 4 codes are enumerated; no catch-all variants | Debuggability | Low | Current approach is sound; closed set enforces completeness | By design |
| TD-040 | Logging verbosity | Current `-v` / `-vv` mapping is fine; `TOME_LOG` env filter is undocumented | UX | Low | — | Acceptable |

### Low Priority

Nice-to-have improvements:

| ID | Area | Description | Impact | Effort |
|----|------|-------------|--------|--------|
| TD-050 | Presentation module exports | `comfy_table::Cell` + `CellAlignment` imported directly by consumers | API convenience | Low |

## Security Concerns

### High Risk

| ID | Area | Description | Risk Level | Mitigation | Status |
|----|------|-------------|------------|-----------|--------|
| SEC-001 | Phase 2 BGE testing (T088) | Vector search correctness not yet measured against real BGE models (bge-small-en-v1.5, bge-reranker-base) | High | Complete developer-machine pass with real models; validate SC-001 / SC-002 correctness assertions | Pending Phase 4+ / Developer pass |

### Medium Risk

| ID | Area | Description | Risk Level | Mitigation | Status |
|----|------|-------------|------------|-----------|--------|
| SEC-002 | Model-download UX (Phase 2–4) | User declines model-download prompt (e.g., in `tome plugin enable`) → returns exit code (ambiguous). Semantically different from system interrupt | Medium | Lock down user-decline vs. system-interrupt distinction in future iteration | Documented |
| SEC-003 | Interactive disable (Phase 5+) | User declining disable confirmation returns 0 (no error); semantically different from user-decline in other prompts | Low | Currently consistent with interactive flow semantics; monitor for UX confusion | Documented |
| SEC-010 | Credential scrubber (Phase 1–4) | Regex-based scrubbing is pattern-based, not semantic; exotic credential formats may leak (e.g., GitLab private tokens with non-standard delimiters) | Medium | Current rules (R-8 + PR #36 widening for RFC-3986 schemes + PR #54 x-amz-* params) cover common patterns. Add integration tests against real Git helper output. Monitor GitHub issues | Ongoing / Last updated Phase 3 Polish PR #54 |
| SEC-011 | Phase 4 Qwen model integrity | Qwen2.5-0.5B-Instruct GGUF SHA-256 is a placeholder sentinel in F6; real hash lands when US4 ships the model fetch | Medium | F6 registers placeholder; US4 swaps real hash; download gate enforces `has_placeholder_checksum()` rejection (exit 31) until real hash is live | Phase 4 US4 (in progress) |

### Low Risk

| ID | Area | Description | Risk Level | Mitigation | Status |
|----|------|-------------|------------|-----------|--------|
| SEC-020 | MSRV drift | Dependency updates may require MSRV bump; current MSRV is pinned (1.93) but not validated in a separate CI job | Low | MSRV CI job exists and passes; keep Renovate PRs reviewed for MSRV compatibility | CI gate in place |
| SEC-021 | Plugin identity validation (Phase 2–4) | Shape validation prevents directory traversal (`..`, `.`, `/`, etc.), but doesn't constrain character set; Unicode or non-ASCII plugin names are accepted | Low | Lenient on purpose (forward-compat); real-world risk is low. Monitor for exploit reports | Documented |

## Known Bugs

Active bugs that haven't been fixed:

| ID | Description | Workaround | Severity | Status |
|----|-------------|------------|----------|--------|
| (none documented) | — | — | — | All known issues tracked in GitHub issues; no unfixed bugs blocking shipped phases |

## Fragile Areas

Code areas that are brittle or risky to modify:

| Area | Why Fragile | Precautions |
|------|-------------|-------------|
| `src/catalog/git.rs::scrub_credentials` | Regex patterns are order-dependent; adding a rule can change ordering semantics | Add test case to `tests/scrubbing.rs` for every rule addition; verify no overlaps with existing rules. Phase 3 Polish PR #36 widened URL_LOGIN to RFC-3986 schemes; PR #54 extended to AWS presigned params |
| `src/catalog/manifest.rs::validate_source` | Path canonicalization behavior differs across platforms (symlinks, case sensitivity); one test failure can indicate subtle cross-platform issue | Test on both Linux and macOS (CI covers both); `tests/path_validation.rs` has Unix-specific symlink tests |
| `src/catalog/store.rs::write_atomic` (Phase 4 US2.d-1) | Unified atomic-write surface now used by all workspace/project/harness file writers; mode preservation + symlink refusal are critical security boundaries | Do not remove mode-preservation or symlink-refusal checks; test with `tests/security_hardening.rs` on every platform. Verify callers use this surface exclusively |
| `src/embedding/download.rs::download_model` | HTTP stream and checksum verification are separate; cleanup closure ensures both failure paths clean `.partial/` (lines 77–87) | Pipeline closure must wrap full download→verify→rename chain; any new step must be inside closure to maintain atomicity guarantee |
| `src/presentation/prompt.rs::require_terminal()` | TTY check runs on both stdin and stdout; must catch non-TTY in both dimensions to prevent prompt corruption via piped output | Always call `require_terminal()` at flow entry before any prompt; test with `Command::new()` (no pty) to verify short-circuit |
| `src/commands/plugin/{enable,disable,interactive}.rs` | Non-TTY pointer-message-then-error pattern appears at 3+ sites | Pattern consolidation would yield cleaner code; worth folding in when 4th occurrence appears |
| `src/index/skills.rs::upsert_skill` | `sqlite-vec` virtual tables do NOT support `INSERT OR REPLACE` or `ON CONFLICT` (Phase 3 latent bug fix). Uses `DELETE`-then-`INSERT` which is idempotent | Verify this pattern on any future upsert-like operation involving virtual tables; do not attempt `INSERT OR REPLACE` on `skill_embeddings` |
| `src/main.rs::--version pre-parse` | Early arg scanning before clap dispatch is custom; any change to pre-parse logic could break `--version` routing | Test both `tome --version` and `tome -V` in CLI integration tests; verify `--json` flag is also detected; check that non-matching args pass through to clap normally |
| `src/plugin/lifecycle.rs::cascade_disable_for_catalog` | Single lock acquisition per cascade; each plugin's deletion is its own transaction. TOCTOU window between pre-check (without lock) and cascade (under lock): another process may enable a plugin between check and delete | This is intentional (readers never block writers). The pre-check reports a stale but valid list; the cascade acts on what's actually there and is correct either way. Document the TOCTOU window and its benign semantics |
| `src/catalog/store.rs::reference_count` (Phase 3–4) | Reference-count read is NOT taken under advisory lock (Phase 3); moved to DB table in Phase 4 (F11b FR-366) under single advisory lock for cleanup | Phase 3 TOCTOU is intentional (readers never block writers). Phase 4 design mirrors Phase 9's cascade pre-check. TOCTOU window is documented and benign: clone persists until no references, delete is best-effort, dangling reference is recovered on next `update` |
| `src/mcp/log.rs::FileMakeWriter` | Mutex serialises every JSON log emit; LockedFile guard holds lock for duration of write. MCP server is single-threaded, so contention is theoretical | Test isolation: don't share FileMakeWriter between concurrent test threads; use separate temp log files. Production: single-threaded by design (R-2), so no contention risk |
| `src/index/migrations.rs::MIGRATIONS_OVERRIDE` (Phase 3 F7 + US5) | Public static (not `#[cfg(test)]`) so integration tests outside crate can inject synthetic migrations | Documented as test-only via doc comment. Only read by production `apply_pending` (write path already under advisory lock). Each migration runs in own transaction; failure rolls back that migration + subsequent steps don't run. Monitor: no production code should ever manually manipulate `MIGRATIONS_OVERRIDE` |
| `src/mcp/preflight.rs::verify_embedder_artefacts` (Phase 3 F8) | Runs full SHA-256 over primary embedder (~66 MB) at every startup; no caching or early exit | By design for long-running server correctness (FR-110). Cold-cache startup latency visible to harness. In test, use `StubEmbedder` to avoid real hash cost |
| `src/mcp/tools/{search_skills,get_skill}.rs::error translation` | Error translation from TomeError to structured MCP codes must be exhaustive | Test assertion in `tests/mcp_server.rs` that all tool error paths translate to specific codes; audit on every new TomeError variant. No generic fallback (FR-108) |
| `src/workspace/init.rs::init` (Phase 3 US2) | Staging directory created inside workspace root to ensure same-filesystem rename atomicity. If workspace root is not on the intended filesystem, stage-rename could silently cross mount boundary (not atomic). Crash between rename-aside and rename-in leaves `.tome.old/` orphan | Atomic staging pattern is sound: create in workspace root to guarantee same filesystem. `.tome.old/` orphans are recorded as TD-016; doctor (Phase 4+) will clean up. Test interruption scenarios thoroughly. Test rollback path on rename failure (pre-existing `.tome/`) |
| `src/mcp/tools/get_skill.rs::walk_dir` (Phase 3 US1, PR #56) | Explicit symlink skip via `is_symlink()` check (FR-S-02). Defence in depth: `lstat` does NOT follow symlinks; the skip ensures they never appear in resources list | Do not remove the symlink skip; hostile catalogs can commit `skills/foo/creds → ~/.ssh/id_rsa`. Test with `tests/security_hardening.rs` harness |
| `src/workspace/inventory.rs::read_registry` (Phase 3 US2, PR #56) | Registry validation with size cap (1 MiB), entry cap (10k), NUL rejection, `..` rejection (FR-S-03) | Caps are defensive against `cat /dev/urandom > workspaces.txt`; benign malformed entries are silently dropped. Do not lower caps without load-testing. Test injection in `tests/security_hardening.rs` |
| `src/mcp/log.rs::open_appender` (Phase 3 F8, PR #56) | MCP log file opened with explicit 0600 mode; existing files tightened on startup | chmod 0600 prevents other local users from reading workspace paths in logs. Test with `tests/security_hardening.rs` on Unix. Windows ACL model not covered (N/A) |
| `src/index/lock.rs::with_lock()` (Phase 3–4) | Advisory lockfile guards all DB writes; Phase 4 cache cleanup now under lock (F11b FR-366) | Critical: do not move any DB write or shared-resource cleanup outside the lock. Test concurrent access via `tests/concurrency.rs` (cross-process lock stress test) |
| `src/harness/rules_file.rs` (Phase 4 US1.b) | RULES.md symlink check on write-back; refuses symlinks (exit 7) | Complements Phase 3 skill symlink defence. Do not remove the check; hostile harness rules could point to system files |
| `src/harness/mcp_config.rs` (Phase 4 US1.b) | MCP config symlink check and read-modify-write via `toml_edit` (comment preservation); symlink refusal on write-back | Complements rules-file symlink defence. Do not remove the check or downgrade to non-lenient parse. Always use `toml_edit` for third-party TOML configs; plain `toml` for Tome-owned files only |
| `src/workspace/name.rs::validate_grammar` (Phase 4 US1) | Workspace names alphanumeric + underscore only; no path separators, no traversal | Grammar prevents accidental directory escape. Do not relax constraint without audit |
| `src/workspace/binding.rs::bind_project` (Phase 4 US1.a) | Project path is canonical (canonicalize must succeed) and UTF-8 (to_str check); stored as TEXT PK in `workspace_projects` | Critical: do not remove UTF-8 check (R-B1 fix). Canonicalisation failure surfaces as exit 7 (IO error). Dangerous-CWD check (`$HOME`, `/`) guards against user error | Do not relax UTF-8 or canonicalisation requirements |
| `src/util/atomic_dir.rs` (Phase 4) | Atomic populated-directory landing via staging + same-FS rename; prefix `.tome.tmp.` reserved for future doctor orphan cleanup | Precondition: target parent must exist. Staging dir is a sibling; SIGINT between keep() and rename leaves orphan. Test SIGINT scenarios; verify orphan cleanup in doctor (US5) | Phase 4 US5 doctor --fix will clean orphans |
| `src/settings/mod.rs` (Phase 4 Foundational F8) | Phase 4 introduces layered settings composition with override semantics (global + workspace + project) | New settings shapes all carry `deny_unknown_fields` (T098n verified); test round-trip through compose + override pipeline | Verify strict boundary in future additions |
| `src/workspace/rename.rs` (Phase 4 US2) | Transaction wraps marker rewrites (C-B2 fix, US2.d-1); SQL failure leaves markers pointing at `<new>` with DB at `<old>`, but DB + old markers stay consistent. Uses `toml_edit` to preserve marker fields (T-B1 fix) | Do not revert to pre-transaction or non-lenient TOML parsing. Test marker field preservation + partial-failure rollback scenarios. Monitor: SQL failure recovery is documented but partial-state is possible |
| `src/workspace/remove.rs` (Phase 4 US2) | Cascade narrowed to per-project effective harness list (C-B1 fix, US2.d-1); prevents unconditional iteration of all harness dirs | Do not revert to global `SUPPORTED_HARNESSES` iteration. Test with multi-harness projects. Verify resolver is called correctly |

## Deferred Findings from Phase 4 / US1 Review

Phase 4 / US1 audit produced 3 blockers + 25 majors. Three blockers applied in PR US1.d-2a (R-B1 UTF-8 validation, T-B1 contract amendment on env preservation, T-B2 sync idempotence test). Nine majors applied in the same PR (R-M1 atomic binding transaction, R-M2 error handling on Inline path read, R-M4 canonicalise error handling, R-M6 --global doc scrub, R-M7 --workspace override, C-N1 HarnessClash doctor pointer, R-M3 docstring downgrade, S-M3 mode preservation, T-M1/T-M4/T-M7 test additions). Remaining 16 majors deferred to follow-up or US5 doctor polish:

| ID | Category | Disposition |
|----|----------|-----------|
| C-M1 | Contract | Multi-harness mixed-style edge case; deferred to US3.c full harness matrix tests |
| C-M3 | Contract | Temp file mode 0600 vs 0644 contract; resolves via S-M3 mode-preservation fix |
| R-M5 | Rust | `bind_project` 130-line refactor (ergonomics); deferred to follow-up polish |
| S-M1 | Security | Unbounded `read_to_string` on third-party files; deferred to dedicated util helper + PR before v0.4 cut |
| S-M2 | Security | Symlink TOCTOU window; documented as benign; full closure needs `O_NOFOLLOW` open + dirfd rename; deferred to US5 hardening sweep |
| S-M4 | Security | Harness-owned parent-dir chmod 0700 on Tome-create vs respecting harness convention; design choice deferred |
| T-M2, T-M3, T-M5, T-M6, T-M8, T-M9, T-M10, T-M11 | Test | 8 test gap items rolled into "us1-test-gap-followups" tracking issue for US2/US3 polish phases |
| (30+ minors + nits) | Various | Docstring drift, redundant assertions, formatting; bulk-deferred to tracking issue |

See `specs/004-phase-4-refactor-harnesses/review/us1-disposition.md` for full triage.

## Deferred Findings from Phase 4 / US2 Review

Phase 4 / US2 audit produced 4 blockers + 23 majors. All 4 blockers applied in PR US2.d-1 (C-B1 effective-list narrowing for cascade, C-B2 transaction wrapping for rename, C-B3 JSON array bare emit for workspace list, T-B1 toml_edit for marker preservation). Eleven majors applied in US2.d-1 (C-M1/C-M2/C-M4/C-M5 contract alignments, S2-M1/S2-M2 unified atomic-write surface with mode preservation + symlink refusal, S2-M4 chmod 0o700 recovery, T-M1 JSON wire-shape pins, T-M3 cascade test coverage, T-M4/T-M5/T-M6 test gap coverage). Remaining 12 majors deferred to follow-up:

| ID | Category | Disposition |
|----|----------|-----------|
| R-M1 | Rust | SQL DISTINCT cleanup; cosmetic, deferred |
| R-M3 | Rust | Init FnOnce clone efficiency; cosmetic, deferred |
| R-M5 | Rust | Summariser lock held during invocation; performance trade-off, revisit in US4.a when LlamaSummariser ships |
| R-M6 | Rust | TOCTOU comment binding; defensive, defer to future refactor |
| R-M7 | Rust | `compute_info` early-return; cosmetic, deferred |
| R-M8 | Rust | Rename pre-check vs write TOCTOU; small surface, doctor surfaces drift, defer |
| S2-M3 | Security | Unbounded reads on workspace-owned files (settings.toml, RULES.md, config.toml); mirrors US1 S-M1 deferral; multi-site fix planned for v0.4 polish |
| T-M2 | Test | Concurrent init/rename/regen tests; pattern established, defer to test-hardening follow-up |
| (minors + nits) | Various | Docstring drift, formatting; bulk-deferred to tracking issue |

See `specs/004-phase-4-refactor-harnesses/review/us2-disposition.md` for full triage.

## Deprecated Code

Code marked for removal:

| Area | Deprecation Reason | Removal Target | Replacement |
|------|-------------------|----------------|-------------|
| `src/config.rs::Config::catalogs` field | F11b moved catalog enrolment to central DB (`index.db::catalog_entries`) | Follow-up cleanup PR | Existing: `catalog_entries` DB table |

## Performance Concerns

Known performance issues:

| ID | Area | Description | Impact | Mitigation | Status |
|----|------|-------------|--------|------------|--------|
| PERF-001 | Catalog refresh | Each `git fetch` is sequential; large catalogs block the command | Slow UX for multiple catalogs | Phase 1 spec requires sequential; parallelize in Phase 5+ with async | Deferred |
| PERF-010 | Cache validation | Manifest is re-parsed on every `show` command; no caching layer | Negligible impact (small files) | Cache not needed; revisit if manifests grow large | Acceptable |
| PERF-020 | Model download progress | Download wrapped in indeterminate spinner, not byte-progress bar | Poor visibility on large files | Phase 4 F6 defers; enhancement for Phase 4 polish or Phase 5+ (TD-010) | Tracked |
| PERF-030 | MCP pre-flight timing | SHA-256 over ~66 MB primary embedder file on every startup | Visible startup latency in cold cache | Acceptable for daemon; defer `--verify` optimization to Phase 5+ unless profiling shows impact (TD-012) | Design decision |
| PERF-040 | Doctor command latency | Catalog enumeration + harness probing on every run (non-cached) | Slower than status; expected for comprehensive diagnosis | By design: status is the narrow fast path (~200 ms); doctor is the broad slower path for troubleshooting | By design |
| PERF-050 | Phase 4 Qwen download | Large model file (~400 MB) download wrapped in indeterminate spinner (F6); byte-progress callback deferred | Poor UX visibility during first model fetch | Phase 4 F6 uses indeterminate spinner; upgrade to byte-progress in Phase 4 polish or F6 if time permits | Tracked as TD-010 |
| PERF-060 | Summariser lock overhead | `workspace regen-summary` holds advisory lock for full LlamaSummariser invocation (many seconds) | Blocks concurrent workspace operations | Acceptable for now; revisit when LlamaSummariser ships in US4.a — `tome catalog update` may need to coexist | R-M5 deferral, tracked |

## TODO Items

Active TODO comments in codebase:

| Location | TODO | Priority | Status |
|----------|------|----------|--------|
| (none found) | — | — | Code is TODOs-clean; all planned work tracked in spec and GitHub PRs |

## External Dependency Risks

Dependencies that may need attention:

| Package | Version | Concern | Action Needed | Status |
|---------|---------|---------|---------------|--------|
| `clap` | 4.x | Actively maintained; track for 5.x breaking changes | Monitor releases; plan migration before major version bump | Stable |
| `serde` | 1.x | Stable; ecosystem standard | None | Stable |
| `rusqlite` | 0.32.x (Phase 2–3) | Bundled SQLite; monitor for platform-specific build issues | Test across CI matrix | Stable |
| `sqlite-vec` | vendored (Phase 2–4) | Custom C extension vendored under `vendor/sqlite-vec/`; compiled in via `build.rs` | Compiled as part of build; no separate update cadence | Pinned |
| `fastembed-rs` | 4.x (Phase 2–4) | Wraps ONNX Runtime; size-critical dependency | Monitor for updates; test binary size on bump | Active |
| `ort` (transitive) | (Phase 2–4) | ONNX Runtime via fastembed; intrinsically large (~25 MB contribution) | Size budget already accounted for; no waivable constraint | Locked by use case |
| `tempfile` | 3.x (Phase 1–4) | Actively maintained; used for critical atomicity | Upgrade within 3.x when available; test after upgrade | Stable |
| `ctrlc` | 3.x (Phase 1–4) | Small, stable crate; signal handling is straightforward | None; review if signal handling becomes more complex | Stable |
| `regex` | 1.x (Phase 1–4) | Actively maintained; no known security issues | None | Stable |
| `reqwest` | 0.12.x (Phase 2–4) | HTTP client; used for model downloads | Monitor for TLS/security updates | Active |
| `indicatif` | 0.17.x (Phase 2–4) | Progress bar library; non-critical | Routine updates | Stable |
| `inquire` | 0.7.x (Phase 2–4) | Interactive prompts library; used in non-TTY-refusable flows | Monitor for prompt-injection or TTY-related bugs; keep up to date | Stable |
| `rmcp` | 1.x (Phase 3–4) | MCP protocol implementation; required for MCP server (US1) | Monitor for spec-alignment updates; test integration with harness | Active |
| `tokio` | 1.x (Phase 3–4, scoped) | Async runtime; used only in `src/mcp/` (structural test enforces boundary) | Constitution gate: verify tokio stays out of Phase 1–2 code paths; test async boundary quarterly | Active |
| `tracing-subscriber` | 0.3.x (Phase 3–4) | Structured logging framework; used in MCP server only | Monitor for JSON formatter updates and file I/O edge cases | Stable |
| `schemars` | 1.x (Phase 3–4) | JSON schema generation for MCP tool inputs; used at compile-time | Monitor for schema correctness issues on MCP tool definitions | Active |
| `llama-cpp-2` | 0.1.x (Phase 4, minor-pinned) | Summariser inference runtime; C++ static link | Pre-1.0 crate; monitor for API churn; test on every minor bump; CPU-only features enforced | Active / Pre-1.0 |
| `toml_edit` | 0.25.x (Phase 4, minor-pinned) | Comment-preserving TOML edits for harness config + workspace marker preservation | Monitor for breaking changes; no known security issues. Used in critical US2 marker-preservation path (T-B1 fix) | Active |

## Phase 3 Deferred Items Disposition (Research §R-17)

Per Phase 4 research §R-17, Phase 3 deferred items are dispositioned as follows:

| Item | Disposition | Target | Rationale |
|------|-------------|--------|-----------|
| **Read-only DB open refactor** (P10-deferred) | Fold into Foundational (F2) | F2 complete | Phase 4's central single DB amplifies the value; all read paths now open via `index::open_read_only`; see F1 commits |
| **MCP `Input` length caps** (P8-deferred) | Fold into US5 (doctor extensions) | US5 (in progress) | Add validation; reuse exit code 2 (`Usage`) or new variant |
| **`fabricate_models` rename** (P6-deferred) | Fold into F6 (summariser bootstrap) | F6 complete | Third fabricator (summariser) triggers rename sweep; completed in Foundational |
| **`subsystem` enum promotion** (P6-deferred at >6 arms) | Fold into US5.a (doctor extensions) | US5.a (in progress) | Phase 4 hits ~11 arms (embedder, reranker, catalogs, schema, harnesses, settings, summariser, projects, workspaces); promote to enum |
| **Drop synthetic `SuggestedFix` injection** (P7-deferred) | Fold into F9 (schema migration registration) | F9 (in progress) | Phase 4 registers first real migration; synthetic injection no longer needed for framework testing |
| **`tome workspace prune`** (P8-deferred) | Out of scope for Phase 4 | Phase 5+ | Named-workspace + central registry model makes this naturally a "remove workspace whose bound projects are gone" feature |
| **`Paths.config_file` field rename** (P8-deferred) | Drop the rename (field gone post-F2 reshape) | F2 complete | Phase 4 F2 reshapes `Paths` entirely; the historic field name no longer exists |
| **Byte-progress callback on `download_model`** (P10-deferred TD-010) | Fold into F6 or polish | F6 / Phase 4 polish | Qwen weights (~400 MB) large enough that indeterminate spinner is poor UX; tracked as TD-010 |
| **M-MCP-3 / M-MCP-11 / m-WKS-*** (P8-deferred) | Fold into Polish (same pattern as Phase 3) | Phase 4 Polish | Coverage gaps in MCP + workspace long-tail edge cases |
| **T088 manual SC-001 / SC-002** (P10-deferred) | Out of scope for Phase 4 | Phase 5+ / Developer pass | Needs real BGE models for verification; tracked as SEC-001 HIGH RISK |
| **T093/T094/T095 MCP integration tests** (P8-deferred) | Out of scope for Phase 4 (unless TD-014 seed refactor cheap) | Phase 5+ | Requires TD-014 `McpState` seed exposure or test fixture enhancement |

---

## Concern Severity Guide

| Level | Definition | Response Time |
|-------|------------|----------------|
| Critical | Production impact, security breach, test failure blocking ship | Immediate |
| High | Degraded functionality, security risk, blocking feature | This sprint |
| Medium | Developer experience, minor functional issues, UX confusion | Next sprint |
| Low | Nice to have, enhancement, cosmetic | Backlog |

---

*This document tracks what needs attention. Update when concerns are resolved or discovered.*
