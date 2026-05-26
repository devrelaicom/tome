# Phase 5 / US2 — Reviewer findings

Consolidated from 4 parallel reviewer agents (contract / Rust-lens / test / security).
Source reports: `/tmp/tome-phase5-us2-{contract,rust,test,security}.md`.

## Tally

| Reviewer | Blockers | Majors | Minors |
|---|---|---|---|
| Contract  | 0 | 1 | 2 |
| Rust      | 2 | 5 | 6 |
| Test      | 0 | 3 | 5+ |
| Security  | 0 | 2 (MEDIUM) | 2 (LOW) |
| **Total** | **2** | **11** | **15** |

**The 2 BLOCKERS are concentrated on the no-rescan invariant** (NFR-007 / FR-051), confirmed independently by all 4 reviewers.

## BLOCKERS

### B1: `WORKSPACE_DATA` error misrouted to wrong variant + wrong exit code
**Source**: Rust review B1
**File**: `src/substitution/builtins.rs:50-67`
**Issue**: When `WorkspaceName::parse` fails on a `${TOME_WORKSPACE_DATA}` reference, the error is wrapped in `SubstitutionError::PluginDataDirCreationFailed` (exit 9) — but the actual subsystem that failed is the workspace data dir (exit 25). The path field also points at `workspace_data_dir` while the variant name implies a plugin path. Wire-shape + exit-code violation.
**Fix**: Use `WorkspaceDataDirCreationFailed`. Branch is unreachable from production (workspace name flows from validated `WorkspaceName::as_str()`); consider `debug_assert!` or document the unreachable invariant.

### B2: No-rescan invariant violated — Stage 2 sweeps full Stage 1 output (DATA EXFILTRATION VECTOR)
**Source**: Rust review B2 + Contract review (only major) + Test review (missing test)
**File**: `src/substitution/mod.rs:103-108`
**Issue**: `apply_env` runs `replace_all` over THE ENTIRE Stage 1 output, not just the substring outside Stage 1 matches. This violates the contract verbatim ("Substituted values are NOT re-scanned by later stages — FR-051"). 

**Privilege-escalation vector**: a hostile plugin author can set `"version": "${TOME_ENV_GITHUB_TOKEN}"` in their `plugin.json` (the field is leniently parsed at `src/plugin/manifest.rs:21`). Any skill body that interpolates `${TOME_PLUGIN_VERSION}` will leak the operator's `TOME_ENV_GITHUB_TOKEN` (or any other `TOME_ENV_*`) into the LLM context.

Other affected Stage 1 fields: `entry_path`, `entry_dir`, `plugin_root_dir`, `skill_path`, `entry_name`, `catalog_name`, `plugin_name`. `WorkspaceName` is validated but others aren't constrained against `${`.

**Fix shape**: merge Stage 1 + Stage 2 into a single regex sweep using union pattern `\$\{TOME_(?:ENV_([A-Z0-9_]+)|([A-Z0-9_]+))(?::-(.*?))?\}` — the env branch has higher specificity and wins on alternation. Single-allocation `String::with_capacity` shape carries over.

MUST land before US3 ships, or the same defect repeats for argument-bearing substitutions.

## Majors

### R-M1: `apply_env` `Cow` fast-path materialises unconditionally
**File**: `src/substitution/env.rs:40` + `src/substitution/mod.rs:105`
**Issue**: `apply_env` returns `Cow<'_, str>`, but the sole caller calls `.into_owned()` immediately. Fast-path `Cow::Borrowed(body)` forces a `body.to_owned()` clone, costing one extra allocation per no-match call.
**Fix**: B2's merged Stage 1+2 sweep naturally subsumes this. If not merging, change `apply_env` to `fn apply_env(body: String) -> String`.

### R-M2: `SubstitutionContext.plugin_data_dir` + `workspace_data_dir` fields are dead
**File**: `src/substitution/context.rs:30,127-130,193-195`
**Issue**: Required on builder, validated in `build()`, never read in production path (resolution goes via `data_dir::ensure_plugin_data` directly). Paying a `PathBuf` alloc + Vec push per render with no read.
**Fix**: Drop both fields from `SubstitutionContext` and matching builder steps.

### R-M3: `apply_builtins` triggers debug-log for every `${TOME_ENV_*}` reference
**File**: `src/substitution/builtins.rs:120-127`
**Issue**: Stage 1 regex `\$\{TOME_([A-Z0-9_]+)(?::-(.*?))?\}` matches `${TOME_ENV_FOO}` (because `ENV_FOO` is `[A-Z0-9_]+`). `resolve_builtin("ENV_FOO", ...)` returns `Ok(None)`, match emitted verbatim, debug event fires. Every env reference logs a "leaving verbatim" event despite being intentional.
**Fix**: B2's merged regex subsumes. Narrow fix: special-case `name.starts_with("ENV_") => return Ok(None)` without the debug event.

### R-M4: `caps.get(1).unwrap_or("")` masks future regex changes
**File**: `src/substitution/builtins.rs:116` + `src/substitution/env.rs:49`
**Issue**: Capture group 1 is `[A-Z0-9_]+`, non-optional. `.unwrap_or("")` silently swallows a future pattern change.
**Fix**: `.expect("capture group 1 is non-optional in BUILTIN_REGEX")`.

### R-M5: `apply_builtins` missing no-match fast-path
**File**: `src/substitution/builtins.rs:108-133`
**Issue**: Unconditionally allocates `String::with_capacity(body.len())` even when body has no substitution references.
**Fix**: B2's merged regex subsumes (single fast-path covers both stages). Otherwise add `if !re.is_match(body) { return Ok(body.to_owned()); }`.

### T-M1: NFR-007 no-rescan invariant has no direct test
**Source**: Test review
**File**: `tests/substitution_env.rs` (lacks the test)
**Issue**: Per Test reviewer, no test verifies that a Stage-1 substituted value containing `${TOME_ENV_*}` syntax is NOT re-substituted by Stage 2. The existing `body_with_no_env_references_is_unchanged_by_stage_2` only tests the fast-path with no Stage-1 references.
**Fix**: Add `stage_1_substituted_value_containing_tome_env_syntax_not_rescanned_by_stage_2` — construct a context where a built-in (e.g. via override-installed `PLUGIN_VERSION` test value) resolves to a string containing `${TOME_ENV_X}` and verify Stage 2 doesn't substitute it. Pairs with B2 fix.

## Majors deferred to v0.6+ backlog

- **C-M1** (NFR-007 violation): subsumed by R-B2 fix
- **C-m1**: Builder `InvalidArgumentFrontmatter` reuse for missing-field errors — cosmetic
- **C-m2**: data-model.md §6 missing `PluginDataDirWriteFailed→9` row — doc-only
- **R-S1-S6**: 6 Rust suggestions (drop `_default`, `body[..]` over `m.as_str()`, `u32::MAX` clip, `WorkspaceName::parse` storage, `rewrite_marker_workspace` allocation, `Arc<Paths>`)
- **T-M2**: `WorkspaceDataDirCreationFailed` variant untested
- **T-M3 + other**: edge cases (empty body, adjacent references, Unicode values)
- **Security MEDIUM 1**: env-var exfiltration via legit `TOME_ENV_*` (intentional behaviour; document in operator guide for v0.6 polish)
- **Security MEDIUM 2**: Unicode in path component sanitisation (existing sanitise covers common cases)
- **Security LOW 1+2**: symlink check in rename, 0600 mode on plugin-data dirs
