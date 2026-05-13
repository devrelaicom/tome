# Known Concerns

> **Purpose**: Document technical debt, known risks, bugs, fragile areas, and improvement opportunities.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-13

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
| TD-010 | `src/embedding/download.rs` | No byte-progress callback for model downloads | UX | Low | Currently wrapped in indeterminate spinner; enhancement for polish pass |
| TD-020 | Error categorisation | All Phase 1 + Phase 2 codes are enumerated; no catch-all variants | Debuggability | Low | Current approach is sound; closed set enforces completeness |
| TD-030 | Code duplication (Phase 4) | `paths_for(&ToolEnv) -> Paths` duplicated in 3 integration test files (`plugin_list.rs`, `plugin_show.rs`, `plugin_interactive.rs`) | Test maintainability | Low | Promote to `tests/common/mod.rs` when a 4th caller appears (likely Phase 5 `plugin_disable.rs`) |

### Low Priority

Nice-to-have improvements:

| ID | Area | Description | Impact | Effort |
|----|------|-------------|--------|--------|
| TD-040 | Logging verbosity | Current `-v` / `-vv` mapping is fine; `TOME_LOG` env filter is undocumented | UX | Low |

## Security Concerns

### High Risk

| ID | Area | Description | Risk Level | Mitigation | Status |
|----|------|-------------|------------|-----------|--------|
| SEC-001 | Phase 2 BGE testing (T088) | Vector search correctness not yet measured against real BGE models (bge-small-en-v1.5, bge-reranker-base) | High | Complete developer-machine pass with real models; validate SC-001 / SC-002 | Pending Phase 3 |

### Medium Risk

| ID | Area | Description | Risk Level | Mitigation | Status |
|----|------|-------------|------------|-----------|--------|
| SEC-002 | Phase 3+ model-download UX | User declines model-download prompt (e.g., in `tome plugin enable`) → returns exit 8 (reused from Interrupted); no dedicated exit code | Medium | Lock down user-decline vs. system-interrupt distinction in future iteration | Design debt |
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

## Deprecated Code

Code marked for removal:

| Area | Deprecation Reason | Removal Target | Replacement |
|------|-------------------|----------------|-------------|
| (none) | — | — | — |

All Phase 1, Phase 2, and Phase 4 code is current; no legacy to remove yet.

## Performance Concerns

Known performance issues:

| ID | Area | Description | Impact | Mitigation |
|----|------|-------------|--------|-----------|
| PERF-001 | Catalog refresh | Each `git fetch` is sequential; large catalogs block the command | Slow UX for multiple catalogs | Phase 1 spec requires sequential; parallelize in Phase 2 with async |
| PERF-010 | Cache validation | Manifest is re-parsed on every `show` command; no caching layer | Negligible impact (small files) | Cache not needed in Phase 1; revisit if Phase 2 manifests grow large |
| PERF-020 | Model download progress | Download wrapped in indeterminate spinner, not byte-progress bar | Poor visibility on large files | Enhancement for Phase 3 polish (TD-010) |

## TODO Items

Active TODO comments in codebase:

| Location | TODO | Priority | Status |
|----------|------|----------|--------|
| (none found) | — | — | Code is TODOs-clean; all planned work tracked in spec and PRD |

## External Dependency Risks

Dependencies that may need attention:

| Package | Version | Concern | Action Needed | Status |
|---------|---------|---------|---------------|--------|
| `clap` | 4.x | Actively maintained; track for 5.x breaking changes | Monitor releases; plan migration before major version bump | Stable |
| `serde` | 1.x | Stable; ecosystem standard | None | Stable |
| `rusqlite` | 0.31.x (Phase 2) | Bundled SQLite; monitor for platform-specific build issues | Test across CI matrix | Stable |
| `fastembed-rs` | (Phase 2) | Wraps ONNX Runtime; size-critical dependency | Monitor for updates; test binary size on bump | Active |
| `ort` (transitive) | (Phase 2) | ONNX Runtime via fastembed; intrinsically large (~25 MB contribution) | Size budget already accounted for; no waivable constraint | Locked by use case |
| `tempfile` | 3.x | Actively maintained; used for critical atomicity | Upgrade within 3.x when available; test after upgrade | Stable |
| `ctrlc` | 3.x | Small, stable crate; signal handling is straightforward | None in Phase 1; review if signal handling becomes more complex | Stable |
| `regex` | 1.x | Actively maintained; no known security issues | None | Stable |
| `reqwest` | 0.11.x (Phase 2) | HTTP client; used for model downloads | Monitor for TLS/security updates | Active |
| `indicatif` | (Phase 2) | Progress bar library; non-critical | Routine updates | Stable |
| `inquire` | (Phase 4) | Interactive prompts library; used only in non-TTY-refusable flows | Monitor for prompt-injection or TTY-related bugs; keep up to date | Stable |

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
| `presentation::tables` module exports | `Cell` + `CellAlignment` imported directly by consumers from `comfy_table` | Re-export from `presentation::tables` for convenience | Cleaner API |

## Monitoring Gaps

Areas lacking proper observability:

| Area | Missing | Impact | Priority |
|------|---------|--------|----------|
| Git operation timing | No latency metrics | Can't detect slow clones/fetches in automation | Low (single-user CLI; Phase 2 MCP may need metrics) |
| Index database health | No validation of persisted state on startup (Phase 2) | Corrupted index undetected until query | Low (atomicity guarantees + integrity_check PRAGMA should prevent corruption) |
| Model download errors | Network failures not distinguished from checksum failures | Harder to diagnose transient vs. persistent issues | Low (both map to Io/ModelChecksumMismatch; rare in practice) |
| Catalog size statistics | No cache size tracking | Can't warn on large catalogs | Low (Phase 2 may add quota management) |

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

### Phase 3 (Slice 1 Complete, Slice 2+ In Progress)

**Completed (Slice 1)**:
- ✓ Model registry with real checksums (verified at start of Phase 3)
- ✓ Atomic model download + verification
- ✓ Plugin enable/disable wired to CLI
- ✓ Skill metadata parsing (lenient)
- ✓ Plugin manifest parsing (lenient, FR-013a)

**In progress**:
- T088: Real BGE model testing against SC-001/SC-002
- Query command full implementation
- Reindex command full implementation

**Key risks to monitor**:
- SEC-001: BGE model testing still pending (T088)
- SEC-002: User-decline vs. interrupt exit code (design debt)
- TD-010: Model download progress UX (polish pass)

### Phase 4 (Complete)

**Completed (US2)**:
- ✓ Interactive `tome plugin` browse flow with catalog/plugin/action selectors
- ✓ TTY enforcement at flow entry via `require_terminal()` — exit 54 if no TTY (FR-051)
- ✓ Prompt functions (`select`, `multiselect`, `confirm`) with non-TTY short-circuits
- ✓ Post-action redraw and navigation (Back, Quit)
- ✓ Non-TTY refusal test (`plugin_interactive.rs::bare_plugin_without_a_terminal_exits_54_with_pointer_message`)
- ✓ Scripted pty test covering full navigation tree (T101)

**Key security additions**:
- Interactive flows refuse to run in non-TTY contexts (exit 54), preventing prompt-injection and mangled input
- User cancellation via Ctrl-C/Ctrl-D surfaces as exit 8 (Interrupted), semantically aligned with system SIGINT
- Dependency: `inquire` (MIT) — stable, actively maintained

**Known design debts** (minor):
- User-declines-model-download prompt reuses exit 8 (same as system interrupt); future phases may lock down distinction (SEC-002)
- Test code duplicates `paths_for()` helper in 3 files; promote to `tests/common/mod.rs` when 4th caller appears (TD-030)

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
