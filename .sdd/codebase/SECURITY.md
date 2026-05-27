# Security

> **Purpose**: Document authentication, authorization, security controls, and vulnerability status.
> **Generated**: 2026-05-27
> **Last Updated**: 2026-05-27 (Phase 5 / US3 complete; argument substitution secured via no-rescan invariant)

## Overview

Tome is a Rust CLI (and MCP server) for managing plugin catalogs, embeddings, workspace settings, project bindings, workspace summarisation, command/prompt entries, and harness synchronisation across multiple coding harnesses. As a synchronous, file-based tool without user authentication, security focuses on:

1. Preventing path traversal and directory-escape attacks via plugin source paths, plugin identities, workspace names, project paths, entry body paths, and harness configurations
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
15. No domain-error leakage in MCP tool responses (structured codes only)
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
33. Phase 5 / US1 additions: Entry body-file path validation rejects `..` traversal and absolute paths (prevents directory-escape via skill/command bodies); arguments list hard-capped at 256 entries in frontmatter parser (DoS mitigation)
34. Phase 5 / US2 additions (CRITICAL SECURITY FIX): No-rescan invariant (NFR-007 / FR-051) enforced via SINGLE unified regex pass for Stages 1+2 substitution (`COMBINED_RE`); resolved values emitted directly to output buffer and never re-scanned, closing the data-exfiltration vector where a hostile plugin's `"version": "${TOME_ENV_GITHUB_TOKEN}"` could leak operator's env vars into LLM context
35. **Phase 5 / US3 additions (STRUCTURAL SECURITY)**: Argument substitution (Stage 3) folded into the unified `COMBINED_RE` regex; caller-supplied args never recursive-substitute (no `$ARGUMENTS` output re-matched for `${TOME_*}`, no argument-value re-matched for `$N`); structural enforcement via single `captures_iter` loop with direct output emission — hostile argument values containing `${TOME_*}` or `$ARGUMENTS[N]` patterns cannot exfiltrate (Stage 3 is coerced once per render, not re-scanned); 0 security findings from US3 reviewer pass

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

### Entry Body-File Path Validation (Phase 5 / US1)

| Layer | Validation | Rules |
|-------|-----------|-------|
| **Parse barrier** | `resolve_entry_body_path` at substitution-engine entry point | `src/substitution/mod.rs` (Phase 5 US1) |
| **Rejection criteria** | Rejects `..` parent-directory traversal and absolute paths | No `..` anywhere in path, not absolute on Unix/Windows |
| **Exit code** | Returns `SubstitutionFailed` (exit 28) on traversal | Prevents DoS via malicious entry paths in plugin manifests |
| **Testing** | Negative-case tests for traversal patterns | Phase 5 US1 integration tests |

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

### Manifest Strictness

| Rule | Implementation | Enforcement |
|------|----------------|-------------|
| **Unknown fields banned** | `#[serde(deny_unknown_fields)]` on all Tome-owned Deserialize structs | `src/catalog/manifest.rs`, `src/config.rs`, `src/embedding/registry.rs::ModelManifest`, `src/summarise/registry.rs`, `src/settings/mod.rs`, `src/workspace/binding.rs::ProjectMarkerConfig`, etc. |
| **Compile-time check** | Every Tome-owned Deserialize struct preceded by attribute | Verified by structural grep test |
| **Test enforcement** | `tests/manifest_strictness.rs` — assertion on 100% coverage | Test fails if any struct lacks attribute |
| **Phase 4 US4 audit** | T098n extended to `SummariserRegistry`, `CachedSummaries` (with deny check); `src/summarise/registry.rs::SUMMARISER_ENTRY` manually audited | Phase 4 complete; all Tome-owned types verified; zero missing |
| **Lenient third-party inputs** | `plugin.json` and `SKILL.md` frontmatter parsed without `deny_unknown_fields` (FR-013a) | Forward-compatible with upstream schema additions |
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
| **Model downloads** | Stream-to-partial → rename | Cleanup closure ensures partial removed on checksum mismatch |

### Symlink Refusal (Defense in Depth)

| Layer | Refusal Point | Exit Code | Location |
|-------|---------------|-----------|-----------
| Workspace/project binding | Marker path symlink check | Exit 7 (Io) | `src/workspace/binding.rs` |
| Harness rules file | Write-back symlink check | Exit 7 | `src/harness/rules_file.rs::refuse_symlink` |
| Harness MCP config | Write-back symlink check | Exit 7 | `src/harness/mcp_config.rs::refuse_symlink` |
| Atomic dir landing | Staging path symlink refusal | Exit 7 | `src/util/atomic_dir.rs::refuse_symlink` |
| MCP get_skill walk | Directory symlink skip | Silent skip | `src/mcp/tools/get_skill.rs` (`is_symlink()` filter) |
| Doctor orphan cleanup | Symlink-skip in sweep | Silent skip | `src/doctor/orphan_cleanup.rs::sweep_one` (symlink_metadata check) |

**Threat Model**: Hostile catalog clones `skills/creds → ~/.ssh/id_rsa`, operator runs `tome plugin enable` → would leak SSH key via skill content. **Mitigation**: symlink skip in MCP `get_skill` walk + symlink refusal on writes.

### File Mode Preservation

| Operation | Mode Capture | Mode Restoration | Location |
|-----------|--------------|------------------|----------|
| Harness config rewrite | `symlink_metadata(target)` before write | `chmod` staged tempfile before `persist` | `src/catalog/store.rs::write_atomic` (unified) |
| Workspace settings edit | Same | Same | `src/settings/edit.rs::save_settings` (via `write_atomic`) |
| Rules file rewrite | Same | Same | `src/harness/rules_file.rs::atomic_write` (via `write_atomic`) |
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

**Phase 5 / US2 additions**:
- Exit 25: `WorkspaceDataDirWriteFailed` (US2.d B1 blocker fix: was incorrectly routing through exit 9 PluginDataDirWriteFailed)
- Exit 26: `PromptArgumentMismatch`
- Exit 27: `EntryNotFound`
- Exit 28: `SubstitutionFailed` (body-path traversal, env var issues)
- Exit 29: `InvalidArgumentFrontmatter` (arguments list DoS cap, etc.)

**Phase 5 / US3 additions**:
- No new exit codes (uses Phase 5 / US2 codes: 26 for mismatch, 28 for substitution issues)

See `specs/005-phase-5-commands-prompts/contracts/exit-codes-p5.md` for full enumeration.

---

*This document defines security controls. Update when security posture changes.*
*Last refreshed 2026-05-27 against Phase 5 / US3 complete source (1000+ tests passing, ~130 suites); argument substitution secured via unified no-rescan invariant.*
