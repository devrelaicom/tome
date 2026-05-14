# Known Concerns

> **Purpose**: Document technical debt, known risks, bugs, fragile areas, and improvement opportunities.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-14 (Phase 3 complete: US1–US5 shipped; PR #56 security hardening applied)

## Technical Debt

### High Priority

Items that should be addressed in current or near-term phases:

| ID | Area | Description | Impact | Effort | Notes |
|----|------|-------------|--------|--------|-------|
| TD-001 | `src/index/` (Phase 2) | Advisory locking for concurrent catalog/index access | Concurrency safety | High | Phase 3 MCP server exposes concurrent harness access; advisory lockfile (FR-040) is implemented; T088 (real BGE testing) is the verification gap |
| TD-003 | Binary size (Phase 2/3) | SQLite + ONNX pushed binary to 29.56 MB; cap revised to 50 MB (CONSTITUTION v1.2.0) | Headroom management | Medium | 22 MiB on macOS arm64 (28 MiB headroom remains); discipline holds |

### Medium Priority

Items to address when working in the area:

| ID | Area | Description | Impact | Effort | Mitigation |
|----|------|-------------|--------|--------|-----------|
| TD-010 | `src/embedding/download.rs` | No byte-progress callback for model downloads | UX | Low | Currently wrapped in indeterminate spinner in both `plugin enable` and `models download`; enhancement for polish pass. Also applies to `tome reindex` progress visibility (Phase 7) |
| TD-011 | `src/index/migrations.rs` (Phase 3 F7 + US5) | Schema-migration framework implementation complete; zero registered migrations shipped | Testing coverage | Low | Framework landed in Phase 3 Foundational F7 + US5. Phase 4+ adds first real `Migration` rows + e2e test via `MIGRATIONS_OVERRIDE` injection |
| TD-012 | `src/mcp/preflight.rs` (Phase 3 F8) | MCP startup pre-flight runs SHA-256 over primary embedder (~66 MB) at every startup | Startup latency | Low | Acceptable for long-running server; cold-cache startup may see latency. Consider `--verify` flag on `tome status` to skip SHA-256 on non-suspect runs. Defer unless profiling shows impact |
| TD-013 | Phase 3 US1 testing (T088, T093–T095) | Manual verification pending: real BGE models + live harness for SC-001/SC-002 coverage | Integration testing | High | Three categories: (1) happy-path search_skills/get_skill returns (T092 partial via `mcp_server.rs`), (2) MCP protocol purity (T093), (3) latency budget (T094 p50<300ms, p99<600ms), (4) SIGINT graceful shutdown (T095). Tracked in `retro/P3.md`. **Status**: T088 deferred pending developer access to real BGE models for live container/harness testing |
| TD-014 | `src/mcp/state.rs` (Phase 3 F8) | McpState embedder/reranker seed exposure for test integration | Test isolation | Medium | Handlers derive seeds from `state.embedder_entry.name/version`, hard-coded to MODEL_REGISTRY entries. Tests can't bootstrap index with stub seeds + use handlers without tripping drift detection. Refactor `McpState` to carry `embedder_seed` / `reranker_seed` directly. Est. 1 hour, defer to post-US1 |
| TD-015 | Error code documentation drift | Contract vs. production code discrepancy on "Index DB missing" | Documentation | Low | Contract lists exit 35 for "Index DB missing" but production surfaces exit 60 (`McpStartupFailed`). Amend contract in polish (already done in Phase 3 Polish PR #54) |
| TD-016 | `src/workspace/init.rs` (Phase 3 US2) | `.tome.old/` orphan cleanup on crash between rename-aside and rename-in | Recovery cleanup | Low | If `--force` rename-in fails after moving old `.tome/` to `.tome.old/`, rollback restores the old state. But if a crash occurs between rename-aside and rename-in (before rollback logic), `.tome.old/` is left orphan. Doctor (Phase 4+, out of scope for US2) should surface and offer cleanup. Currently documented in contract as a known limitation (FR-M-WKS-2) |
| TD-017 | `src/catalog/store.rs::reference_count` (Phase 3 US3) | Catalog cache TOCTOU window between pre-check and `remove_dir_all` | Concurrency safety | Low | Two processes racing `tome catalog remove` may both observe empty refs and both call `remove_dir_all` (benign: one wins, one no-ops). Worse: process A observes empty, process B adds URL before A deletes clone → dangling reference (recovered by `tome catalog update` re-clone). Documented design; same profile as Phase 9 cascade pre-check. No lock taken because readers shouldn't block writers |
| TD-018 | `src/doctor/harness_detect.rs` (Phase 3 US4) | Harness-detected list is privacy-sensitive | Privacy | Low | Presently local-only (never transmitted); document explicitly. Review at design time if any downstream feature proposes report transmission (e.g., crash reporting, bug-filing UI). Recommend opt-in privacy gate before enabling network transmission |
| TD-020 | Error categorisation | All Phase 1 + Phase 2 + Phase 3 codes are enumerated; no catch-all variants | Debuggability | Low | Current approach is sound; closed set enforces completeness |
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
| SEC-001 | Phase 2 BGE testing (T088) | Vector search correctness not yet measured against real BGE models (bge-small-en-v1.5, bge-reranker-base) | High | Complete developer-machine pass with real models; validate SC-001 / SC-002 correctness assertions | Pending Phase 3 / Developer pass |

### Medium Risk

| ID | Area | Description | Risk Level | Mitigation | Status |
|----|------|-------------|------------|-----------|--------|
| SEC-002 | Model-download UX (Phase 2+) | User declines model-download prompt (e.g., in `tome plugin enable`) → returns exit code (ambiguous). Semantically different from system interrupt | Medium | Lock down user-decline vs. system-interrupt distinction in future iteration | Documented |
| SEC-003 | Interactive disable (Phase 5) | User declining disable confirmation returns 0 (no error); semantically different from user-decline in other prompts | Low | Currently consistent with interactive flow semantics; monitor for UX confusion | Documented |
| SEC-010 | Credential scrubber (Phase 1–3) | Regex-based scrubbing is pattern-based, not semantic; exotic credential formats may leak (e.g., GitLab private tokens with non-standard delimiters) | Medium | Current rules (R-8 + PR #36 widening for RFC-3986 schemes + PR #54 x-amz-* params) cover common patterns. Add integration tests against real Git helper output. Monitor GitHub issues | Ongoing / Last updated Phase 3 Polish PR #54 |

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
| `src/catalog/git.rs::scrub_credentials` | Regex patterns are order-dependent; adding a rule can change ordering semantics | Add test case to `tests/scrubbing.rs` for every rule addition; verify no overlaps with existing rules. Phase 3 Polish PR #36 widened URL_LOGIN to RFC-3986 schemes; PR #54 extended to AWS presigned params |
| `src/catalog/manifest.rs::validate_source` | Path canonicalization behavior differs across platforms (symlinks, case sensitivity); one test failure can indicate subtle cross-platform issue | Test on both Linux and macOS (CI covers both); `tests/path_validation.rs` has Unix-specific symlink tests |
| `src/catalog/store.rs::write_atomic` | Atomic rename only works on same filesystem; moving across mounts silently falls back to non-atomic copy | Document assumption in code; consider detecting mount boundary and erroring explicitly |
| `src/embedding/download.rs::download_model` | HTTP stream and checksum verification are separate; cleanup closure ensures both failure paths clean `.partial/` (lines 77–87) | Pipeline closure must wrap full download→verify→rename chain; any new step must be inside closure to maintain atomicity guarantee |
| `src/presentation/prompt.rs::require_terminal()` | TTY check runs on both stdin and stdout; must catch non-TTY in both dimensions to prevent prompt corruption via piped output | Always call `require_terminal()` at flow entry before any prompt; test with `Command::new()` (no pty) to verify short-circuit |
| `src/commands/plugin/{enable,disable,interactive}.rs` | Non-TTY pointer-message-then-error pattern appears at 3+ sites | Pattern consolidation would yield cleaner code; worth folding in when 4th occurrence appears |
| `src/index/skills.rs::upsert_skill` | `sqlite-vec` virtual tables do NOT support `INSERT OR REPLACE` or `ON CONFLICT` (Phase 7 latent bug fix). Uses `DELETE`-then-`INSERT` which is idempotent | Verify this pattern on any future upsert-like operation involving virtual tables; do not attempt `INSERT OR REPLACE` on `skill_embeddings` |
| `src/main.rs::--version pre-parse` | Early arg scanning before clap dispatch is custom; any change to pre-parse logic could break `--version` routing | Test both `tome --version` and `tome -V` in CLI integration tests; verify `--json` flag is also detected; check that non-matching args pass through to clap normally |
| `src/plugin/lifecycle.rs::cascade_disable_for_catalog` | Single lock acquisition per cascade; each plugin's deletion is its own transaction. TOCTOU window between pre-check (without lock) and cascade (under lock): another process may enable a plugin between check and delete | This is intentional (readers never block writers). The pre-check reports a stale but valid list; the cascade acts on what's actually there and is correct either way. Document the TOCTOU window and its benign semantics |
| `src/catalog/store.rs::reference_count` (Phase 3 US3) | Reference-count read is NOT taken under advisory lock | This is intentional (readers never block writers). Design mirrors Phase 9's cascade pre-check. TOCTOU window is documented and benign: clone persists until no references, delete is best-effort, dangling reference is recovered on next `update` |
| `src/mcp/log.rs::FileMakeWriter` | Mutex serialises every JSON log emit; LockedFile guard holds lock for duration of write. MCP server is single-threaded, so contention is theoretical | Test isolation: don't share FileMakeWriter between concurrent test threads; use separate temp log files. Production: single-threaded by design (R-2), so no contention risk |
| `src/index/migrations.rs::MIGRATIONS_OVERRIDE` (Phase 3 US5) | Public static (not `#[cfg(test)]`) so integration tests outside crate can inject synthetic migrations | Documented as test-only via doc comment. Only read by production `apply_pending` (write path already under advisory lock). Each migration runs in own transaction; failure rolls back that migration + subsequent steps don't run. Monitor: no production code should ever manually manipulate `MIGRATIONS_OVERRIDE` |
| `src/mcp/preflight.rs::verify_embedder_artefacts` (Phase 3 F8) | Runs full SHA-256 over primary embedder (~66 MB) at every startup; no caching or early exit | By design for long-running server correctness (FR-110). Cold-cache startup latency visible to harness. In test, use `StubEmbedder` to avoid real hash cost |
| `src/mcp/tools/{search_skills,get_skill}.rs::error translation` | Error translation from TomeError to structured MCP codes must be exhaustive | Test assertion in `tests/mcp_server.rs` that all tool error paths translate to specific codes; audit on every new TomeError variant. No generic fallback (FR-108) |
| `src/workspace/init.rs::init` (Phase 3 US2) | Staging directory created inside workspace root to ensure same-filesystem rename atomicity. If workspace root is not on the intended filesystem, stage-rename could silently cross mount boundary (not atomic). Crash between rename-aside and rename-in leaves `.tome.old/` orphan | Atomic staging pattern is sound: create in workspace root to guarantee same filesystem. `.tome.old/` orphans are recorded as TD-016; doctor (Phase 4+) will clean up. Test interruption scenarios thoroughly. Test rollback path on rename failure (pre-existing `.tome/` with `--force`) |
| `src/mcp/tools/get_skill.rs::walk_dir` (Phase 3 US1, PR #56) | Explicit symlink skip via `is_symlink()` check (FR-S-02). Defence in depth: `lstat` does NOT follow symlinks; the skip ensures they never appear in resources list | Do not remove the symlink skip; hostile catalogs can commit `skills/foo/creds → ~/.ssh/id_rsa`. Test with `tests/security_hardening.rs` harness |
| `src/workspace/inventory.rs::read_registry` (Phase 3 US2, PR #56) | Registry validation with size cap (1 MiB), entry cap (10k), NUL rejection, `..` rejection (FR-S-03) | Caps are defensive against `cat /dev/urandom > workspaces.txt`; benign malformed entries are silently dropped. Do not lower caps without load-testing. Test injection in `tests/security_hardening.rs` |
| `src/mcp/log.rs::open_appender` (Phase 3 F8, PR #56) | MCP log file opened with explicit 0600 mode; existing files tightened on startup | chmod 0600 prevents other local users from reading workspace paths in logs. Test with `tests/security_hardening.rs` on Unix. Windows ACL model not covered (N/A) |

## Deprecated Code

Code marked for removal:

| Area | Deprecation Reason | Removal Target | Replacement |
|------|-------------------|----------------|-------------|
| (none) | — | — | — |

All Phase 1, Phase 2, Phase 3 Foundational, Phase 3 User Stories 1–5, and Phase 3 Polish code is current; no legacy to remove yet.

## Performance Concerns

Known performance issues:

| ID | Area | Description | Impact | Mitigation | Status |
|----|------|-------------|--------|------------|--------|
| PERF-001 | Catalog refresh | Each `git fetch` is sequential; large catalogs block the command | Slow UX for multiple catalogs | Phase 1 spec requires sequential; parallelize in Phase 4+ with async | Deferred |
| PERF-010 | Cache validation | Manifest is re-parsed on every `show` command; no caching layer | Negligible impact (small files) | Cache not needed; revisit if manifests grow large | Acceptable |
| PERF-020 | Model download progress | Download wrapped in indeterminate spinner, not byte-progress bar | Poor visibility on large files | Enhancement for Phase 3 polish (TD-010). Phase 7 `tome reindex` also lacks per-skill progress visibility | Tracked |
| PERF-030 | MCP pre-flight timing | SHA-256 over ~66 MB primary embedder file on every startup | Visible startup latency in cold cache | Acceptable for daemon; defer `--verify` optimization to Phase 4+ unless profiling shows impact (TD-012) | Design decision |
| PERF-040 | Doctor command latency | Catalog enumeration + harness probing on every run (non-cached) | Slower than status; expected for comprehensive diagnosis | By design: status is the narrow fast path (~200 ms); doctor is the broad slower path for troubleshooting | By design |

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
| `rusqlite` | 0.31.x (Phase 2) | Bundled SQLite; monitor for platform-specific build issues | Test across CI matrix | Stable |
| `sqlite-vec` | vendored (Phase 2) | Custom C extension vendored under `vendor/sqlite-vec/`; compiled in via `build.rs` | Compiled as part of build; no separate update cadence | Pinned |
| `fastembed-rs` | (Phase 2) | Wraps ONNX Runtime; size-critical dependency | Monitor for updates; test binary size on bump | Active |
| `ort` (transitive) | (Phase 2) | ONNX Runtime via fastembed; intrinsically large (~25 MB contribution) | Size budget already accounted for; no waivable constraint | Locked by use case |
| `tempfile` | 3.x | Actively maintained; used for critical atomicity | Upgrade within 3.x when available; test after upgrade | Stable |
| `ctrlc` | 3.x | Small, stable crate; signal handling is straightforward | None; review if signal handling becomes more complex | Stable |
| `regex` | 1.x | Actively maintained; no known security issues | None | Stable |
| `reqwest` | 0.11.x (Phase 2) | HTTP client; used for model downloads | Monitor for TLS/security updates | Active |
| `indicatif` | (Phase 2) | Progress bar library; non-critical | Routine updates | Stable |
| `inquire` | (Phase 4+) | Interactive prompts library; used in non-TTY-refusable flows | Monitor for prompt-injection or TTY-related bugs; keep up to date | Stable |
| `rmcp` | (Phase 3) | MCP protocol implementation; required for MCP server (US1) | Monitor for spec-alignment updates; test integration with harness | Active |
| `tokio` | (Phase 3, scoped) | Async runtime; used only in `src/mcp/` (structural test enforces boundary) | Constitution gate: verify tokio stays out of Phase 1–2 code paths; test async boundary quarterly | Active |
| `tracing-subscriber` | (Phase 3) | Structured logging framework; used in MCP server only | Monitor for JSON formatter updates and file I/O edge cases | Stable |
| `schemars` | (Phase 3) | JSON schema generation for MCP tool inputs; used at compile-time | Monitor for schema correctness issues on MCP tool definitions | Active |

## Phase 3 Polish Deferrals

Phase 3 closes with the following deferred items tracked in `retro/P8.md`:

| ID | Area | Description | Effort | Target Phase |
|----|------|-------------|--------|--------------|
| T088 | Integration testing | Manual SC-001 / SC-002 verification against real BGE models (vector search correctness + reranker ranking) | High | Developer pass (Phase 4+) |
| T093 | MCP protocol testing | Protocol purity + latency + SIGINT graceful shutdown integration tests (need real models or stub-injection point) | High | Phase 4+ (requires TD-014 refactor or test fixture enhancement) |
| T094 | MCP latency budget | Latency targets: p50 < 300ms, p99 < 600ms for search_skills + get_skill | Medium | Phase 4+ |
| T095 | MCP SIGINT handling | Graceful shutdown testing under realistic tool-handler load | Medium | Phase 4+ |
| m-WKS-* | Workspace long-tail tests | Remaining edge-case coverage gaps in workspace commands + catalog refcount | Low | Phase 4+ |

---

## Concern Severity Guide

| Level | Definition | Response Time |
|-------|------------|---------------|
| Critical | Production impact, security breach, test failure blocking ship | Immediate |
| High | Degraded functionality, security risk, blocking feature | This sprint |
| Medium | Developer experience, minor functional issues, UX confusion | Next sprint |
| Low | Nice to have, enhancement, cosmetic | Backlog |

---

*This document tracks what needs attention. Update when concerns are resolved or discovered.*
