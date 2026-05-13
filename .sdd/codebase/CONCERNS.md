# Known Concerns

> **Purpose**: Document technical debt, known risks, bugs, fragile areas, and improvement opportunities.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-13 (Phase 8 incremental)

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

## Deprecated Code

Code marked for removal:

| Area | Deprecation Reason | Removal Target | Replacement |
|------|-------------------|----------------|-------------|
| (none) | — | — | — |

All Phase 1, Phase 2, Phase 4, Phase 5, Phase 6, Phase 7, and Phase 8 code is current; no legacy to remove yet.

## Performance Concerns

Known performance issues:

| ID | Area | Description | Impact | Mitigation |
|----|------|-------------|--------|-----------|
| PERF-001 | Catalog refresh | Each `git fetch` is sequential; large catalogs block the command | Slow UX for multiple catalogs | Phase 1 spec requires sequential; parallelize in Phase 2 with async |
| PERF-010 | Cache validation | Manifest is re-parsed on every `show` command; no caching layer | Negligible impact (small files) | Cache not needed in Phase 1; revisit if Phase 2 manifests grow large |
| PERF-020 | Model download progress | Download wrapped in indeterminate spinner, not byte-progress bar | Poor visibility on large files | Enhancement for Phase 3 polish (TD-010). Phase 7 `tome reindex` also lacks per-skill progress visibility |

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
| `sqlite-vec` | vendored (Phase 2) | Custom C extension vendored under `vendor/sqlite-vec/`; compiled in via `build.rs` | Compiled as part of build; no separate update cadence | Pinned |
| `fastembed-rs` | (Phase 2) | Wraps ONNX Runtime; size-critical dependency | Monitor for updates; test binary size on bump | Active |
| `ort` (transitive) | (Phase 2) | ONNX Runtime via fastembed; intrinsically large (~25 MB contribution) | Size budget already accounted for; no waivable constraint | Locked by use case |
| `tempfile` | 3.x | Actively maintained; used for critical atomicity | Upgrade within 3.x when available; test after upgrade | Stable |
| `ctrlc` | 3.x | Small, stable crate; signal handling is straightforward | None in Phase 1; review if signal handling becomes more complex | Stable |
| `regex` | 1.x | Actively maintained; no known security issues | None | Stable |
| `reqwest` | 0.11.x (Phase 2) | HTTP client; used for model downloads | Monitor for TLS/security updates | Active |
| `indicatif` | (Phase 2) | Progress bar library; non-critical | Routine updates | Stable |
| `inquire` | (Phase 4+) | Interactive prompts library; used in non-TTY-refusable flows | Monitor for prompt-injection or TTY-related bugs; keep up to date | Stable |

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

## Monitoring Gaps

Areas lacking proper observability:

| Area | Missing | Impact | Priority |
|------|---------|--------|----------|
| Git operation timing | No latency metrics | Can't detect slow clones/fetches in automation | Low (single-user CLI; Phase 2 MCP may need metrics) |
| Index database health | No validation of persisted state on startup (Phase 2) | Corrupted index undetected until query | Low (atomicity guarantees + integrity_check PRAGMA should prevent corruption) |
| Model download errors | Network failures not distinguished from checksum failures | Harder to diagnose transient vs. persistent issues | Low (both map to Io/ModelChecksumMismatch; rare in practice) |
| Catalog size statistics | No cache size tracking | Can't warn on large catalogs | Low (Phase 2 may add quota management) |

## Design Tradeoffs

Intentional design decisions with known limitations:

| Decision | Area | Rationale | Consequence | Notes |
|----------|------|-----------|-------------|-------|
| **Per-plugin atomicity** (Phase 7) | `src/index/skills.rs::reindex_plugin_atomic` | Simpler transaction model; each plugin reindex commits independently | Multi-plugin `tome catalog update` or `tome reindex` may leave earlier plugins committed if interrupted between plugins | Safe state always (no partial rows); index is always valid. By design, not a bug. Advisory lock per-plugin at entry to reindex, released at commit. |
| **Status lock-free** (Phase 8) | `src/commands/status.rs::run` (no advisory lock taken) | Allows health check to run even when a writer is running; supports use as a non-invasive doctor command (FR-056) | Status report is a point-in-time snapshot; may be stale if another process is concurrently writing | Acceptable trade-off for pre-flight non-blocking diagnosis. Caller should understand the snapshot may be moments old. |

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

**Known design debts** (minor):
- User-declines-model-download prompt reuses exit 8 (same as system interrupt); future phases may lock down distinction (SEC-002)

### Phase 5 (Complete)

**Completed (US3)**:
- ✓ `tome plugin disable` command with confirmation prompt (FR-005 / FR-007)
- ✓ Non-TTY refusal with `--force` bypass (FR-051)
- ✓ Pointer message to stderr guiding users to `--force` flag
- ✓ Cheap re-enable via retained embeddings (FR-006)
- ✓ Integration tests covering all flows (`plugin_disable.rs`)

**Key security characteristics**:
- Disable reads catalog config + writes to index DB under existing advisory lock + transaction discipline
- No new credential surfaces, no new external endpoints, no new file I/O patterns
- TTY enforcement is consistent with existing `plugin enable` and interactive flow patterns
- User declining confirmation is clean exit (0, no state change) — semantically distinct from system interrupt (8)

**Known design debts** (minor):
- Semantic difference between "user declined disable" (0), "user declined model download" (8), and "system interrupt" (8) not formally pinned; currently acceptable per interactive-flow semantics

### Phase 6 (Complete)

**Completed (US4, Slice 1)**:
- ✓ `tome models download | list | remove` CLI surface
- ✓ Surfaces existing security primitives: pinned MODEL_REGISTRY SHA-256s, atomic-rename + checksum-verify pipeline
- ✓ New `embedding::download::sha256_file` public helper for re-verifying installed artefacts via `--verify` flag in `models list`
- ✓ No new attack surface — CLI surfaces already-secured primitives from Phase 2

**Security posture**:
- Model download uses existing atomicity guarantees and integrity verification
- `--verify` flag allows users to audit installed models without re-downloading
- No new credential surfaces, no new external endpoints

**Ongoing**:
- TD-010: Both `plugin enable` and `models download` now ship indeterminate spinners; refactor to byte-progress callback deferred to polish pass

### Phase 7 (Complete)

**Completed (US5, Slice 1–3)**:
- ✓ `src/index/skills.rs::reindex_plugin_atomic` + `auto_disable_orphan` for multi-plugin index reconciliation
- ✓ `tome catalog update` wired to lazy `FastembedEmbedder` load and per-plugin reindex
- ✓ `tome reindex` CLI subcommand with same lazy-load pattern
- ✓ Fixed latent `sqlite-vec` virtual table bug: `upsert_skill` now uses `DELETE`-then-`INSERT` instead of unsupported `INSERT OR REPLACE` (PR #25)
- ✓ No new credentials, attack surface, or external integrations

**Security posture**:
- Index reindex under per-plugin transaction (atomic within plugin, but multiple plugins commit independently)
- Auto-disable on `PluginNotFound` / `PluginManifestParseError` drops rows + emits loud-warning stderr
- Lazy embedder loading ensures zero-enabled-plugin install never touches ONNX models
- Virtual table constraint now explicitly documented in SECURITY.md and CONCERNS.md

**Design constraints**:
- Per-plugin atomicity is intentional: multi-plugin reindex may leave earlier plugins committed if interrupted between plugins (always valid state, by design)

### Phase 8 (Complete)

**Completed (US6, Slice 1)**:
- ✓ `tome status [--verify] [--json]` read-only health check command (FR-056)
- ✓ Extended `tome --version` / `tome -V` to include embedder + reranker identities (reproducibility set)
- ✓ Pre-parse hook in `main.rs` for early `--version` interception before clap dispatch
- ✓ Model re-verification via `--verify` flag (uses existing `sha256_file` helper, no re-download)
- ✓ Drift detection + classification (Ok, Degraded, Unhealthy) to guide bug reporting
- ✓ No new credentials, external endpoints, or file I/O patterns

**Security posture**:
- Status is lock-free (FR-056) — non-invasive pre-flight check even when writer is running
- Embedder drift → Unhealthy (vectors are invalid); reranker drift → Degraded (queries still serve)
- Version output includes only public model registry constants; no secrets exposed
- Pre-parse hook is minimal and self-contained; only scans for `--version` / `-V` flags before clap setup

**Design tradeoffs**:
- Status report is a point-in-time snapshot; may be stale if another process is concurrently writing (acceptable for diagnosis purposes)
- Exit code 1 for both Degraded and Unhealthy (caller must check `--json` for detailed classification)

**Integration test coverage** (Phase 8):
- `tests/status.rs` verifies report assembly, drift detection scenarios, and JSON/human output formats

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
