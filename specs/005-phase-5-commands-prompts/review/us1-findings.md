# Phase 5 / US1 — Reviewer findings

Consolidated from 4 parallel reviewer agents (contract / Rust-lens / test / security).
Source reports: `/tmp/tome-phase5-us1-{contract,rust,test,security}.md`.

## Tally

| Reviewer | Blockers | Majors | Minors |
|---|---|---|---|
| Contract  | 0 | 2 | 2 |
| Rust      | 0 | 6 | 8 |
| Test      | 0 | 3 | 5 |
| Security  | 1 | 2 | 1 |
| **Total** | **1** | **13** | **16** |

The 1 blocker is `S-H1` (path traversal in `resolve_entry_body_path`). All other findings are non-blocking; the disposition (in `us1-disposition.md`) records which majors are applied in US1.d and which defer to v0.6+.

## Severity legend

- **BLOCKER**: must fix before US1.d closeout merges. Security holes, wire-shape wrong, FR violated, constitutional violation.
- **MAJOR**: should fix; deferring requires explicit rationale in disposition.
- **MINOR**: cosmetic / typo / nice-to-have. Defer freely.

## Findings

### BLOCKER

#### S-H1: Path traversal via relative path in `resolve_entry_body_path`
**Source**: Security review
**File**: `src/index/skills.rs::resolve_entry_body_path`
**Threat**: The function accepts a `stored_path` (relative path from the DB) and joins it directly to `plugin_dir` via `plugin_dir.join(&stored)` without canonicalisation or `..` validation. A hostile plugin can craft a relative path like `../../../etc/passwd` to read arbitrary files. The relative path is populated at `plugin enable` time via `file.strip_prefix(plugin_dir)` (`src/plugin/lifecycle.rs:774–776`), which preserves traversal sequences.
**Fix**: Validate that `stored_path` contains no `..` components, OR canonicalise the result and reject any path that escapes `plugin_dir`.

### MAJORS

#### R-M1: `SubstitutionError::PluginDataDirCreationFailed` collapses into `TomeError::WorkspaceDataDirWriteFailed`
**Source**: Rust review
**File**: `src/mcp/prompts.rs:559-563`
**Issue**: The substitution-side `PluginDataDirCreationFailed` and `WorkspaceDataDirCreationFailed` variants both map to a single `TomeError::WorkspaceDataDirWriteFailed`. Variant name + exit code lie about which directory failed. The closed-error-enum principle requires one variant per failure mode.
**Fix**: Pre-allocate `PluginDataDirWriteFailed` companion variant (mirrors substitution-side split) with its own exit code, OR rename `WorkspaceDataDirWriteFailed` to a neutral name.

#### R-M2 + S-L1: `EntryNotFound.kind` stuffed with frontmatter parse error string
**Source**: Rust review (combined with Security low)
**File**: `src/mcp/prompts.rs:543-548`
**Issue**: Frontmatter parse failure at `prompts/get` time maps to `EntryNotFound { kind: "Skill: frontmatter parse failed: <err>" }`. Two problems: (a) `kind` is documented as `"skill"` or `"command"` discriminator — stuffing arbitrary error text breaks downstream `kind ==` matchers; (b) the failure is semantically `SkillFrontmatterParseError` (exit 23), not "entry not found".
**Fix**: Map frontmatter parse failure at this site to `SkillFrontmatterParseError { file: body_path.clone(), message: err.to_string() }`. Keep `EntryNotFound` reserved for "DB row missing" / "on-disk plugin dir missing" cases.

#### R-M3: `render_for_get` opens the index DB twice per `prompts/get` call
**Source**: Rust review
**File**: `src/mcp/prompts.rs:524, 652-660`
**Issue**: First DB open at line 524 (path resolve); second at line 652 (`read_plugin_version`). Every call pays 2× `index::db::open_read_only` + WAL setup.
**Fix**: Cache `plugin_version` on `PromptEntry` at registry build time (add `s.plugin_version` to the registry SELECT). Removes the second DB open from the hot path entirely.

#### R-M4: `entry.path.display().to_string()` lossy round-trip
**Source**: Rust review
**File**: `src/mcp/prompts.rs:531-536`
**Issue**: `display().to_string()` substitutes U+FFFD on non-UTF8 paths. Comment acknowledges it's a hack. The registry already cached the absolute `PathBuf`; the re-resolve via `resolve_entry_body_path` short-circuits on `is_absolute()` anyway, making this round-trip pure overhead AND lossy.
**Fix**: Skip the re-resolve entirely (`let body_path = entry.path.clone();`) since the registry's cached PathBuf is already absolute. Combine with R-M3 fix.

#### R-M5: `LookupHit.row: Box<SkillRecord>` allocated only to be discarded
**Source**: Rust review
**File**: `src/mcp/tools/get_skill.rs:201, 221-223`
**Issue**: `Box<SkillRecord>` heap-allocated, threaded through `spawn_blocking`, then `let _ = row;` discarded. Dead state with a "future extensions" comment.
**Fix**: Remove `row` from `LookupHit`. If a later phase needs it, add it back.

#### T-M1 + T-M5: Error envelope JSON wire shapes not pinned
**Source**: Test review
**File**: `src/mcp/prompts.rs::error_to_mcp_error` (~line 770+)
**Gap**: Tests verify `prompt_not_found` and `prompt_argument_mismatch` logic but don't pin the exact JSON-RPC error envelope serialisation. Contract codes per `mcp-prompts.md § Error responses`.
**Fix**: Add `tests/mcp_prompts_get_error_json_shape.rs` pinning each error code's JSON envelope (template: `mcp_prompts_get_json_shape.rs`).

#### T-M3: Missing entry files during registry build not tested
**Source**: Test review
**File**: `src/mcp/prompts.rs::build_for_workspace` (~lines 240–295)
**Gap**: Three warn-and-skip branches (entry file missing, frontmatter malformed, catalog dir unresolvable) are uncovered. Tests cover empty-workspace and happy-path; the degradation path is silently untested.
**Fix**: Add a test staging a workspace with entries in the DB but files deleted on disk; verify registry builds and surfaces zero prompts.

#### S-M1: Unbounded `arguments` list in frontmatter parsing
**Source**: Security review
**File**: `src/plugin/frontmatter.rs::deserialize_arguments`
**Threat**: No size cap on the arguments list. A hostile plugin can declare 10,000+ argument names in YAML, fitting within the 256 KiB file cap but triggering unbounded allocations during enable. Per-plugin DoS at enable time.
**Fix**: Add `const MAX_ARGUMENTS: usize = 256;` and check in `visit_seq` before pushing.

### MAJORS deferred to v0.6+ backlog (in disposition.md)

- **C-M1**: contract wording (clarifying comment only — no behaviour change)
- **C-M2**: `$ARGUMENTS` heuristic (already deferred to US3 per contract)
- **R-M6**: `Arc<PromptEntry>` in registry (optimization; low-count is common case)
- **T-M2**: substitution failure path test (F3 stub makes this hard; lights up in US2/US3 naturally)
- **S-M2**: YAML deserialisation panic safety (`serde_yaml` rarely panics in practice; bounded read + valid-UTF8 frontmatter already mitigate)

### MINORS deferred (16 items)

All 16 minors are tracked in `us1-disposition.md` and deferred to v0.6 polish or natural-attrition.
