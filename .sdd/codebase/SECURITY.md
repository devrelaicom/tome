# Security

> **Purpose**: Document authentication, authorization, security controls, and vulnerability status.
> **Generated**: 2026-05-27
> **Last Updated**: 2026-05-29 (Phase 6 / US5 privilege governance + doctor extensions; incremental update)

## Overview

Tome is a Rust CLI (and MCP server) for managing plugin catalogs, embeddings, workspace settings, project bindings, workspace summarisation, command/prompt entries, agent translations, agent personas as MCP prompts, and harness synchronisation across multiple coding harnesses. As a synchronous, file-based tool without user authentication, security focuses on:

1. Preventing path traversal and directory-escape attacks via plugin source paths, plugin identities, workspace names, project paths, entry body paths, agent names, and harness configurations
2. UTF-8 validation for project paths stored in the central DB (primary key constraint)
3. Integrity verification for downloaded model artefacts (SHA-256 checksums across three inference runtimes: embedder, reranker, summariser)
4. Symlink refusal on all workspace/project/harness file writes (defence in depth)
5. File mode preservation on atomic rewrites to prevent silent permission downgrades
6. Scrubbing credentials from captured Git output and HTTP errors at the boundary
7. Atomic writes to prevent partial state corruption (staging directory pattern with same-FS rename)
8. Signal handling for clean interruption
9. TTY enforcement on interactive flows to prevent prompt injection and non-interactive misuse
10. Dependency-allowlist enforcement and weekly vulnerability scanning
11. Binary-size constraints to limit attack surface
12. MCP server protocol purity (stdout reserved for MCP protocol, errors to stderr)
13. Structured logging with size-based rotation and credential scrubbing for long-running MCP server
14. MCP startup pre-flight validation with SHA-256 verification and drift detection
15. No domain-error leakage in MPC tool responses (structured codes only)
16. Workspace initialization with secure directory permissions and atomic staging
17. Project binding with workspace-name validation and UTF-8 path enforcement
18. Harness MCP config read-modify-write preserving third-party structure (comment/order preservation via `toml_edit`)
19. RULES.md block insertion and standalone-file strategies with symlink-aware writes
20. Catalog cache content-trust via ref-counting in central DB and re-use on same URL
21. Doctor command harness detection without config parsing; network access gated behind `--fix`
22. Forward-only schema migrations with per-migration transaction atomicity
23. Central SQLite DB with advisory lockfile covering all state mutations and cache cleanup
24. Workspace registry validation with size cap, entry limit, NUL rejection, and parent-dir rejection
25. Workspace init refusal of non-directory `.tome` markers
26. Workspace removal cascade per-project effective list narrowing
27. Workspace rename with DB transaction wrapping marker rewrites
28. Workspace settings and rules file preservation of forward-compat fields
29. Credential scrubbing on MCP log fields and error chains
30. Phase 4 / US4 additions: Bundled Qwen2.5-0.5B-Instruct summariser model; Llama.cpp-2 inference runtime with SHA-256 verified model load; prompt-constructed summaries for plugin descriptions; cached LlamaModel with per-call LlamaContext; summariser-model integrity gate (placeholder detection); length-window enforcement with warn-level logging
31. Phase 4 / US5 additions: Doctor command extensible with five repair classes (embedder/reranker/catalog/binding/summariser); orphan staging-directory cleanup with TOCTOU-safe mtime gating; project-local harness synchronisation separate from workspace-broadcast; user-owned harness MCP override filtered by active fix list; read-only index access for diagnostic lookups
32. Phase 4 / Polish additions: `util::bounded_read_to_string` with per-class caps applied to ~26 sites; `home_root()` validation for absolute, canonical, exists; relative/unset `$HOME` exits 2 Usage; consolidated `ProjectMarkerConfig` to one type + canonical `settings::parser::read_project_marker`; doctor harnesses list hyphenated naming; five-layer defence-in-depth for `remove_dir_all` on scanned dirs
33. Phase 5 / US1 additions: Entry body-file path validation rejects `..` traversal and absolute paths (prevents directory-escape via skill/command bodies); arguments list hard-capped at 256 entries in frontmatter parser (DoS mitigation); per-entry invocability flags (`user_invocable` column); resource enumeration cap (5 per directory + sentinels) with symlink-skip hardening
34. Phase 5 / US2 additions (CRITICAL SECURITY FIX): No-rescan invariant (NFR-007 / FR-051) enforced via SINGLE unified regex pass for Stages 1+2 substitution (`COMBINED_RE`); resolved values emitted directly to output buffer and never re-scanned, closing the data-exfiltration vector where a hostile plugin's `"version": "${TOME_ENV_GITHUB_TOKEN}"` could leak operator's env vars into LLM context
35. Phase 5 / US3 additions (STRUCTURAL SECURITY): Argument substitution (Stage 3) folded into the unified `COMBINED_RE` regex; caller-supplied args never recursive-substitute (no `$ARGUMENTS` output re-matched for `${TOME_*}`, no argument-value re-matched for `$N`); structural enforcement via single `captures_iter` loop with direct output emission — hostile argument values containing `${TOME_*}` or `$ARGUMENTS[N]` patterns cannot exfiltrate (Stage 3 is coerced once per render, not re-scanned); 0 security findings from US3 reviewer pass
36. Phase 5 / US4 additions (DoS MITIGATION): MCP `search_skills` description truncation via bounded O(max) `char_indices` walk (US4.d-1 S-M1 HIGH fix); eliminated O(n) full-string traversal that created meaningful CPU amplifier when caller-controlled `description_max_chars` (0–100,000 cap) × `top_k` results (1–100) × multi-KB descriptions ran over full input; walk stops after `max+1` chars, no reallocation in truncation path; no full-string traversal in no-truncation path
37. Phase 5 / US5 additions (CLOSURE OF PHASE 5 FEATURE WORK): Per-entry invocability flags (`user_invocable` column); `tome plugin show` displays command entries + resource enumeration (5-cap per directory + sentinels); doctor `--fix` extends to summariser re-download (5th repair class); symbolic link skip in `walk_resources` for resource enumeration; `Paths::plugin_data_root()` unified singleton source eliminates layout-drift risk; 0 security findings from Phase 5 / US5 reviewer pass
38. Phase 5 / Polish additions: All Phase 5 feature work critical security invariants (no-rescan, truncation DoS, path traversal, resource walk hardening) verified end-to-end; **0 HIGH / 0 MEDIUM / 0 LOW** security findings from full Phase 5 audit; 4 critical blockers fixed across US1–US5; `validate_db_stored_path` promoted as single SSOT for path-traversal boundary checks across all entry-body-reading surfaces; Paths::plugin_data_root() unified singleton verified across all callers; all Phase 5 contracts (exit-codes-p5, substitution-engine, mcp-tools-p5, entry-schema-p5) ratified
39. Phase 6 / US1 additions (NATIVE AGENTS): Plugin-supplied agent names validated as single safe path segment before becoming a filename (`is_safe_agent_name`); path-traversal vector closed (frontmatter `name: ../../evil` rejected before storing). Defence-in-depth: `target.parent() == Some(dir)` assertion before every agent write. Agent file writes reuse atomic, symlink-refusing, mode-preserving discipline (`write_standalone`). Plugin removal is literal prefix match (`plugin_of_owned_file`), not glob, scoped to directory walk. Privileged-field passthrough (hooks/mcpServers/permissionMode → `.claude/agents/`) is the intended FR-050 default, auditable in-file; striping opt-out (US5) surfaces privilege escalation report in doctor.
40. Phase 6 / US2 additions (REAL HOOKS — THIRD-PARTY JSON WRITE SURFACE): Plugin-supplied `hooks/hooks.json` read on plugin enable with two-variable rewrite (`${CLAUDE_PLUGIN_ROOT}` + `${CLAUDE_PLUGIN_DATA}` → absolute paths); UTF-8 validation (fail-closed exit 44) prevents non-UTF-8 install paths from emitting U+FFFD-corrupted hook commands. Hooks merged into project's `.claude/settings.local.json` (gitignored, never committed) by deep structural-equality match (idempotent, never duplicates user-edited copy). Settings file read/write atomic, symlink-refusing, mode-preserving; 1 MiB read-cap enforced. Trust gate is operator's explicit `tome plugin enable`; Tome never auto-enables. Parent `.claude/` created 0700 on Unix when absent. Hook JSON rewrite is targeted two-token textual substitution only (NOT full Phase 5 substitution pipeline per NFR-007); syntax-valid textual replacement handles double-slash in paths safely. On disable, hooks re-derived and structurally-matching entries removed only (no sidecar ownership tracking; user-edited copy stays). 0 security findings from US2 reviewer pass.
41. Phase 6 / US3 additions (GUARDRAILS SOFT FALLBACK + RULES-FILE CORRECTION): Plugin-shipped `hooks/GUARDRAILS.md` body copied verbatim into per-plugin marker regions in harness rules files; body validation enforces marker-injection defence (`body_contains_marker_line`): any line matching guardrails START/END regex or `tome:begin/end` block-marker regex rejected (exit 46, naming source) closing region-escape (persistent prose injection outside Tome's markers), file-wedge (exit-46-forever DoS), and rules-block corruption; loud-but-isolated (sibling plugins still reconcile). Guardrails write/delete targets (CLAUDE.md/AGENTS.md/GEMINI.md/Cursor sibling) atomic, symlink-refusing (exit 46 for guardrails targets per exit-codes-p6.md), mode-preserving. Path composition uses no attacker-influenced text in filesystem paths; `<catalog>:<plugin>` flows only into marker TEXT. Claude Code rules-include block and guardrails now land in `CLAUDE.md` (Phase 4 correction: was AGENTS.md) — same atomic/symlink/mode discipline, Claude Code now reads a file it actually reads; AGENTS.md shared across other harnesses resolves same `.tome/RULES.md` with no content duplication. 0 security findings from US3 reviewer pass.
42. Phase 6 / US4 additions (AGENT PERSONAS AS MCP PROMPTS): Double opt-in model for persona exposure: operator explicitly enables plugins via `tome plugin enable` (first gate), then optionally sets `expose_agents_as_personas = true` in project/workspace/global settings scope and MCP server is launched (second gate). Personas OFF by default (`expose_agents_as_personas` unset or `false`). Persona path introduces NO new substitution surface: agent bodies undergo identical Phase 5 `build_context_for_entry` + unified-regex substitution (COMBINED_RE) as non-persona entries; no Stage 4 appendage. Wrapped body (frontend layer) is LLM-context-only prose template (not re-parsed file); body text immutable after substitution. Persona MCP prompts folded into Phase 5 collision namespace (same `resolve_collisions` deduplication); drop-persona reserved (empty catalog/plugin/indexed_at sort) ensuring unhijackable insertion point. Persona name validation reuses agent-name checks at index time (safe path segment, no traversal). No security findings from US4 reviewer pass.
43. Phase 6 / US5 additions (PRIVILEGE GOVERNANCE + DOCTOR EXTENSIONS): **Privilege-escalation audit integrity (FR-051)**: the doctor `PrivilegeEscalationReport` reads each enabled agent's SOURCE `.md` (`CanonicalAgent::parse`) and lists those carrying `hooks`/`mcpServers`/`permissionMode`, REGARDLESS of `strip_plugin_agent_privileges` (the strip is a per-emission clone; the shared canonical the audit reads is never mutated — borrow-checker-enforced). A security-conscious admin always sees what a plugin ships even when Tome strips it on emit. **`strip_plugin_agent_privileges` is config governance, not an enforcement boundary** — the trust boundary remains plugin-enable time; the report still surfaces escalation. **`tome doctor --fix` write safety**: re-runs the idempotent `sync_project` with `force=false`; never removes a non-structural-match (user-edited) hook, never deletes user content (rules-file text outside markers; agents not matching `<plugin>__*`), refuses symlinks on every write/delete (single-path `remove_file`, no `remove_dir_all`). Hooks drift is reported, not auto-fixed. **Read-only doctor (FR-124)**: the five Phase 6 check fns + `build_phase6_surfaces` perform no writes / no dir creation. 0 security findings from US5 reviewer pass.

Security controls are enforced in code, tests, and CI—documented in `CONSTITUTION.md` and `specs/` contracts.

## Authentication & Authorization

### Authentication Method

| Method | Implementation | Notes |
|--------|----------------|-------|
| Inherited Git auth | System Git + credential helpers | Tome does not manage credentials (FR-026, XII) |
| No user auth | N/A | CLI is single-user, no multi-user access control |

### Credential Handling

| Control | Implementation | Location |
|---------|----------------|----------|
| **Scrubbing at boundary** | Regex-based pattern detector (R-8) | `src/catalog/git.rs::scrub_credentials` |
| **Never log secrets** | All `git` stderr passed through scrubber | `src/catalog/git.rs::scrub_to_string` |
| **HTTP error scrubbing** | `reqwest::Error` details scrubbed before surfacing | `src/embedding/download.rs::scrub_for_diag` |
| **MCP log scrubbing** | Workspace paths and error messages scrubbed before JSON logging | `src/mcp/tools/search_skills.rs`, `src/mcp/mod.rs` |
| **Model URL scrubbing** | Download URLs with presigned params scrubbed in error chains | `src/embedding/download.rs` (Phase 3 PR #36 + PR #54) |
| **Harness config scrubbing** | MCP config file paths scrubbed in error logs | Phase 4 harness modules |
| **Summariser URL scrubbing (Polish)** | Model download URLs with presigned params scrubbed before logging | `src/summarise/llama.rs` (PR-D additions) |
| **Harness MCP error chain scrubbing (Polish)** | Error messages from harness sync scrubbed before emission | `src/harness/sync.rs` (PR-D additions) |
| **No credential storage** | Inherit user's Git config entirely | Constitution XII |
| **No credential prompting** | Only system Git handles auth | Constitution XII, FR-026 |

The credential scrubber applies four ordered regex patterns to every byte stream from `git` and HTTP operations:
1. URL-embedded credentials: `https?://[^/@\s]+@` → `https://` (drops `user:token@`)
2. SSH login info: `git@[^\s:]+:` → `git@<host>:` (preserves host, scrubs login)
3. Key-value pairs: `(token|password|api[-_]?key|bearer|authorization|signature|x-amz-*)\s*[:=]\s*\S+` → `<scrubbed>` (includes AWS presigned-URL params)
4. Long hex (40+ chars outside safe context): `[0-9a-fA-F]{40,}\b` → `<scrubbed>` (except in `:` or `=` contexts where SHAs are preserved)

**Verification**: Comprehensive test coverage in `tests/scrubbing.rs` covers all four rules with worked examples. PR-D extended coverage to summariser URL + harness MCP error chains.

## Input Validation

### Plugin Source Path Validation (Catalog)

| Layer | Validation | Rules |
|-------|-----------|-------|
| **Manifest parse** | Semantic validation of `plugins[].source` | `src/catalog/manifest.rs::validate_source` |
| **Rejection criteria** | Six strict checks per data-model.md §3 | See FR-012, FR-013 |
| **Testing** | Exhaustive negative-case corpus | `tests/path_validation.rs` (11 test cases) |

**Validation Algorithm** (data-model.md §3, step 6):
1. **Reject URL-shaped values**: `contains("://")` or `starts_with("git@")`
2. **Reject absolute paths**: Unix (`is_absolute()`) and Windows (`C:` prefix)
3. **Reject parent traversal**: `components().any(|c| Component::ParentDir)`
4. **Resolve symlinks**: `canonicalize()` both plugin path and catalog root
5. **Validate bounds**: Resolved plugin path must `starts_with` resolved catalog root
6. **Error on escape**: Return `SourceEscapesRoot` if symlink points outside

**Test Coverage**: Every variant of `ManifestInvalid` has explicit test cases.

### Plugin Identity Validation

| Layer | Validation | Rules |
|-------|-----------|-------|
| **Parse barrier** | `PluginId::from_str` boundary | `src/plugin/identity.rs::validate_segment` |
| **Rejection criteria** | Seven strict checks | No `..`, no `.`, no `/`, no `\`, no leading `.`, no empty |
| **Testing** | Shape validation test | `tests/plugin_*.rs` integration suites |

**Validation Algorithm** (`src/plugin/identity.rs`, lines 48–66):
1. **Reject empty segments**: `segment.is_empty()`
2. **Reject embedded slashes**: `segment.contains('/')`
3. **Reject parent/current traversal**: `segment == ".."` or `segment == "."`
4. **Reject leading dot**: `segment.starts_with('.')`
5. **Reject absolute paths**: `segment.starts_with('/')` or `segment.starts_with('\\')` (Unix and Windows)

**Purpose**: Ensure plugin identities (`<catalog>/<plugin>`) are safe to compose into filesystem paths and cannot escape intended directory bounds.

### Agent Name Validation (Phase 6 / US1)

| Layer | Validation | Rules |
|-------|-----------|-------|
| **Index-time gate** | `is_safe_agent_name` validates before storing | `src/harness/agents.rs::is_safe_agent_name` |
| **Rejection criteria** | Single safe path segment: no `/`, `\`, `..`, `.`, leading `.`, NUL, or components > 1 | See FR-S-1 |
| **Mechanism** | `Path::components()` produces exactly one `Component::Normal` equal to the input | Robust backstop guards platform-specific separator behaviors |
| **Exit code** | Validation failure at enable time returns `AgentTranslationFailed` (exit 45) | Prevents storing invalid `name` in index |
| **Testing** | Negative-case corpus for traversal, separators, NUL | Phase 6 US1 integration tests |

**Validation Algorithm** (`src/harness/agents.rs`, lines 267–294):
1. **Reject empty**: `name.is_empty()`
2. **Reject NUL**: `name.contains('\u{0}')` (invalid in POSIX/Windows paths)
3. **Reject separators**: `name.contains('/')` or `name.contains('\\')`
4. **Reject traversal**: `name == "."` or `name == ".."` or `name.starts_with('.')`
5. **Robust backstop**: `Path::components()` must yield exactly one `Normal` component equal to input

**Purpose**: The emitted filename is `<plugin>__<name>.<ext>` joined to harness agent dir; a hostile `name` like `../../../../tmp/evil` would escape the directory unless blocked at index time.

**Examples of rejections**:
- `../../../../tmp/evil` → Rejected (contains `/` and traversal)
- `..` → Rejected (parent directory)
- `.hidden` → Rejected (leading dot)
- `a\b` → Rejected (backslash separator)
- `evil\u{0}` → Rejected (NUL byte)

### Entry Body-File Path Validation (Phase 5 / US1)

| Layer | Validation | Rules |
|-------|-----------|-------|
| **Parse barrier** | `validate_db_stored_path` (unified SSOT after Phase 5 Polish) | `src/index/skills.rs::validate_db_stored_path` |
| **Rejection criteria** | Rejects `..` parent-directory traversal and absolute paths | No `..` anywhere in path, not absolute on Unix/Windows |
| **Exit code** | Returns `SubstitutionFailed` (exit 28) on traversal | Prevents DoS via malicious entry paths in plugin manifests |
| **Testing** | Negative-case tests for traversal patterns | Phase 5 US1 integration tests + Phase 5 Polish verification |
| **Call sites** | 4 entry points all use unified validator | `resolve_entry_body_path` + MCP `get_skill` + `plugin show` resource walk + `Paths::plugin_data_root()` delegation |

**Purpose**: Skills and commands may reference body files (e.g., `src/utils.md` under skill directory). Validate paths to prevent `../../etc/passwd` escapes. Complements plugin-identity and plugin-source validation in the catalog-boundary model.

**Examples of rejections**:
- `../../etc/passwd` → `SubstitutionFailed`
- `/etc/passwd` → `SubstitutionFailed`
- `../sibling.md` → `SubstitutionFailed`
- `./subdir/body.md` → Accepted (relative, within bounds)
- `body.md` → Accepted (bare filename)

### Entry Arguments Parsing (Phase 5 / US1)

| Layer | Control | Limit |
|-------|---------|-------|
| **Frontmatter deserialiser** | Hard cap on `arguments` field list | 256 entries max (`MAX_ARGUMENTS` in `src/plugin/frontmatter.rs::deserialize_arguments`) |
| **Both parse forms** | Enforced for both space-separated string and YAML list forms | Same cap applied uniformly |
| **Exit code** | Exceeding cap returns `InvalidArgumentFrontmatter` (exit 29) at enable time | Prevents DoS: hostile catalog shipping 1 GiB YAML list |
| **Testing** | Boundary tests for 256-entry limit; beyond-limit rejection | Phase 5 US1 integration tests |

**Purpose**: Argument names are parsed at plugin-enable time. A malicious `plugin.json` could ship a YAML list with millions of entries, forcing unbounded allocation. The 256 cap is intentionally generous (every real-world prompt declares <10 named arguments); it bounds pathological input without constraining legitimate authoring.

**Implementation** (`src/plugin/frontmatter.rs`):
- Space-separated form: loop over `v.split_whitespace()`, check `out.len() >= MAX_ARGUMENTS` before push
- YAML sequence form: check inside `visit_seq`, reject beyond-limit with custom error message

### MCP Tool Input Bounds (Phase 5 / US4)

| Tool | Parameter | Limit | Exit Code | Location |
|------|-----------|-------|-----------|----------|
| `search_skills` | `query` length | 4096 Unicode chars (`MAX_QUERY_CHARS`) | N/A (input validation via MCP `invalid_params`) | `src/mcp/tools/search_skills.rs::153` |
| `search_skills` | `description_max_chars` | 100,000 chars cap (`MAX_DESCRIPTION_MAX_CHARS`) | N/A (input validation via MCP `invalid_params`) | `src/mcp/tools/search_skills.rs::137` |
| `search_skills` | `top_k` | 1–100 range | N/A | `src/mcp/tools/search_skills.rs::127` |

**Security Property**: Caller-controlled `description_max_chars` × `top_k` × result size cannot amplify CPU cost via description truncation. Truncation uses bounded O(max) `char_indices` walk (Phase 5 / US4.d-1 S-M1): walk stops after `max+1` chars, no full-string traversal in no-truncation path, no reallocation in truncation path.

### Workspace Name Validation (Phase 4)

| Layer | Validation | Rules |
|-------|-----------|-------|
| **Parse barrier** | `WorkspaceName::from_str` boundary | `src/workspace/name.rs::validate_grammar` |
| **Rejection criteria** | Alphanumeric + underscore only; no path separators or traversal | No `.`, `..`, `/`, `\` |
| **Testing** | Shape validation | Phase 4 US1/US2 integration tests |

**Purpose**: Workspace names are centrally registered; strict grammar prevents accidental path collisions or manipulation.

### Project Path Validation (Phase 4 / US1)

| Layer | Validation | Rules |
|-------|-----------|-------|
| **Canonicalisation** | `target_root.canonicalize()` before DB storage | `src/workspace/binding.rs::bind_project` (line 133) |
| **UTF-8 enforcement** | `path.to_str()` check; refuse non-UTF8 paths (exit 7) | `src/workspace/binding.rs::bind_project` (lines 140–148, R-B1 fix) |
| **Primary key constraint** | SQLite `workspace_projects(project_path TEXT PRIMARY KEY)` | `src/index/schema.rs` (no duplicate bindings) |
| **Dangerous-CWD refusal** | Refuse `$HOME` and `/` unless `--force` passed | `src/workspace/binding.rs::is_project_root_acceptable` |
| **Testing** | Negative cases for UTF-8, unsafe roots, canonicalisation failures | `tests/workspace_use_binding.rs` |

**Purpose**: Project paths are the PK for workspace binding; UTF-8 validation prevents lossy round-trip through DB and silent data loss (R-B1 security blocker fix).

### Plugin Hooks JSON Validation (Phase 6 / US2)

| Layer | Control | Implementation | Purpose |
|-------|---------|----------------|---------|
| **Presence check** | `hooks/hooks.json` is optional | Returns `Ok(None)` when absent (benign fall-through to guardrails) | `src/harness/hooks.rs::read_rewritten_entries` |
| **Read-size cap** | Bounded read at 1 MiB | `HARNESS_MCP_MAX` enforced by `crate::util::bounded_read_to_string` | Prevents DoS via huge hooks files |
| **JSON parse** | `serde_json::from_str` validation | Returns `HookSpecParseError` (exit 43) on malformed JSON | `src/harness/hooks.rs::read_rewritten_entries` |
| **Structure shape** | Top-level object keyed by event; each value an array of entries | Returns `HookSpecParseError` if not object-of-arrays | Validates expected nested shape |
| **UTF-8 path guard** | Non-UTF8 install paths fail closed, exit 44 | `non_utf8_guard(&plugin_root, &error_path)` prevents U+FFFD in hook commands | `src/harness/hooks.rs::non_utf8_guard` |
| **Two-variable rewrite** | Targeted textual substitution of `${CLAUDE_PLUGIN_ROOT}` + `${CLAUDE_PLUGIN_DATA}` only | Every other `${CLAUDE_*}` left verbatim for Claude Code to resolve | `src/harness/hooks.rs::rewrite_string_leaves` |
| **Rewrite scope** | String leaves in JSON tree only; keys and scalars untouched | Prevents accidental structural alteration | Recursive walk via `rewrite_string_leaves` |
| **Settings file read** | Bounded read at 1 MiB (`HARNESS_MCP_MAX`) | Prevents DoS via huge settings files | `src/harness/hooks.rs::load_settings` |
| **Deep equality match** | Structural identity via `serde_json::Value` equality | Idempotent merge (no duplicate user-edited copy) and precise removal (stale edits left in place) | `src/harness/hooks.rs::append_if_absent`, `remove_from_settings` |

**Validation Algorithm** (`src/harness/hooks.rs::read_rewritten_entries`):
1. **Refuse symlink**: `refuse_symlink(&source)?` at read entry point
2. **Bounded read**: `bounded_read_to_string(..., HARNESS_MCP_MAX)` (1 MiB cap)
3. **Parse**: `serde_json::from_str(&body)` or `HookSpecParseError`
4. **Shape check**: `doc.as_object_mut()` then iterate event keys
5. **Array check**: Each value must be an array
6. **Rewrite**: For each entry, `rewrite_string_leaves(&mut rewritten, plugin_root_str, plugin_data_str)`
7. **UTF-8 guard**: `non_utf8_guard(plugin_root, plugin_root)?` prevents non-UTF-8 paths (exit 44)

**Trust Model**: The trust gate is the operator's explicit `tome plugin enable` — Tome never auto-enables. Hooks are attacker-controlled JSON from the plugin's `hooks/hooks.json`, but only enabled plugins' hooks are merged into settings. The two-variable rewrite is a targeted, syntax-safe textual substitution that handles standard path patterns; no full substitution pipeline re-enters (NFR-007). The merged hooks execute under Claude Code's native hook protocol (outside Tome's scope).

**Examples of rewrite safety**:
- `/path//double/slash` → `/path//double/slash` (double-slash preserved; safe as part of path string)
- `${CLAUDE_PLUGIN_ROOT}/script.sh` → `/installed/root/script.sh` (correct)
- `${CLAUDE_PLUGIN_DATA}/cache` → `/home/user/.tome/plugin-data/<cat>/<plugin>/cache` (correct)
- `${CLAUDE_SESSION_ID}` → `${CLAUDE_SESSION_ID}` (left verbatim; Claude Code resolves at runtime)

### Guardrails Body Validation (Phase 6 / US3)

| Layer | Control | Implementation | Purpose |
|-------|---------|----------------|---------|
| **Presence check** | `hooks/GUARDRAILS.md` is optional | Returns `Ok(None)` when absent (benign fall-through; no guardrails region rendered) | `src/harness/guardrails.rs::read_guardrails_source` |
| **Symlink refusal** | Refuse symlink before read | `refuse_symlink(&source)?` returns exit 46 | `src/harness/guardrails.rs::read_guardrails_source` (line 130) |
| **Read-size cap** | Bounded read at `HARNESS_RULES_MAX` | `crate::util::bounded_read_to_string` prevents huge guardrails files | `src/harness/guardrails.rs::read_guardrails_source` (line 133) |
| **Marker-injection defence (B-1)** | Body scanned for managed marker lines before copying verbatim | `body_contains_marker_line` checks against guardrails START/END regexes + `tome:begin/end` block-marker regex | `src/harness/guardrails.rs::body_contains_marker_line` (lines 149–155) |
| **Rejection on marker match** | Any line matching any managed marker → `GuardrailsWriteFailed` (exit 46) | Naming the source file in the error for diagnostics | `src/harness/guardrails.rs::read_guardrails_source` (lines 139–141) |

**Marker-Injection Defence Detail** (FR-084, B-1 security blocker):

A guardrails body that itself contains a line resembling a managed marker (a guardrails START/END, or a Phase 4 `tome:begin/end` block marker) could:
- **Region-escape**: If the body contains `<!-- END GUARDRAILS: <key> -->`, it could prematurely end the region, allowing subsequent lines (or text from sibling plugins) to escape the marker pair
- **File-wedge DoS**: A stray `<!-- END` line with no matching START makes the parser fail on re-read, blocking future syncs (exit 46 forever)
- **Rules-block corruption**: If the body contains `<!-- tome:begin -->` or `<!-- tome:end -->`, it could interfere with the Phase 4 rules-include block parsing

**Defence Mechanism** (`src/harness/guardrails.rs::read_guardrails_source` + `body_contains_marker_line`):

1. On plugin enable, before copying the GUARDRAILS.md body verbatim, Tome scans every line against:
   - `START_REGEX` (matches the guardrails START marker pattern, e.g., `^<!-- START GUARDRAILS: .+ -->\s*$`)
   - `END_REGEX` (matches the guardrails END marker pattern, key-agnostic)
   - `BLOCK_MARKER_REGEX` (matches the Phase 4 `tome:begin` or `tome:end` line patterns)

2. If ANY line matches ANY of the three regexes, the read fails with `TomeError::GuardrailsWriteFailed` (exit 46), naming the source file.

3. The exact regexes used for validation are the same compiled regexes the reconciler parses with (`start_regex()`, `end_regex()`, `block_marker_regex()` in `src/harness/guardrails.rs`), ensuring the scan and parse can never disagree on what counts as a marker.

4. A plugin's guardrails body is therefore **never stored in the index** and is **never rendered to disk** if it contains any marker line.

**Examples of rejection**:
- Body contains `<!-- START GUARDRAILS: evil:plugin -->` → Rejected (line matches START_REGEX)
- Body contains `<!-- END GUARDRAILS: x:y -->` → Rejected (line matches END_REGEX)
- Body contains `<!-- tome:begin -->` → Rejected (line matches BLOCK_MARKER_REGEX)
- Body with trailing whitespace on a marker line → Rejected (regexes allow trailing whitespace; scan must too)
- Body with ordinary prose, headings, includes, no marker lines → Accepted (B-1 test case)

**Impact on Reconciliation**:

- On a failed marker validation, the plugin's guardrails region is not rendered to any harness's rules file.
- The reconciler continues to process sibling plugins' guardrails normally (loud-but-isolated error handling per FR-084).
- The error surfaces with the source file path in the exit message.
- User-authored guardrails regions in rules files are not affected (they are outside the Tome-managed marker pairs and are never re-parsed).

### Agent Personas Validation (Phase 6 / US4)

| Layer | Control | Implementation | Purpose |
|-------|---------|----------------|---------|
| **Opt-in gate (first)** | Operator explicitly enables plugin | `tome plugin enable` is the trust boundary | `src/commands/plugin/enable.rs` |
| **Opt-in gate (second)** | `expose_agents_as_personas` setting resolved at MCP startup | First-declarer-wins scalar walk (project → workspace → global, default false) | `src/mcp/mod.rs::resolve_expose_personas` |
| **Setting structure** | Three scopes carry `expose_agents_as_personas: Option<bool>` field | `GlobalSettings`, `WorkspaceSettings`, `ProjectMarkerConfig` all carry field | `src/settings/mod.rs` (all with `deny_unknown_fields`) |
| **Scalar resolution** | Single read at startup; no per-request re-resolution | `resolve_scalar_with` first-declarer-wins walk | `src/mcp/mod.rs::resolve_expose_personas` (called once during `build_prompt_registry`) |
| **Startup scope** | MCP server resolved against (project_root, workspace, global) at launch time | Non-project-scoped running server ignores project-scope value for subsequent requests | `src/mcp/mod.rs::build_prompt_registry` + contract `agent-personas.md` § Startup Scope |
| **Persona name validation** | Agent names already validated at index time as single safe path segment | Persona prompts reuse agent identities; no new name validation surface | `src/harness/agents.rs::is_safe_agent_name` (Phase 6 / US1) |
| **Body re-read protection** | Agent body re-read at persona-render time path-validated via `resolve_entry_body_path` | Path validation rejects `..` traversal and absolute paths (identical to non-persona entries) | `src/mcp/prompts.rs::wrap_persona_body` calls `resolve_entry_body_path` |
| **Substitution boundary** | Persona bodies undergo SAME Phase 5 `build_context_for_entry` + unified-regex substitution as non-persona entries | NO new substitution surface; `COMBINED_RE` single-pass enforces no-rescan invariant (NFR-007) for persona bodies too | `src/mcp/prompts.rs::collect_persona_identities` → `build_context_for_entry` |
| **Collision namespace** | Personas folded into Phase 5 `resolve_collisions` deduplication | `<name>-persona` or `<plugin>-<name>-persona` + drop-persona in single namespace | `src/mcp/prompt_collision.rs::resolve_collisions` |
| **Drop-persona reservation** | Global `drop-persona` reserved prompt inserted after agent loop | Empty catalog/plugin/indexed_at sort ensures agents process first; drop-persona appended last (unhijackable) | `src/mcp/prompts.rs::collect_persona_identities` → append drop-persona in `BTreeMap` (sorted iteration) |
| **Templating** | Persona body wrapped in role-assumption template at render time | Template is pure text (not re-parsed file); body immutable after substitution | `src/mcp/prompts.rs::wrap_persona_body` (lines 175–181) |

**Trust Model** (Phase 6 / US4): 
1. **Double opt-in**: Operator enables plugin (explicit trust); operator enables personas via settings (explicit feature gate)
2. **No new substitution surface**: Agent body → `build_context_for_entry` → unified-regex substitution (identical to Phase 5 non-persona entries)
3. **Path validation**: Body re-read at render time reuses `resolve_entry_body_path` (validates against `..` traversal, absolute paths)
4. **Limitation (acceptable)**: A persona body containing text that resembles the role-template closing tag (`</persona_name>`) could confuse the LLM context (in-band limitation, not a re-parsed file like guardrails). Documented as caveat in prompt description.
5. **0 security findings from US4 reviewer pass**.

### Manifest Strictness

| Rule | Implementation | Enforcement |
|------|----------------|-------------|
| **Unknown fields banned** | `#[serde(deny_unknown_fields)]` on all Tome-owned Deserialize structs | `src/catalog/manifest.rs`, `src/config.rs`, `src/embedding/registry.rs::ModelManifest`, `src/summarise/registry.rs`, `src/settings/mod.rs`, `src/workspace/binding.rs::ProjectMarkerConfig`, etc. |
| **Compile-time check** | Every Tome-owned Deserialize struct preceded by attribute | Verified by structural grep test |
| **Test enforcement** | `tests/manifest_strictness.rs` — assertion on 100% coverage | Test fails if any struct lacks attribute |
| **Phase 4 US4 audit** | T098n extended to `SummariserRegistry`, `CachedSummaries` (with deny check); `src/summarise/registry.rs::SUMMARISER_ENTRY` manually audited | Phase 4 complete; all Tome-owned types verified; zero missing |
| **Lenient third-party inputs** | `plugin.json` and `SKILL.md` frontmatter parsed without `deny_unknown_fields` (FR-013a); hooks JSON also lenient on forward-compat; guardrails body never parsed | Forward-compatible with upstream schema additions |
| **Coverage** | Strict targets: `CatalogManifest`, `Owner`, `PluginDeclaration`, `Config`, `CatalogEntry`, `ModelManifest`, `ModelKind`, `WorkspaceName`, `ProjectMarkerConfig`, all Phase 4 additions | Mandatory, no exceptions |

### Harness Configuration Validation (Phase 4 / US1.b + US5 + Polish)

| Control | Implementation | Location |
|---------|----------------|----------|
| **MCP config read-modify-write** | `toml_edit` for comment preservation on third-party TOML configs; `serde_json` with `preserve_order` for JSON | `src/harness/mcp_config.rs` |
| **JSON config validation** | `serde_json` with `preserve_order` feature for order preservation | Phase 4 harness modules |
| **Symlink rejection on write** | Refuses symlinks on RULES.md and MCP config write-back via `is_symlink()` check → exit 7 (FR-M-HRN-2) | `src/harness/rules_file.rs` line 79, `src/harness/mcp_config.rs` line 92 |
| **Ownership marker** | Entry is Tome-owned iff `command == "tome" && args[0] == "mcp"` (FR-501) | `src/harness/mcp_config.rs::is_tome_owned` |
| **User-owned MCP override (US5)** | Doctor `--fix --force` filters rewrites to only HarnessMcp entries with active SuggestedFix (S-M2 fix) | `src/doctor/fixes.rs::apply_one_fix` |
| **Config clash detection** | Harness clash errors surface on `tome workspace use` with hint to use `--force` | `src/error.rs::HarnessClash` (code 19); amended contract `mcp-config-integration.md` for env preservation semantics |
| **Mode preservation on rewrite** | Read existing target's mode before write; chmod staged tempfile to that mode before persist | `src/catalog/store.rs::write_atomic` (unified surface) + all callers (harness modules, workspace, project) |
| **Bounded read on project marker (Polish)** | `settings::parser::read_project_marker` uses `bounded_read_to_string` with per-class cap (PR-C consolidation) | `src/settings/parser.rs` (new canonical parser) |

### Doctor Command Input Validation (Phase 4 / US5 + Polish)

| Control | Implementation | Purpose |
|---------|----------------|---------|
| **MCP query length cap** | 4096 chars enforced at search_skills input boundary (FR-555) | `src/mcp/tools/search_skills.rs::106` validates before expensive compute |

## Real Hooks Settings File Write Security (Phase 6 / US2)

### File Target & Scope

| Control | Implementation | Guarantee |
|---------|----------------|-----------|
| **Settings file target** | Project's machine-local `.claude/settings.local.json` (gitignored) | Only local, machine-specific file written; committed `.claude/settings.json` never touched (FR-002) |
| **Parent directory** | `.claude/` created 0700 on Unix when absent; mode not changed if exists | Secure parent owned by user, readable/writable by user only |
| **Hook settings path** | Only Claude Code harness (via `m.hooks_strategy() == RealJson`) participates | Every other harness is `GuardrailsOnly`; no real-hooks participation for non-Claude-Code |

### Atomic Hooks Write Discipline

| Layer | Control | Guarantee |
|-------|---------|-----------|
| **Symlink refusal** | `refuse_symlink_settings(target)?` checks target before read/write | Exit 7 (Io); prevents writing through symlinks to `.claude/` system files |
| **Read → load settings document** | Existing file read as JSON object; missing file creates empty hooks object | Idempotent: second write with no change produces identical file |
| **Merge/Remove operation** | Deep structural-equality match using `serde_json::Value` | Idempotent: no duplicate appends, exact-match removal, user-edited copy preserved (NFR-003) |
| **Atomic persist** | Write to tempfile via `write_settings(target, &doc)` → POSIX rename | Crash mid-write leaves no partial file; same-FS rename is atomic |
| **Mode preservation** | Parent `.claude/` mode preserved (typically 0700); file created with tempfile default (0600 on Unix) or existing mode if rewrite | Permissions not silently downgraded |

**Code Location** (`src/harness/hooks.rs`):
```rust
/// Merge hooks into settings.local.json, atomic + mode-preserving + symlink-refusing.
pub fn merge_into_settings(target: &Path, hooks: &RewrittenHooks) -> Result<bool, TomeError> {
    refuse_symlink_settings(target)?;  // Exit 7 if symlink
    let (mut doc, existed) = load_settings(target)?;
    // ... merge logic with deep equality match ...
    if !existed || changed {
        write_settings(target, &doc)?;  // Atomic persist
    }
    Ok(changed)
}
```

### Hook Entry Merge & Removal

| Operation | Mechanism | Ownership |
|-----------|-----------|-----------|
| **Add (enabled plugin)** | For each event, append entry to array **only if no deep-equal entry exists** | `append_if_absent(hooks_obj, event, entry)` checks `serde_json::Value` equality |
| **Remove (disabled plugin)** | Re-derive plugin's rewritten entries, remove **only structurally-matching entries** by deep equality | Non-matching entries (user-edited post-Tome-write) left in place |
| **Empty pruning** | After removal, prune empty event arrays; otherwise-empty `hooks` object left | Container structure preserved for idempotence |

**Trust Model** (NFR-003): Ownership is **re-derivation + structural match only** — no sidecar provenance marker. A hook the user hand-edited after Tome wrote it no longer matches the re-derived entry and is conservatively left in place on removal. Tome never deletes a hook it cannot prove it owns.

**Example**: Plugin ships `hooks.json` with a `"postToolUse"` entry at enable time. Tome appends it to `settings.local.json`. User edits that entry (changes the command slightly). Later, plugin is disabled. Tome re-derives what it would have appended, finds no exact match (because user edited it), and leaves the entry in place. The user's hook remains active.

### Credentials in Hooks

| Surface | Scrubbing | Location |
|---------|-----------|----------|
| **Settings file reads** | No special scrubbing (operators manage secrets) | Hooks content is from trusted `plugin.json` sources; secret tokens in hook commands are operator-determined |
| **Error chains** | Error messages from hooks read/write/merge scrubbed via `scrub_credentials` regex | `src/harness/hooks.rs` error paths |
| **MCP log** | Harness sync errors scrubbed before JSON logging | `src/mcp/` log output |

**Design note**: The rewritten hooks become literal executed commands — if a hook command contains a secret (e.g., an API key), that secret is operator-supplied, not Tome's responsibility to scrub. The trust model assumes operators deliberately include secrets in hooks when that's correct for their use case.

## Guardrails and Rules-File Write Security (Phase 6 / US3)

### Guardrails Write Discipline

| Layer | Control | Guarantee |
|-------|---------|-----------|
| **Symlink refusal on read** | `refuse_symlink(&source)?` at read entry point | Exit 46 (GuardrailsWriteFailed); prevents reading through symlinks to source |
| **Symlink refusal on write** | `refuse_symlink(target)` checks target before read/write (in-file and sibling writes) | Exit 46; prevents writing to symlink targets |
| **Mode preservation** | Read existing target's mode before write; chmod staged tempfile to that mode before persist | `src/harness/rules_file.rs::atomic_write` path; permissions preserved on rewrite |
| **Atomic persist** | Write to tempfile → POSIX rename (via `rules_file::atomic_write`) | Crash mid-write leaves no partial file; same-FS rename is atomic |
| **Marker validation** | Body validated via `body_contains_marker_line` before copying verbatim | Exit 46 on marker match; prevents region-escape, file-wedge, rules-block corruption |
| **Deterministic ordering** | Regions ordered lexicographically by `<catalog>:<plugin>` key within file | Re-syncs never reorder existing content; idempotence verified |
| **In-place overwrite** | Existing region content overwritten between markers; no duplication | Re-synced region with new body replaces old body in place |
| **Orphan removal** | Regions for disabled plugins or unsuppressed-for-file plugins removed entirely (including preceding blank separator) | Per-plugin removal via marker-key match and orphaned-region detection |

**Code Location** (`src/harness/guardrails.rs`):
```rust
/// Reconcile guardrails regions in an in-file target (CLAUDE.md, AGENTS.md, GEMINI.md).
pub fn reconcile_in_file_region(
    target: &Path,
    desired: &BTreeMap<String, String>,
) -> Result<GuardrailsAction, TomeError> {
    refuse_symlink(target).map_err(|_| TomeError::GuardrailsWriteFailed { ... })?;
    // ... read, compose, validate, write via atomic_write ...
    Ok(action)
}

/// Validate body before copying verbatim.
pub fn read_guardrails_source(plugin_root: &Path) -> Result<Option<String>, TomeError> {
    let source = plugin_root.join("hooks").join("GUARDRAILS.md");
    refuse_symlink(&source).map_err(|_| TomeError::GuardrailsWriteFailed { ... })?;
    let body = bounded_read_to_string(&source, ...)?;
    if body_contains_marker_line(&body) {
        return Err(TomeError::GuardrailsWriteFailed { path: source });
    }
    Ok(Some(body))
}
```

### Claude Code Rules-File Correction (Phase 6 / US3)

| Control | Implementation | Change from Phase 4 |
|---------|----------------|---------------------|
| **Claude Code target (FR-020)** | Both rules-include block and guardrails regions target `CLAUDE.md` | Was `AGENTS.md` — Phase 4 latent error (Claude Code does not read `AGENTS.md` natively) |
| **Candidate precedence (FR-022)** | Claude Code candidates: `CLAUDE.md` > `.claude/CLAUDE.md` (first existing wins; create `CLAUDE.md` when none exist) | Previously listed `AGENTS.md` first; new correction makes Claude Code read a file it actually reads |
| **Shared AGENTS.md (FR-021)** | Codex, Gemini, OpenCode continue sharing one `AGENTS.md` rules-include block | Unchanged from Phase 4 |
| **No transitive import** | Tome does NOT create a chain where Claude Code imports AGENTS.md (FR-022) | Design constraint: both files resolve same `.tome/RULES.md` with no content duplication |
| **Both point to same rules file** | Claude Code's `CLAUDE.md` block + shared `AGENTS.md` block both resolve same `.tome/RULES.md` via the include directive (no duplicated rules) | Symmetry: one file, two harness-local entry points, no double-apply |

**Rationale**: Phase 4 mistakenly targeted `AGENTS.md` for Claude Code because the Codex/Gemini/OpenCode shared block was in `AGENTS.md`. Claude Code does not natively read `AGENTS.md` and never has — the shared block would be invisible to Claude Code users. The Phase 6 correction makes the rules-include block land in `CLAUDE.md` for Claude Code, while the other harnesses keep their shared `AGENTS.md` block. Both include directives point at the same `.tome/RULES.md`, so there is no content duplication and no reliance on Claude Code ever shipping native `AGENTS.md` support.

## Agent File Write Security (Phase 6 / US1)

### Safe Name Gate

| Control | Implementation | Guarantee |
|---------|----------------|-----------|
| **Index-time validation** | `is_safe_agent_name(name)` at parse time (exit 45 on fail) | Prevents storing invalid names in canonical agent struct |
| **Single safe segment** | `Path::components()` yields exactly one `Component::Normal` | Robust platform-independent check; no traversal tokens, separators, or NUL |
| **Filename composition** | `agent_filename(plugin, name, ext) → "<plugin>__<name>.<ext>"` | Double underscore provenance; filename is deterministic from validated name |

### Atomic Write Discipline

| Layer | Control | Guarantee |
|-------|---------|-----------|
| **Symlink refusal** | `refuse_symlink(target)` check before write | Exit 7; prevents writing through symlinks to unintended locations |
| **Atomic persist** | `write_standalone` via `tempfile` + POSIX rename | Crash mid-write leaves no partial file; same-FS rename is atomic |
| **Mode preservation** | Capture existing mode; chmod staged file before rename | Permissions preserved; no downgrade on rewrite |
| **Directory assertion** | `target.parent() == Some(dir)` verified before every write | Defence-in-depth: name validation alone isn't trusted; final target is checked |

**Code Location** (`src/harness/sync.rs`, lines 840–855):
```rust
let target = dir.join(&translated.filename);
// S-1 defence-in-depth: the agent `name` is validated as a single
// safe path segment at index time, but assert here too that the
// joined target stays directly inside `dir`.
if target.parent() != Some(dir) {
    if recon.first_error.is_none() {
        recon.first_error = Some(TomeError::AgentTranslationFailed {
            agent: format!("{}/{}", agent.canonical.plugin, agent.canonical.name),
        });
    }
    continue;  // Never write outside `dir`
}
match write_agent_file(&target, &translated.rendered) { ... }
```

### Plugin Ownership & Removal

| Control | Implementation | Purpose |
|---------|----------------|---------|
| **Provenance mechanism** | `<plugin>__<name>.<ext>` double-underscore separator (FR-040, R-19) | No ambiguity; no provenance frontmatter key (avoids harness parser confusion) |
| **Inverse recovery** | `plugin_of_owned_file(filename) → Option<&str>` (SSOT) | Single source of truth for ownership; reconciliation consumes this for both per-plugin removal and orphan cleanup (FR-043) |
| **Literal prefix match** | Split on `__`, validate non-empty plugin and stem, return plugin prefix | NOT a shell glob; scoped to `read_dir` entries only; cannot escape directory or widen to other plugins |
| **Removal scope** | Per-plugin removal iterates owned files, filters by literal prefix | Orphaned `<oldplugin>__<name>` files left until full plugin disable (US5 doctor `--fix` removes via `PrivilegeEscalationReport`) |

**Code Location** (`src/harness/agents.rs`, lines 522–533):
```rust
pub(crate) fn plugin_of_owned_file(filename: &str) -> Option<&str> {
    let (plugin, rest) = filename.split_once("__")?;
    if plugin.is_empty() {
        return None;
    }
    // Require a non-empty `<name>` before the extension dot.
    let stem = rest.rsplit_once('.').map(|(s, _)| s).unwrap_or(rest);
    if stem.is_empty() {
        return None;
    }
    Some(plugin)
}
```

### Privileged Field Passthrough & Governance (Phase 6 / US5)

| Control | Implementation | Location |
|---------|----------------|----------|
| **Default passthrough** | Privileged fields (`hooks`, `mcp_servers`, `permission_mode`) forwarded to Claude Code intact | `src/harness/agents.rs::CanonicalAgent` carries all three as opaque `serde_json::Value` |
| **In-file audit trail** | Each field written to emitted agent file; operators can inspect via `cat .claude/agents/<name>.md` | Transparency by design (FR-050) |
| **Opt-out stripping** | `strip_plugin_agent_privileges` setting (US5) drops all three fields on Claude Code agent emission | Operator control; governance responsibility to decide when to trust plugin-supplied hooks |
| **Privilege escalation report (US5)** | Doctor command surface `PrivilegeEscalationReport` listing all installed agents carrying privileged fields | Audit visibility: reads EACH ENABLED AGENT'S SOURCE `.md` via `CanonicalAgent::parse`, surfaces those with `hooks`/`mcpServers`/`permissionMode`, REGARDLESS of `strip_plugin_agent_privileges` setting (the strip is a per-emission clone; shared canonical never mutated — borrow-checker-enforced) |

**Security Model**: Trust boundary is at plugin enable time — operator explicitly authorizes the plugin and accepts whatever privileges it declares. The passthrough is intentional (FR-050: a capability advantage). The `PrivilegeEscalationReport` surfaces what was accepted, supporting governance workflows. **Admin visibility guarantee**: A security-conscious admin can always see what a plugin ships (via the report reading source files) even when Tome strips it on emit (local copy mutation). The strip is config governance only, not an enforcement boundary.

## Doctor Command Security (Phase 6 / US5)

### Privilege-Escalation Audit

| Control | Implementation | Guarantee |
|---------|----------------|-----------|
| **Read-only audit** | `PrivilegeEscalationReport` reads each enabled agent's SOURCE `.md` (canonical form) | No mutations to shared canonical; audit data is truth as shipped by plugin |
| **Stripping transparency** | Report surfaces privileges REGARDLESS of `strip_plugin_agent_privileges` setting | Admin always sees what plugin declared, independent of emission-time stripping |
| **Escalation listing** | Report lists all agents with `hooks`, `mcp_servers`, or `permission_mode` fields non-empty | Structured output for governance decisions and attestation |
| **Exit code** | Privilege escalation report emitted on `tome doctor` (read-only); no exit-code failure | Diagnostic only; non-blocking |

**Code Location** (`src/doctor/mod.rs` + `src/doctor/checks.rs`):
```rust
/// PrivilegeEscalationReport reads each enabled agent SOURCE canonical form
/// and lists those with privileged fields, regardless of strip setting.
pub struct PrivilegeEscalationReport {
    pub enabled_agents_with_privileges: Vec<PrivilegedAgentInfo>,
}

pub struct PrivilegedAgentInfo {
    pub plugin: String,
    pub agent_name: String,
    pub has_hooks: bool,
    pub has_mcp_servers: bool,
    pub has_permission_mode: bool,
}
```

### `--fix` Write Safety

| Control | Implementation | Guarantee |
|---------|----------------|-----------|
| **Idempotent re-sync** | `tome doctor --fix` re-runs `sync_project(workspace, project, force=false)` | Same harness sync pipeline; no new logic; idempotence verified by existing tests |
| **User-edited hook preservation** | Hooks re-derived; only structurally-matching ones removed (via deep equality) | User-edited copy (changed post-Tome-write) preserved on disable; no data loss (NFR-003) |
| **User-edited rules content preservation** | Rules-file text outside Tome marker pairs never touched | User-authored guardrails regions and prose remain intact; only marked regions reconciled |
| **Agent removal scope** | Only agents matching `<plugin>__*` pattern (via `plugin_of_owned_file`) removed on plugin disable | Stale agents left until plugin fully disabled; US5 doctor removes via `PrivilegeEscalationReport` listing orphans |
| **Symlink refusal on every write** | All file writes (rules, hooks, agents, guardrails) refuse symlinks before operation | Exit 46 for guardrails; exit 7 for others; no write-through-symlinks allowed |
| **No directory deletion** | `--fix` uses single-path `remove_file` for file-removal only; never `remove_dir_all` | Files removed individually; directories never force-deleted (defensive: prevent cascading accidents) |
| **Hooks drift reported, not auto-fixed** | Doctor surfaces stale hook entries in `HooksReport`; does not auto-remove | Operator reviews before manual cleanup; prevents silent orphaning of operator-intended hooks |

**Code Location** (`src/doctor/fixes.rs::apply_one_fix`):
```rust
/// Apply one suggested fix (e.g., re-sync harness, re-download model).
/// Re-runs idempotent sync_project; never removes non-structural-match hooks;
/// never deletes user content outside markers.
pub fn apply_one_fix(
    fix: &SuggestedFix,
    workspace: &WorkspaceName,
    project_root: &Path,
) -> Result<(), TomeError> {
    match fix {
        SuggestedFix::ReSyncHarness { harness, enabled } => {
            // Re-run same harness sync logic; idempotent
            sync_project(workspace, project_root, Some(harness), force=false)?;
        }
        // ... other fix types ...
    }
    Ok(())
}
```

### Read-Only Doctor (FR-124)

| Control | Implementation | Guarantee |
|---------|----------------|-----------|
| **No writes by default** | Five check functions + `build_phase6_surfaces` perform only reads | No file writes; no directory creation; no state mutation |
| **`--fix` opt-in** | Repairs only applied when user explicitly passes `--fix` flag | Diagnostic-by-default posture; user controls when mutations happen |
| **Pre-flight compute** | `build_phase6_surfaces` constructs reports without touching disk | All surfaces computed from in-memory state (index, settings, harness detection) |
| **Harness detection** | Non-parsing directory probe (look for `claude_code.toml`, etc.) | Never parses harness configs; no IO errors from parse failures; read-only detection |

**Code Location** (`src/doctor/checks.rs`):
```rust
/// Build all Phase 6 check surfaces without writing.
pub fn build_phase6_surfaces(...) -> Result<Phase6Surfaces, TomeError> {
    let mut surfaces = Phase6Surfaces::default();
    // Read-only: construct from index + settings + harness detection
    surfaces.privilege_escalation = report_privilege_escalation(&index)?;  // Read-only
    surfaces.persona = build_persona_report(&index, settings)?;           // Read-only
    surfaces.hooks = report_hooks_state(&project)?;                       // Read-only
    // No writes; no dir creation
    Ok(surfaces)
}
```

## Substitution Engine Security (Phase 5 / US2–US3, CRITICAL)

### No-Rescan Invariant (NFR-007 / FR-051) — Structural Fix

**CRITICAL SECURITY ARCHITECTURE**: The substitution engine enforces the no-rescan invariant via SINGLE unified regex pass for Stages 1 (built-ins), 2 (env passthrough), AND 3 (caller-supplied arguments).

**The Vulnerability** (US2.d B2 blocker):
- Naive two-pass design: Stage 1 scans `${TOME_*}` → resolves to string → Stage 2 re-scans result
- Hostile plugin scenario: `plugin.json` sets `"version": "${TOME_ENV_GITHUB_TOKEN}"`
- Stage 1 resolves: `PLUGIN_VERSION` → `"${TOME_ENV_GITHUB_TOKEN}"` (the literal string from manifest)
- Stage 2 re-scans: `"${TOME_ENV_GITHUB_TOKEN}"` → resolves to operator's actual GitHub token
- Result: operator's secret leaks into LLM context (e.g., skill prompt includes `${TOME_PLUGIN_VERSION}`)

**The Fix** (`src/substitution/regex_sets.rs::COMBINED_RE`):
- Single regex pattern: `\$\{TOME_(?:ENV_([A-Z0-9_]+)|([A-Z0-9_]+))(?::-(.*?))?\}|\$ARGUMENTS\[(\d+)\]|\$ARGUMENTS|\$(\d+)|\$([a-z_][a-z0-9_]*)`
- Six capture groups with leftmost-first alternation:
  - Group 1: `ENV_` branch (Stage 2; env-passthrough)
  - Group 2: Built-in branch (Stage 1; names)
  - Group 3: `:-default` (applies to Stage 1 or 2)
  - Groups 4–6: Stage 3 alternatives (`$ARGUMENTS[N]`, `$N`, `$<name>`)
  - Bare `$ARGUMENTS` has no capture group (dispatcher matches `m.as_str()`)
- Single `captures_iter` loop emits resolved values directly to output buffer (lines 161–238 of `src/substitution/mod.rs`)
- **Resolved values never re-enter the scanner** → exfiltration vector closed structurally

**Code Location**: `src/substitution/mod.rs::render()` (lines 115–262)
```rust
// Stages 1 + 2 + 3 unified pass — no intermediate re-scan
let re = regex_sets::combined_regex();
for caps in re.captures_iter(body) {
    // Per-match: exactly one of group 1 (env), 2 (builtin), 4/5/6 (arg) is set
    if let Some(env_name) = caps.get(1) {
        let value = env::resolve_env(env_name.as_str(), default);  // Stage 2
        out.push_str(&value);  // Direct emit; never re-scanned
    } else if let Some(builtin_name) = caps.get(2) {
        let value = builtins::resolve_builtin(...)?;  // Stage 1
        out.push_str(&value);  // Direct emit; never re-scanned
    } else if let Some(args) = resolved_args.as_ref() {
        // Stage 3: argument dispatch (details below)
        // Coerced argument values emitted directly; never re-scanned
    }
}
```

**Trust Boundary Definition**:
- **Input**: Entry body (skill or command YAML frontmatter, third-party authored)
- **Trusted operations**: Built-in paths (`${TOME_SKILL_DIR}`, `${TOME_WORKSPACE_DATA}`), env passthrough, caller arguments
- **Substituted output**: Rendered body fed to LLM/prompt context
- **Constraint**: Operator explicitly enables plugins (trusted decision); argument values NOT escaped (shell-style quoting per FR-043)
- **Documented limitation** (Phase 6+): Operators must not enable plugins from untrusted sources if plugins use `$ARGUMENTS` in high-privilege commands

**Verification**:
- `tests/substitution_pipeline.rs` — integration tests for unified pass
- `tests/substitution_env.rs` — env-passthrough tests
- `tests/substitution_builtins.rs` — built-in resolution tests
- `tests/substitution_arguments.rs` — Stage 3 argument dispatch tests
- Test assertion: no resolved value from any stage is fed back to `combined_regex()` for re-matching
- Phase 5 / US2 blocker (Closed C-B1): No-rescan invariant verified end-to-end
- Phase 5 / US3 (Closed): Argument substitution folded into unified regex; 0 security findings
- Phase 5 / Polish: Structural invariants verified across all test suites (1172 tests, 147 suites)

### Stage 1: Built-ins Resolution

**Registered Placeholders** (via `src/substitution/builtins.rs::resolve_builtin`):

| Placeholder | Resolves To | Scope | Side Effects |
|------------|------------|-------|-------------|
| `${TOME_SKILL_DIR}` | Entry directory (absolute path) | Per-entry | None |
| `${TOME_SKILL_PATH}` | Entry file (absolute path) | Per-entry | None |
| `${TOME_SKILL_NAME}` | Entry name (string) | Per-entry | None |
| `${TOME_PLUGIN_DIR}` | Plugin root (absolute path) | Per-plugin | None |
| `${TOME_PLUGIN_NAME}` | Plugin name (string) | Per-plugin | None |
| `${TOME_PLUGIN_VERSION}` | Version from `plugin.json` | Per-plugin | None |
| `${TOME_PLUGIN_DATA}` | Plugin data directory (absolute path) | Per-plugin | Creates `~/.tome/plugin-data/<catalog>/<plugin>/` via `create_dir_all` |
| `${TOME_CATALOG_NAME}` | Catalog name (string) | Per-catalog | None |
| `${TOME_WORKSPACE_NAME}` | Workspace name (string) | Per-workspace | None |
| `${TOME_WORKSPACE_DATA}` | Workspace data directory (absolute path) | Per-workspace | Creates `~/.tome/workspaces/<name>/plugin-data/<catalog>/<plugin>/` via `create_dir_all`; workspace-name validation applied (US2.d B1 blocker fix: error exit 25, not 9) |
| `${TOME_DATE}` | ISO 8601 date (YYYY-MM-DD) | At render time | None; uses `substitution::current_clock()` with `SUBSTITUTION_CLOCK_OVERRIDE` test seam |
| `${TOME_TIMESTAMP}` | RFC 3339 timestamp with offset | At render time | None |

**Unknown Names**: Pass through verbatim (e.g., `${TOME_FUTURE_NAME}` remains unchanged), emitting `tracing::debug!` per FR-023.

### Stage 2: Env Passthrough

**Reference Format**: `${TOME_ENV_<UPPERCASE_NAME>}`

**Resolution Algorithm** (`src/substitution/env.rs::resolve_env`):
1. Construct host key: `format!("TOME_ENV_{name}")`
2. Attempt `std::env::var(&key)`
3. **If set**: return host value (ignore `:-default`)
4. **If unset + default**: return default
5. **If unset + no default**: return empty string + log `tracing::debug!`

**Namespace Enforcement** (FR-033 + NFR-005):
- Leftmost-alternation in `COMBINED_RE` guarantees `${TOME_ENV_FOO}` always takes env branch
- Untyped `${GITHUB_TOKEN}`, `${PATH}` references NEVER match
- Operators must deliberately export `TOME_ENV_*` prefixed variables
- Governance responsibility: operators are stewards of `TOME_ENV_SECRET` exports (Phase 6+ operator guide documents this)

**Security Property**: Resolved value is a pure function of:
- Plugin metadata (`plugin.json`, fixed at enable time)
- Workspace/entry context (fixed at prompt render)
- Host environment (operator-controlled exports)

No transitive data exfiltration possible (Stage 1 output cannot leak into Stage 2).

### Stage 3: Argument Substitution (Phase 5 / US3)

**Reference Formats** (`src/substitution/arguments.rs`):

| Format | Example | Resolution |
|--------|---------|-----------|
| **Indexed positional** | `$ARGUMENTS[N]` | Nth positional argument (0-indexed) |
| **Bare positional join** | `$ARGUMENTS` | All positional arguments joined by space |
| **Positional shorthand** | `$N` | Nth positional argument (equivalent to `$ARGUMENTS[N]`) |
| **Named argument** | `$<name>` | Lookup by declared argument name |

**Coercion Algorithm** (`src/substitution/arguments.rs::coerce_arguments`):

The caller-supplied arguments are coerced once per render (not per-match) according to the entry's declared argument shape:

1. **Declared names (entry specifies `arguments: ["foo", "bar"]`)**
   - Caller supplies single string → shell-split into tokens; bind positionally to names
   - Caller supplies object with named keys → lookup by name; extra values are positional
   - Object with unknown keys → `PromptArgumentMismatch` (exit 26)

2. **Catch-all (entry specifies `arguments: "args"`)**
   - Caller supplies single string → keep as whole positional (no shell-split)
   - Caller supplies object → error (exit 26)

3. **No arguments (entry has no `arguments` field)**
   - Caller supplies any arguments → references like `$0` left verbatim (Stage 3 structurally skipped)

**Security Structural Fix** (`src/substitution/mod.rs::render()`):

The coerced `ResolvedArguments` are held in a single `resolved_args` variable (lines 144–147) created ONCE before the regex loop. Per-match dispatch in the loop (lines 197–221) looks up the resolved value and emits it directly to the output buffer:

```rust
// Coerce once, before loop (never re-scanned)
let resolved_args = match &context.args {
    Some(values) => Some(arguments::coerce_arguments(values, &context.declared_args)?),
    None => None,
};

for caps in re.captures_iter(body) {
    // ...
    } else if let Some(args) = resolved_args.as_ref() {
        // Stage 3: emit resolved value directly
        let (value, substituted) = arguments::apply_arguments_match(&caps, args);
        if substituted {
            out.push_str(&value);  // Direct emit; never re-scanned
        }
    }
}
```

**No Recursive Substitution**:
- A Stage-1 built-in resolving to `$0` cannot be hijacked by a Stage-3 positional substitution (Stage 1 output never re-entered the scanner)
- A caller-supplied argument value containing `${TOME_*}` or `$ARGUMENTS[N]` patterns cannot exfiltrate (argument values emitted directly; never re-scanned)
- Example: hostile caller supplies `arg0="${TOME_ENV_SECRET}"` → rendered as literal `"${TOME_ENV_SECRET}"` (not re-interpreted)

**Test Coverage** (`tests/substitution_arguments.rs` + `tests/substitution_pipeline.rs`):
- Indexed positional, bare join, shorthand, named dispatch tests
- Shell-split coercion tests (quoted strings, escapes)
- Named/positional mismatch tests
- No-rescan verification: `stage_1_output_cannot_be_hijacked_by_stage_3`
- Mixed-stage pipeline tests confirming output never re-enters scanner

### Default Value Support

Both stages support optional `:-fallback` syntax:
```
${TOME_ENV_VAR:-fallback}     → fallback if VAR unset
${TOME_SKILL_NAME:-default}   → Built-in always set; default unused but accepted
```

Per contract `contracts/substitution-engine.md` § Stage 2 table.

### Stage 4: ARGUMENTS Append Fallback (Phase 5 / US3)

**Trigger Conditions** (per `contracts/substitution-engine.md` § Stage 4):
- Caller supplied arguments AND Stage 3 reported zero replacements in the body

**Value** (per research §R-13):
- `Single("<string>")` → whole string verbatim
- `Object({...})` → positional values joined by single space (no shell-split reversal)

**Separator Policy** (`src/substitution/mod.rs::stage_4_value`):
- Body ends with `\n` → add one `\n` + `ARGUMENTS: `
- Body ends with non-`\n` → add two `\n`s + `ARGUMENTS: `

**Example**:
```
Body: "Hello, world!"
Args: ["foo", "bar"]
Result: "Hello, world!\n\nARGUMENTS: foo bar"
```

## File System Security

### Atomic Writes (All Phases)

| Subsystem | Pattern | Guarantee |
|-----------|---------|-----------|
| **Workspace/project init** | Staging dir → same-FS rename | POSIX-atomic; crash mid-populate leaves no debris (TempDir::drop) or orphan (cleaned by doctor) |
| **Harness config write** | `toml_edit` via `write_atomic` | Mode preserved; symlinks refused; tmpfile POSIX-rename |
| **Rules file write** | Block-insertion or standalone | Symlink refused; atomic persist |
| **Settings edits** | `toml_edit` via `write_atomic` | Mode preserved; symlinks refused |
| **Agent file write** | `write_standalone` (Phase 6 / US1) | Symlink refused; atomic persist; mode preserved |
| **Hooks settings write** | Read → merge → write via `write_settings` (Phase 6 / US2) | Atomic persist; mode preserved; idempotent |
| **Guardrails write** | Read → compose → write via `atomic_write` (Phase 6 / US3) | Symlink refused; atomic persist; mode preserved; marker-validated |
| **Model downloads** | Stream-to-partial → rename | Cleanup closure ensures partial removed on checksum mismatch |

### Symlink Refusal (Defense in Depth)

| Layer | Refusal Point | Exit Code | Location |
|-------|---------------|-----------|-----------
| Workspace/project binding | Marker path symlink check | Exit 7 (Io) | `src/workspace/binding.rs` |
| Harness rules file | Write-back symlink check | Exit 7 | `src/harness/rules_file.rs::refuse_symlink` |
| Harness MCP config | Write-back symlink check | Exit 7 | `src/harness/mcp_config.rs::refuse_symlink` |
| Atomic dir landing | Staging path symlink refusal | Exit 7 | `src/util/atomic_dir.rs::refuse_symlink` |
| **Hooks settings file (US2)** | **Write-back symlink check** | **Exit 7** | **`src/harness/hooks.rs::refuse_symlink_settings` (new US2 surface)** |
| **Agent file write** | **Write-back symlink check (Phase 6 / US1)** | **Exit 7** | **`src/harness/rules_file.rs::refuse_symlink` (reused for agents)** |
| **Guardrails source read (US3)** | **Source file symlink check** | **Exit 46** | **`src/harness/guardrails.rs::read_guardrails_source` (new US3 surface)** |
| **Guardrails target write (US3)** | **Target file symlink check** | **Exit 46** | **`src/harness/guardrails.rs::reconcile_in_file_region` + `reconcile_standalone_sibling` (new US3 surfaces)** |
| MCP get_skill walk | Directory symlink skip | Silent skip | `src/mcp/tools/get_skill.rs` (`is_symlink()` filter) |
| Doctor orphan cleanup | Symlink-skip in sweep | Silent skip | `src/doctor/orphan_cleanup.rs::sweep_one` (symlink_metadata check) |
| **MCP plugin show resource walk** | **Directory symlink skip (Phase 5 / US5)** | **Silent skip** | **`src/mcp/tools/plugin_show.rs::walk_resources` (`is_symlink()` filter; US5 new surface)** |

**Threat Model**: Hostile catalog clones `skills/creds → ~/.ssh/id_rsa`, operator runs `tome plugin enable` → would leak SSH key via skill content. **Mitigation**: symlink skip in MCP `get_skill` walk + symlink refusal on writes + resource enumeration skip in `plugin_show`. Phase 6 US1 extends to agent file writes: symlink refusal before emitting to `.claude/agents/`. Phase 6 US2 extends to hooks settings file writes: symlink refusal before reading/writing `.claude/settings.local.json`. Phase 6 US3 extends to guardrails source reads and target writes: symlink refusal for both directions, exit 46 dedicated to guardrails failures.

### File Mode Preservation

| Operation | Mode Capture | Mode Restoration | Location |
|-----------|--------------|------------------|----------|
| Harness config rewrite | `symlink_metadata(target)` before write | `chmod` staged tempfile before `persist` | `src/catalog/store.rs::write_atomic` (unified) |
| Workspace settings edit | Same | Same | `src/settings/edit.rs::save_settings` (via `write_atomic`) |
| Rules file rewrite | Same | Same | `src/harness/rules_file.rs::atomic_write` (via `write_atomic`) |
| **Hooks settings file (US2)** | **Existing file mode, or tempfile default (0600) for new** | **`chmod` staged file before `persist`** | **`src/harness/hooks.rs::write_settings` (new US2 surface)** |
| **Agent file write** | **Same (Phase 6 / US1)** | **Same** | **`src/harness/rules_file.rs::write_standalone` (reused for agents)** |
| **Guardrails target write (US3)** | **Existing file mode, or tempfile default for new** | **`chmod` staged file before `persist`** | **`src/harness/rules_file.rs::atomic_write` (reused for guardrails)** |
| Project marker write | Same | Same | `src/util/atomic_dir.rs::land_directory` (chmod 0o700 before keep) |

**Protection Against**: Silent permission downgrade (0o600 → 0o644) when a rewrite via `NamedTempFile::persist` doesn't preserve existing mode. Relevant for sensitive config files; tests verify via `tests/security_hardening.rs`.

## Model Integrity

### SHA-256 Verification

| Model | Registry | Verified Hash | Verification Point | Exit Code on Mismatch |
|-------|----------|---------------|-------------------|----------------------|
| Embedder (bge-small-en-v1.5) | `src/embedding/registry.rs::MODEL_REGISTRY` | `08fcd0fa…5e47c4f3` | `embedding::download::download_model` | Exit 32 (ModelChecksumMismatch) |
| Reranker (bge-reranker-base) | `src/embedding/registry.rs::MODEL_REGISTRY` | `2ef7c436…62b37627` | Same | Exit 32 |
| Summariser (Qwen2.5-0.5B-Instruct) | `src/summarise/registry.rs::SUMMARISER_ENTRY` | `74a4da8c…d7a9db` (Phase 4 blocker fix: real hash, not placeholder) | `summarise::llama::LlamaSummariser::new` | Exit 32 (via SubstitutionFailed or model init) |

**Verification Flow**:
1. Download to `<path>.partial`
2. Compute SHA-256 on complete file
3. Compare to registry value
4. On match: rename `.partial` → final destination
5. On mismatch: delete `.partial`, return error, emit warning

**Failure Mode**: File not usable until next `tome models download` or `tome plugin enable` (triggers auto-download).

## Logging & Observability

### MCP Server Logging (`src/mcp/log.rs`)

| Control | Implementation | Purpose |
|---------|----------------|---------|
| **File mode** | 0600 on Unix | Prevent other local users from reading workspace paths in log file |
| **Log rotation** | 10 MiB atomic-rename rotation (FileMakeWriter) | Bound disk usage; prevent DOS via repeated MCP restarts |
| **JSON lines** | One event per line; timestamp + structured fields | Machines can parse; no blob-log grep needed |
| **Credential scrubbing** | Workspace paths + error messages passed through `scrub_credentials` | Errors from harness sync don't leak secrets |
| **Error-only stderr sibling** | `error!` level also emitted to stderr (FR-222) | Harness sees critical failures on stderr; MCP protocol stays on stdout |

**Test Coverage**: `tests/mcp_log_format.rs` (contract-pinned JSON field names: `ts`, `msg`, etc.)

## Exit Codes & Error Semantics

**Closed set principle**: Every error class has enumerated exit code; no generic/unknown fallback.

**Phase 5 / US1–US5 additions**:
- Exit 25: `WorkspaceDataDirWriteFailed` (US2.d B1 blocker fix: was incorrectly routing through exit 9 PluginDataDirWriteFailed)
- Exit 26: `PromptArgumentMismatch`
- Exit 27: `EntryNotFound`
- Exit 28: `SubstitutionFailed` (body-path traversal, env var issues)
- Exit 29: `InvalidArgumentFrontmatter` (arguments list DoS cap, etc.)

**Phase 6 / US1 additions**:
- Exit 45: `AgentTranslationFailed` (malformed agent frontmatter, unsafe agent name, target-directory escape)

**Phase 6 / US2 additions**:
- Exit 43: `HookSpecParseError` (malformed / unparsable `hooks/hooks.json`, non-UTF-8 paths)
- Exit 44: `HookSettingsWriteFailed` (failure to read/merge/write `.claude/settings.local.json`)

**Phase 6 / US3 additions**:
- Exit 46: `GuardrailsWriteFailed` (failed guardrails render/write to rules files or Cursor sibling, marker-injection violation in plugin body, symlink refusal on guardrails source or target)

See `specs/005-phase-5-commands-prompts/contracts/exit-codes-p5.md` + `specs/006-phase-6-hooks-agents/contracts/exit-codes-p6.md` for full enumeration.

---

*This document defines security controls. Update when security posture changes.*
*Last refreshed 2026-05-29 against Phase 6 / US5 privilege governance + doctor extensions (incremental update); 0 security findings from US5 reviewer pass. US5 introduces: PrivilegeEscalationReport (reads enabled agents' source canonical forms, surfaces privileged fields REGARDLESS of strip_plugin_agent_privileges setting); read-only doctor (no writes by default, --fix opt-in); `--fix` write safety (idempotent re-sync, user-edited hook/rules preservation, single-path remove_file discipline, no remove_dir_all). Phase 6 / US1–US4 all verified; Phase 5 feature work critical security invariants (no-rescan, truncation DoS, path traversal, resource walk hardening) all verified end-to-end; 0 HIGH / 0 MEDIUM / 0 LOW security findings from full Phase 5 audit.*
