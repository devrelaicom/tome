# Known Concerns

> **Purpose**: Document technical debt, known risks, bugs, fragile areas, and improvement opportunities.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-11

## Technical Debt

### High Priority

Items that should be addressed before Phase 2 ships:

| ID | Area | Description | Impact | Effort | Notes |
|----|------|-------------|--------|--------|-------|
| TD-001 | `src/catalog/git.rs` | Advisory locking for concurrent catalog access | Concurrency safety | High | Phase 2 MCP server will expose concurrent harness access; current atomic-rename model insufficient |
| TD-002 | Async runtime | Synchronous-only by design will become blocking in Phase 2 | Performance, scalability | High | Phase 1 complete, but MCP server (expected Phase 2 forcing function) requires async. Plan migration path to `tokio` + `hyper` |
| TD-003 | Dependency footprint for Phase 2 | SQLite + ONNX (likely Phase 2 components) will significantly exceed 10 MB | Binary size | Medium | Current ~2.7 MB leaves 7.3 MB headroom; preemptively justify or restructure |

### Medium Priority

Items to address when working in the area:

| ID | Area | Description | Impact | Effort | Mitigation |
|----|------|-------------|--------|--------|-----------|
| TD-010 | `src/catalog/store.rs` | No explicit cleanup of temp directories on panics | Disk space leak | Low | Rust RAII is generally robust, but `tempfile` crate docs recommend reviewing drop paths; add test for panic during write |
| TD-020 | Error categorisation | `Internal(anyhow::Error)` variant is a catch-all for unclassified panics | Debuggability | Low | Avoid using `Internal`; make `TomeError` more granular if new categories needed |

### Low Priority

Nice-to-have improvements:

| ID | Area | Description | Impact | Effort |
|----|------|-------------|--------|--------|
| TD-030 | Module organisation | `src/commands/catalog/` has five subcommand files; could be collapsed into `commands.rs` in Phase 2 | Code clarity | Low |
| TD-040 | Logging verbosity | Current `-v` / `-vv` mapping to info/debug is fine, but `TOME_LOG` env filter is undocumented | UX | Low |

## Security Concerns

### High Risk

| ID | Area | Description | Risk Level | Mitigation |
|----|------|-------------|------------|-----------|
| SEC-001 | Phase 2 planning | MCP server without mutex/advisory locking will be vulnerable to TOCTTOU (time-of-check-to-time-of-use) race conditions on concurrent catalog updates | High | Design Phase 2 concurrency model before MCP server implementation; consider file-based advisory locking or mutex per catalog |
| SEC-002 | Binary size | Phase 2 dependencies (SQLite, ONNX) may push binary significantly over 10 MB; current justification framework is weak | Medium | Preemptively research smaller alternatives (e.g., `rusqlite` vs `sqlx`, `ort` vs full ONNX runtime); plan binary-size review gate for Phase 2 |

### Medium Risk

| ID | Area | Description | Risk Level | Mitigation |
|----|------|-------------|------------|-----------|
| SEC-010 | Credential scrubber | Regex-based scrubbing is pattern-based, not semantic; exotic credential formats may leak (e.g., GitLab private tokens with special delimiters) | Medium | Current rules (R-8) cover common patterns. Add integration tests against real Git helper output before Phase 2. Monitor GitHub issues for scrubbing edge cases |

### Low Risk

| ID | Area | Description | Risk Level | Mitigation |
|----|------|-------------|------------|-----------|
| SEC-020 | MSRV drift | Dependency updates may require MSRV bump; current MSRV is pinned but not validated in a separate CI job | Low | MSRV CI job exists and passes; keep Renovate PRs reviewed for MSRV compatibility |

## Known Bugs

Active bugs that haven't been fixed:

| ID | Description | Workaround | Severity | Status |
|----|-------------|------------|----------|--------|
| (none documented) | — | — | — | All known issues tracked in GitHub issues; no unfixed bugs in Phase 1 spec |

## Fragile Areas

Code areas that are brittle or risky to modify:

| Area | Why Fragile | Precautions |
|------|-------------|-------------|
| `src/catalog/git.rs::scrub_credentials` | Regex patterns are order-dependent; adding a rule can change ordering semantics | Add test case to `tests/scrubbing.rs` for every rule addition; verify no overlaps with existing rules |
| `src/catalog/manifest.rs::validate_source` | Path canonicalization behavior differs across platforms (symlinks, case sensitivity); one test failure can indicate subtle cross-platform issue | Test on both Linux and macOS (CI covers both); `tests/path_validation.rs` has Unix-specific symlink tests |
| `src/catalog/store.rs::write_atomic` | Atomic rename only works on same filesystem; moving across mounts silently falls back to non-atomic copy | Document assumption in code; consider detecting mount boundary and erroring explicitly |

## Deprecated Code

Code marked for removal:

| Area | Deprecation Reason | Removal Target | Replacement |
|------|-------------------|----------------|-------------|
| (none) | — | — | — |

All Phase 1 code is current; no legacy to remove yet.

## Performance Concerns

Known performance issues:

| ID | Area | Description | Impact | Mitigation |
|----|------|-------------|--------|-----------|
| PERF-001 | Catalog refresh | Each `git fetch` is sequential; large catalogs block the command | Slow UX for multiple catalogs | Phase 1 spec requires sequential; parallelize in Phase 2 with async |
| PERF-010 | Cache validation | Manifest is re-parsed on every `show` command; no caching layer | Negligible impact (small files) | Cache not needed in Phase 1; revisit if Phase 2 manifests grow large |

## TODO Items

Active TODO comments in codebase:

| Location | TODO | Priority | Status |
|----------|------|----------|--------|
| (none found) | — | — | Code is TODOs-clean; all planned work tracked in spec and PRD |

## External Dependency Risks

Dependencies that may need attention:

| Package | Version | Concern | Action Needed |
|---------|---------|---------|---------------|
| `clap` | 4.x | Actively maintained; track for 5.x breaking changes | Monitor releases; plan migration before major version bump |
| `serde` | 1.x | Stable; ecosystem standard | None |
| `tempfile` | 3.x | Actively maintained; used for critical atomicity | Upgrade within 3.x when available; test after upgrade |
| `ctrlc` | 3.x | Small, stable crate; signal handling is straightforward | None in Phase 1; review if signal handling becomes more complex in Phase 2 |
| `regex` | 1.x | Actively maintained; no known security issues | None |

**No unmaintained or vulnerable dependencies detected.** `cargo-audit` weekly + PR checks.

## Improvement Opportunities

Areas that could benefit from enhancement:

| Area | Current State | Desired State | Benefit |
|------|---------------|---------------|---------|
| Config file validation | Parsed strictly, but no schema documentation | Add inline documentation or separate schema doc | Easier for users to edit config manually |
| Manifest validation errors | Clear but sometimes verbose | Add `--json` error detail with machine-readable remediation suggestions | Better tooling integration |
| Cancellation messaging | Silent on Ctrl-C (clean, but terse) | Optional verbose exit message for debugging | Better UX in automation contexts |
| Symlink security testing | Unix-only symlink escape test | Windows junction/hardlink escape tests | Cross-platform parity |

## Monitoring Gaps

Areas lacking proper observability:

| Area | Missing | Impact | Priority |
|------|---------|--------|----------|
| Git operation timing | No latency metrics | Can't detect slow clones/fetches in automation | Low (single-user CLI; Phase 2 MCP may need metrics) |
| Catalog size statistics | No cache size tracking | Can't warn on large catalogs | Low (Phase 2 may add quota management) |
| Registry health | No validation of persisted state on startup | Corrupted registry undetected | Low (atomicity guarantees should prevent corruption) |

## Risk Summary by Phase

### Phase 1 (Current)

**Status**: All critical security controls implemented and tested.

- ✓ Credential scrubbing at capture boundary
- ✓ Path traversal prevention (six-step validation)
- ✓ Atomic writes for registry and cache
- ✓ Signal handling with clean child cleanup
- ✓ Closed error set with documented exit codes
- ✓ Licence and vulnerability scanning in CI

**Open items**: None blocking Phase 1 completion.

### Phase 2 (MCP Server)

**Expected introducing**: Async runtime, concurrent harness access, SQLite integration.

**Key risks to plan**:
1. TD-001: Concurrent catalog access requires mutex or advisory locking
2. TD-002: Async migration is a major refactoring (consider `tokio` early)
3. TD-003: Binary size: SQLite + ONNX will threaten 10 MB cap; justify or split binary

**Recommended pre-Phase-2 work**:
- Prototype async migration path (async main, tokio runtime)
- Benchmark SQLite integration for size footprint
- Design concurrency model for registry + cache with tests

### Phase 3+ (Long-term)

**Deferred**:
- Encryption at rest (if catalyst storage becomes sensitive)
- Audit logging (if multi-user or compliance required)
- Rate limiting (if deployed as shared service)

---

## Concern Severity Guide

| Level | Definition | Response Time |
|-------|------------|---------------|
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
