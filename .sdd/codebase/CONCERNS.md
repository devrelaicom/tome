# Known Concerns

> **Purpose**: Document technical debt, known risks, bugs, fragile areas, and improvement opportunities.
> **Generated**: 2026-05-26
> **Last Updated**: 2026-05-26 (Phase 4 / US4 completed; all 4 blockers + 9 majors applied in PR #97)

## Technical Debt

### High Priority

Items that should be addressed in current or near-term phases:

| ID | Area | Description | Impact | Effort | Notes |
|----|------|-------------|--------|--------|-------|
| TD-001 | `src/index/` (Phase 2–4) | Advisory locking for concurrent catalog/index access | Concurrency safety | High | Phase 3 MCP server + Phase 4 central DB exposes concurrent access; advisory lockfile (FR-040, F11b FR-366) is implemented; harness use/remove + workspace operations now under advisory lock (US3 PR #92); T088 (real BGE testing) is the verification gap |
| TD-003 | Binary size (Phase 2–4) | SQLite + ONNX + llama.cpp pushed binary to ~30 MB; cap is 50 MB | Headroom management | Medium | Phase 4 projection: ~28.4 MB macOS arm64, ~34 MB Linux x86_64 (research §R-4); discipline holds with 16+ MB headroom; US4 added llama-cpp-2 (~1.5 MB) |

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
| TD-050-US4 | `src/summarise/` (Phase 4 US4) | Byte-progress callback for ~400 MB Qwen model download (TD-010 amplified for summariser) | UX | Low | Phase 4 F6 uses indeterminate spinner; upgrade to byte-progress callback in Phase 4 polish if time permits | Phase 4 Polish tracking |
| TD-051-US4 | `src/summarise/llama.rs` | `LlamaSummariser::new` verifies SHA-256 + loads model on every fresh instantiation (no per-process caching across multiple `new()` calls) | Startup latency (rare) | Low | Per-instantiation load is correct for type semantics; process-global singleton `LlamaBackend` is cached. If a future feature creates multiple `LlamaSummariser` instances in sequence without reuse, per-instance load overhead would be measurable. Current triggers create one per regen call; acceptable | Acceptable design |

### Low Priority

Nice-to-have improvements:

| ID | Area | Description | Impact | Effort |
|----|------|-------------|--------|--------|
| TD-060 | Presentation module exports | `comfy_table::Cell` + `CellAlignment` imported directly by consumers | API convenience | Low |

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
| SEC-011 | Phase 4 US4 Qwen model integrity (RESOLVED) | Qwen2.5-0.5B-Instruct GGUF SHA-256 was a placeholder sentinel in F6; real hash landed in US4.d-1 PR #97 (C-B1) | **RESOLVED** | F6 shipped placeholder; US4.d-1 C-B1 swapped real hash `74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db` verified 2026-05-26; S-M3 belt-and-braces gate prevents regression; T-guard via `tests/summariser_registry_no_placeholder.rs` | ✅ Resolved PR #97 |
| SEC-012-US4 | Phase 4 US4 summariser prompt injection (NEW) | Workspace's enabled-plugin descriptions + skill names interpolated into LLM prompts; descriptions from third-party `plugin.json` (lenient parse) | Medium | Trust boundary: documented as acceptable per "user authored which plugins are enabled" posture; S-M1 deferral — documentation only, no code change needed. Production prompt construction avoids shell/code-generation context; length-capped outputs + non-empty validation | By design / Documented |
| SEC-013-US4 | Phase 4 US4 MCP tool description broadcast (NEW) | Cached workspace short summary embedded in MCP tool description broadcast to every connected harness client | Medium | Read once at startup; in-memory cached until `LlamaSummariser` drops; summary length-capped (SHORT: 800 chars per FR-425) + non-empty validated. Do not include sensitive data in plugin descriptions (governance boundary) | By design / Documented |

### Low Risk

| ID | Area | Description | Risk Level | Mitigation | Status |
|----|------|-------------|------------|-----------|--------|
| SEC-020 | MSRV drift | Dependency updates may require MSRV bump; current MSRV is pinned (1.93) but not validated in a separate CI job | Low | MSRV CI job exists and passes; keep Renovate PRs reviewed for MSRV compatibility | CI gate in place |
| SEC-021 | Plugin identity validation (Phase 2–4) | Shape validation prevents directory traversal (`..`, `.`, `/`, etc.), but doesn't constrain character set; Unicode or non-ASCII plugin names are accepted | Low | Lenient on purpose (forward-compat); real-world risk is low. Monitor for exploit reports | Documented |
| SEC-022-US4 | LLM model dependency (NEW) | `llama-cpp-2` pre-1.0 crate; upstream llama.cpp evolves frequently | Low | Pin llama-cpp-2 minor version (0.1.x); test on every minor bump; CPU-only features enforced (no CUDA/Metal). Monitor upstream for security advisories | Active monitoring |

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
| `src/catalog/store.rs::write_atomic` (Phase 4 US2.d-1 + US4) | Unified atomic-write surface now used by all workspace/project/harness/settings file writers; mode preservation + symlink refusal are critical security boundaries | Do not remove mode-preservation or symlink-refusal checks; test with `tests/security_hardening.rs` on every platform. Verify callers use this surface exclusively |
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
| `src/commands/harness/mod.rs::CentralDbScopeProvider` (Phase 4 US3, PR #92) | Production scope provider now consults central DB for workspace membership; three-way classification (not registered → UnknownWorkspace, exists without settings → Ok(None), exists with settings → Ok(Some)) | Critical fix: production sync previously used `StubScope::new()` always returning UnknownWorkspace. Central DB check must succeed before settings-file read. Test all three paths explicitly. Do not revert to `PathsScopeProvider` or any provider that masks IO/parse errors as UnknownWorkspace |
| `src/commands/harness/{use_,remove}.rs` (Phase 4 US3, PR #92) | Advisory lock held across entire read-modify-write window (US3 C-M5 fix, S-M2 security fix) | Critical: both commands now acquire `index.lock` before any settings-file access. Lock must be held until sync completes or no-op is confirmed. Race-safe concurrent edits rely on this lock. Do not move lock acquisition or shorten lock hold window without full concurrency audit |
| `src/settings/edit.rs::save_settings` (Phase 4 US3, PR #92) | Routes through `write_atomic` for unified mode preservation + symlink refusal. Tested in `tests/security_hardening.rs::preserve_file_mode_on_workspace_settings_via_settings_edit` + `refuses_symlink_on_settings_edit` | Critical: settings file writes must use this path to inherit security boundary (mode preservation, symlink refusal). Do not bypass write_atomic. Regressions are caught by security_hardening tests |
| `tests/common/mod.rs::HOME_MUTEX + HomeGuard` (Phase 4 US3, PR #92) | Process-global `Mutex<()>` serialises HOME mutations across parallel tests; RAII guard restores HOME on drop before releasing mutex | Critical: all harness tests using `std::env::set_var("HOME", ...)` must acquire mutex and use HomeGuard. Parallel tests no longer collide on HOME. Do not bypass mutex or use raw env::set_var without guard |
| `src/summarise/llama.rs::LlamaSummariser` (Phase 4 US4) | Loads Qwen2.5-0.5B-Instruct GGUF via llama-cpp-2; caches `LlamaModel` on `self` after SHA-256 verify + load; per-call `LlamaContext` (US4.d-1 S-M4) | Do not remove SHA-256 verify before load (S-M3); do not remove placeholder gate (belt-and-braces C-B1 regression protection). Model caching avoids re-hash per trigger. Test with `tests/summariser_real.rs` + length-window warn via `tests/workspace_regen_summary.rs` |
| `src/summarise/mod.rs::backend() + backend_poison recovery` (Phase 4 US4) | Process-wide `LlamaBackend` singleton initialized lazily; init is guarded by Mutex; poison recovery via `PoisonError::into_inner` instead of error bubble (US4.d-1 R-M7) | Poison recovery is intentional: init lock guards only one-shot init; cross-thread panic shouldn't permanently disable summarisation on later callers. Do not add .expect() after mutex lock acquisition. Test via `tests/summariser_real.rs` |
| `src/workspace/regen_summary.rs` (Phase 4 US4) | Generates short + long summaries via `LlamaSummariser::summarise`; length-window warn emitted at 800/2500 char thresholds; values cached in `settings.toml` `[summaries]` section; RULES.md rewritten atomically | Do not remove length-window enforcement (FR-425); do not relax to non-atomic write. Test warn via custom `tracing-subscriber::Layer` in `tests/workspace_regen_summary.rs`. Verify regen happens inside advisory lock (other sections preserved via `toml_edit::DocumentMut`) |
| `src/commands/catalog/remove.rs::cascade_after_disable` (Phase 4 US4, PR #97 R-M6) | `tome catalog remove --force` now calls `regenerate_for_trigger` after cascade-disable completes (outside advisory lock; regen takes its own) | Do not move regenerate inside the cascade lock (would block concurrent operations unnecessarily). Mirrors plugin-disable pattern. Test via integration tests (catalog_remove_cascade extends to cover regen trigger) |
| `src/embedding/registry.rs + src/summarise/registry.rs` (Phase 4 US4) | Both registries carry model metadata + checksums; `SUMMARISER_SHA256` / `SUMMARISER_SIZE_BYTES` pinned to real Qwen hash (US4.d-1 C-B1); test guards prevent drift between the two sources | Critical: do not update one source without updating the other. `tests/summariser_registry_no_placeholder.rs` catches placeholder regressions. On model bump, verify real hash before pinning; do not use placeholders in production. Test via `tests/summariser_registry_no_placeholder.rs` (3 tests) |

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
|----|----------|--------|
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

## Deferred Findings from Phase 4 / US3 Review (Resolved in PR #92)

Phase 4 / US3 audit produced **6 blockers + 29 majors**. **All 6 blockers + 12 selected majors resolved in PR #92** (US3.d-1):

### Blockers Applied

| # | Category | Fix | Status |
|---|----------|-----|--------|
| **C-B1 + S-M1** | Critical production bypass | `StubScope::new()` replaced by `CentralDbScopeProvider` in `harness::sync::sync_project` | ✅ Resolved PR #92 |
| **C-B2 + S-M4 + R-M1** | Provider masking errors | `PathsScopeProvider` → `CentralDbScopeProvider` distinguishes workspace-not-found vs. IO errors | ✅ Resolved PR #92 |
| **C-B3** | Info command incomplete | `harness info` now computes effective list and reports source chain | ✅ Resolved PR #92 |
| **T-B1** | Test race condition | `HOME_MUTEX` + `HomeGuard` serialises parallel `std::env::set_var("HOME", ...)` calls | ✅ Resolved PR #92 |
| **T-B2** | Test output never verified | Command run functions refactored to take `out: &mut dyn Write` sink; tests now assert on output | ✅ Resolved PR #92 |
| **T-B3** | Test override slot races | Per-file `OVERRIDE_MUTEX` added to `harness_skeleton.rs` | ✅ Resolved PR #92 |

### Majors Applied (12/29)

| # | Category | Fix | Status |
|---|----------|-----|--------|
| **C-M1** | Contract gap | `EffectiveHarness.source_chain` extended to carry reference strings (`[workspaces.shared]`) | ✅ Resolved PR #92 |
| **C-M2** | DB membership check missing | `harness list <workspace>` validates central DB membership; exits 13 on unknown | ✅ Resolved PR #92 |
| **C-M4 + T-M8** | Per-entry validation | Unsupported harness validation moved inside `resolve_list` after parsing each `Include` | ✅ Resolved PR #92 |
| **C-M5 + R-M2 + S-M2** | Concurrent write race | Advisory lock around `harness use\|remove` read-modify-write window | ✅ Resolved PR #92 |
| **C-M6** | Multi-block edge case | `write_rules_for_path` correctly handles multi-block-collapse case | ✅ Resolved PR #92 |
| **R-M3** | Error context wrong | `WorkspaceRefOutsideProject` uses actual scope where `[workspace]` found | ✅ Resolved PR #92 |
| **R-M8** | Malformed parse fallthrough | `CompositionRef::parse` rejects malformed bracketed refs | ✅ Resolved PR #92 |
| **T-M1** | FR-447 resolver path untested | New test: `[workspaces.unknown]` → exit 13 via resolver | ✅ Resolved PR #92 |
| **T-M2** | JSON shape unpinned | Added JSON byte-stability tests for `HarnessBareEntry`, `HarnessInfoOutcome`, etc. | ✅ Resolved PR #92 |
| **T-M3** | `--force` path untested | New test: `tome harness use --force` exercises FR-502 wiring | ✅ Resolved PR #92 |
| **T-M5** | CLI thin-wrapper untested | New test: `tome harness sync` against missing project marker → exit 2 | ✅ Resolved PR #92 |
| **S-M3** | Settings-edit security untested | New tests in `security_hardening.rs`: mode preservation + symlink refusal for `save_settings` | ✅ Resolved PR #92 |

### Majors Deferred (17/29)

| # | Category | Reason | Target |
|----|----------|--------|--------|
| C-M3 | Contract | Bare's MCP-config heuristic — cosmetic edge case | Tracking issue |
| C-M7 | Contract | Informational notice differentiation — cosmetic wording | Tracking issue |
| C-M8 | Contract | Cycle path scope-kind prefix — depends on C-M1 fix (now resolved); revisit naturally | Follow-up |
| R-M4 | Rust | Post-edit re-snapshot error masks drift — defensive concern | Tracking issue |
| R-M5 | Rust | Redundant `home_root()` calls — perf cosmetic | Tracking issue |
| R-M6 | Rust | Typed enum for direct_scopes — superseded by C-M1 + C-B3 fixes | Tracking issue |
| R-M7 | Rust | Dead `_for_use` aliases — cleanup | Tracking issue |
| T-M4 | Test | Informational-notice path — pairs with C-M7 deferral | Tracking issue |
| T-M6 | Test | `remove_last_entry_leaves_empty_array` substring vs. parse — defer tightening | Tracking issue |
| T-M7 | Test | Cycle-through-`[global]` — small gap | Tracking issue |
| T-M9 | Test | Idempotence extends to rules/MCP mtimes — extends pattern | Tracking issue |
| (minors) | Various | Docstring drift, redundant comments, formatting | Tracking issue |

See `specs/004-phase-4-refactor-harnesses/review/us3-disposition.md` for full triage.

## Deferred Findings from Phase 4 / US4 Review (Resolved in PR #97)

Phase 4 / US4 audit produced **5 blockers + 21 majors**. **All 4 actionable blockers + 9 selected majors resolved in PR #97** (US4.d-1):

### Blockers Applied

| # | Category | Fix | Status |
|---|----------|-----|--------|
| **C-B1** | Placeholder hash regression | Real Qwen2.5-0.5B-Instruct SHA-256 `74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db` (verified 2026-05-26) pinned in both `src/embedding/registry.rs` + `src/summarise/registry.rs`; size 491,400,032 bytes | ✅ Resolved PR #97 |
| **C-B2** | Exit code wrong | Scrubbed stale "exit 20" refs; all summariser failures now route to closed-enum exit 24 (`TomeError::SummariserFailure`) | ✅ Resolved PR #97 |
| **C-B3 / R-B1** | Length-window drift | Unified `SHORT_MAX_CHARS` (800) + `LONG_MAX_CHARS` (2500) in `src/summarise/mod.rs`; eliminated 100-char drift (long was 2400 in prompts.rs vs 2500 in regen_summary); LONG prompt bumped to match | ✅ Resolved PR #97 |
| **T-B1** | Trigger coverage zero | Added `tests/summariser_triggers_end_to_end.rs` (2 tests) + `tests/summariser_triggers.rs::model_missing_trigger_is_silent_noop` exercising override slot + trigger paths | ✅ Resolved PR #97 |

### Majors Applied (9/21)

| # | Category | Fix | Status |
|---|----------|-----|--------|
| **C-M1** | Prompt format | `format_input_descriptions` no longer prefixes lines with `"- "`; removed contradictory bullet pattern from SHORT prompt | ✅ Resolved PR #97 |
| **C-M2** | Silent no-op semantics | Contract `summariser.md` now documents "ModelMissing" silent-no-op for trigger callers vs. hard-fail for `regen-summary` (FR-420 corollary) | ✅ Resolved PR #97 |
| **R-M2** | Registry lookup | `LlamaSummariser::new` uses `summariser_entry()` instead of inline find | ✅ Resolved PR #97 |
| **R-M6** | Cascade consistency | `tome catalog remove --force` now calls `regenerate_for_trigger` post-cascade (mirrors plugin-disable pattern) | ✅ Resolved PR #97 |
| **R-M7** | Mutex poison | `backend()` recovers via `PoisonError::into_inner` instead of error bubble | ✅ Resolved PR #97 |
| **S-M3** | Placeholder gate | `LlamaSummariser::new` belt-and-braces rejects all-zero placeholder with `ModelMissing` (S-M3) | ✅ Resolved PR #97 |
| **S-M4** | Model caching | `LlamaSummariser` caches loaded `LlamaModel` on `self`; per-call `LlamaContext` eliminates 400 MB per-trigger re-hash | ✅ Resolved PR #97 |
| **T-M2** | Silent no-op test | `tests/summariser_triggers.rs::model_missing_trigger_is_silent_noop` verifies trigger carve-out | ✅ Resolved PR #97 |
| **T-M5** | Length-window test | `tests/workspace_regen_summary.rs::regen_summary_long_window_emits_warn_via_layer` captures warn via custom tracing Layer | ✅ Resolved PR #97 |

### Majors Deferred (12/21)

| # | Category | Reason | Target |
|----|----------|--------|--------|
| C-M3 | Contract | Qwen context-window size rationale comment; cosmetic, doc-only | Tracking issue |
| C-M4 | Contract | Reserved token budget discrepancy (plan vs. actual); revisit if Qwen bump planned | Tracking issue |
| C-M5 | Contract | Top-p vs. temp interaction semantics in SHORT mode; defensive, established | Tracking issue |
| R-M1 | Rust | Qwen URL const dedup (appears in registry + download); cosmetic, defer | Tracking issue |
| R-M3 | Rust | Llama-cpp-2 error messages not scrubbed in logs; defer to v0.4 credential scrubber sweep (S-M1 + S2-M3 parallel) | Tracking issue |
| R-M4 | Rust | `LlamaSampler::chain_simple` sampling order tuning; defer unless perf profiling shows impact | Tracking issue |
| R-M5 | Rust | `LlamaContext` KV cache size tuning (4096 tokens); acceptable; defer to profiling | Tracking issue |
| T-M1, T-M3, T-M4, T-M6, T-M7, T-M8 | Test | 6 test edge cases (corner inputs, cross-model consistency, output stability); deferred to v0.4 | Tracking issue |
| (minors) | Various | Docstring cleanup, comment consistency | Tracking issue |

See `specs/004-phase-4-refactor-harnesses/review/us4-disposition.md` + `us4-findings.md` for full triage.

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
| PERF-050 | Phase 4 Qwen download | Large model file (~400 MB) download wrapped in indeterminate spinner (F6); byte-progress callback deferred | Poor UX visibility during first model fetch | Phase 4 F6 uses indeterminate spinner; upgrade to byte-progress in Phase 4 polish or F6 if time permits (TD-010 / TD-050-US4) | Tracked |
| PERF-060 | Summariser lock overhead | `workspace regen-summary` holds advisory lock for full LlamaSummariser invocation (many seconds) | Blocks concurrent workspace operations | Acceptable for now; revisit when LlamaSummariser ships in US4.a — `tome catalog update` may need to coexist | R-M5 deferral, tracked |
| PERF-070-US4 | Summariser model load (US4.d-1 S-M4 fix) | Model load + caching per `LlamaSummariser` instance; per-call context | Eliminated: S-M4 removed ~400 MB per-trigger re-hash by caching model on `self` | Caching verified; `LlamaModel: Send + Sync` upstream holds `Summariser: Send + Sync` bound | ✅ Resolved PR #97 |

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
| `llama-cpp-2` | 0.1.x (Phase 4, minor-pinned) | Summariser inference runtime; C++ static link | Pre-1.0 crate; monitor for API churn; test on every minor bump; CPU-only features enforced; US4 first production use (C-B1 real hash) | Active / Pre-1.0 |
| `toml_edit` | 0.25.x (Phase 4, minor-pinned) | Comment-preserving TOML edits for harness config + workspace marker preservation | Monitor for breaking changes; no known security issues. Used in critical US2 marker-preservation path (T-B1 fix) and US3 settings-edit (S-M3 fix) | Active |

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
| **Byte-progress callback on `download_model`** (P10-deferred TD-010) | Fold into F6 or polish | F6 / Phase 4 polish | Qwen weights (~400 MB) large enough that indeterminate spinner is poor UX; tracked as TD-010 / TD-050-US4 |
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
