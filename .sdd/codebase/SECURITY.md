# Security

> **Purpose**: Document authentication, authorization, security controls, and vulnerability status.
> **Generated**: 2026-05-26
> **Last Updated**: 2026-05-26 (Phase 4 / US4 completed; 4 blockers + 9 majors applied in PR #97)

## Overview

Tome is a Rust CLI (and MCP server) for managing plugin catalogs, embeddings, workspace settings, project bindings, and workspace summarisation across multiple harnesses. As a synchronous, file-based tool without user authentication, security focuses on:

1. Preventing path traversal and directory-escape attacks via plugin source paths, plugin identities, workspace names, and project path canonicalization
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
| **MCP log scrubbing** | Workspace paths and error messages scrubbed before JSON logging | `src/mcp/tools/search_skills.rs` (line 272), `src/mcp/mod.rs` |
| **Model URL scrubbing** | Download URLs with presigned params scrubbed in error chains | `src/embedding/download.rs` (Phase 3 PR #36 + PR #54) |
| **Harness config scrubbing** | MCP config file paths scrubbed in error logs | Phase 4 harness modules |
| **No credential storage** | Inherit user's Git config entirely | Constitution XII |
| **No credential prompting** | Only system Git handles auth | Constitution XII, FR-026 |

The credential scrubber applies four ordered regex patterns to every byte stream from `git` and HTTP operations:
1. URL-embedded credentials: `https?://[^/@\s]+@` → `https://` (drops `user:token@`)
2. SSH login info: `git@[^\s:]+:` → `git@<host>:` (preserves host, scrubs login)
3. Key-value pairs: `(token|password|api[-_]?key|bearer|authorization|signature|x-amz-*)\s*[:=]\s*\S+` → `<scrubbed>` (includes AWS presigned-URL params)
4. Long hex (40+ chars outside safe context): `[0-9a-fA-F]{40,}\b` → `<scrubbed>` (except in `:` or `=` contexts where SHAs are preserved)

**Verification**: Comprehensive test coverage in `tests/scrubbing.rs` covers all four rules with worked examples.

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

### Harness Configuration Validation (Phase 4 / US1.b)

| Control | Implementation | Location |
|---------|----------------|----------|
| **MCP config read-modify-write** | `toml_edit` for comment preservation on third-party TOML configs; `serde_json` with `preserve_order` for JSON | `src/harness/mcp_config.rs` |
| **JSON config validation** | `serde_json` with `preserve_order` feature for order preservation | Phase 4 harness modules |
| **Symlink rejection on write** | Refuses symlinks on RULES.md and MCP config write-back via `is_symlink()` check → exit 7 (FR-M-HRN-2) | `src/harness/rules_file.rs` line 79, `src/harness/mcp_config.rs` line 92 |
| **Ownership marker** | Entry is Tome-owned iff `command == "tome" && args[0] == "mcp"` (FR-501) | `src/harness/mcp_config.rs::is_tome_owned` |
| **Config clash detection** | Harness clash errors surface on `tome workspace use` with hint to use `--force` | `src/error.rs::HarnessClash` (code 19); amended contract `mcp-config-integration.md` for env preservation semantics |
| **Mode preservation on rewrite** | Read existing target's mode before write; chmod staged tempfile to that mode before persist | `src/catalog/store.rs::write_atomic` (unified surface) + all callers (harness modules, workspace, project) |

### Workspace Scope Provider (Phase 4 / US3)

| Control | Implementation | Purpose |
|---------|----------------|---------|
| **Central DB membership check** | `CentralDbScopeProvider::workspace_is_registered` queries central `workspaces` table | Verifies workspace name is registered before attempting to read settings |
| **Replacement of PathsScopeProvider** | Production `harness::sync::sync_project` + `harness list <workspace>` now use `CentralDbScopeProvider` instead of `PathsScopeProvider` | Critical fix: production no longer uses `StubScope::new()` which always returned `UnknownWorkspace` for non-global workspaces |
| **Three-way classification** | (1) workspace not in registry → `UnknownWorkspace` (exit 13); (2) workspace exists, settings absent → `Ok(None)`; (3) workspace exists, settings present → `Ok(Some(list))` | Distinguishes "workspace doesn't exist" (exit 13) from "workspace exists but settings unreadable" (exit 70 `WorkspaceMalformed`) |
| **Bootstrap fallback** | When central DB absent (fresh install), only `WorkspaceName::global()` is considered registered | Allows production to function before first `tome workspace init` creates the central DB |
| **Implementation location** | `src/commands/harness/mod.rs::CentralDbScopeProvider` | Used by `harness::sync::sync_project` (line 178) and `harness list` subcommand |

## Data Protection

### Sensitive Data Handling

| Data Type | Protection | Storage | Phase 4 / US4 Changes |
|-----------|-----------|---------|---|
| Git credentials | Inherited from system Git config | Credential helper, not Tome | Unchanged |
| Model artefacts (embedder, reranker) | SHA-256 verification on download | `<home>/.tome/models/<name>/` | Layout unchanged; central registry enforced |
| Model artefacts (summariser) | SHA-256 verification on download + on-load (US4.d-1 C-B1 real hash) | `<home>/.tome/models/qwen2.5-0.5b-instruct/` | New Phase 4 US4; SHA-256 `74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db` verified 2026-05-26; size 491,400,032 bytes pinned |
| Configuration file | Atomic writes, chmod inherited from original on rewrite via mode-preservation | `<home>/.tome/` | Consolidated under single root; enrolment now in DB |
| Workspace settings | Layered composition with override semantics; atomic writes with mode preservation | `<workspace>/.tome/settings.toml` | New Phase 4 settings model; mode preserved on regen-summary |
| Project marker config | Atomic writes, chmod inherited from existing; workspace binding pointer at `<project>/.tome/config.toml` | `<project>/.tome/config.toml` | New Phase 4 project binding; mode preserved on rename |
| Central index DB | SQLite with `PRAGMA foreign_keys = ON` and `journal_mode = WAL` | `<home>/.tome/index.db` | Centralized DB (Phase 3 per-workspace → Phase 4 single central) |
| Catalog cache | Atomic refresh, ref-counted across workspaces/projects via DB table, re-used on same URL | `<home>/.tome/cache/<sha256-of-url>/` | Layout unchanged; ref-counting via `workspace_catalogs` table |
| Git stderr output | Scrubbed before tracing/display | `src/catalog/git.rs::scrub_credentials` | Unchanged |
| HTTP error output | Scrubbed before surfacing | `src/embedding/download.rs::scrub_for_diag` | Unchanged |
| MCP server logs | JSON-lines to file (0600 chmod), error-only stderr | `<home>/.tome/mcp.log` | New layout under single root |
| Workspace paths in logs | Scrubbed via `scrub_to_string` before emission | `src/mcp/tools/search_skills.rs` | Unchanged |
| Error messages in logs | Scrubbed via `scrub_to_string` before emission | `src/mcp/tools/search_skills.rs` | Unchanged |
| Workspace summaries | Cached short + long text; short embedded in MCP tool description broadcast to clients | `<workspace>/.tome/settings.toml` (under `[summaries]`) | New Phase 4 US4; length-capped (SHORT: 800 chars, LONG: 2500 chars per FR-425); warn-level logging on exceedance; values still cached |

### Model Registry & Integrity (Phase 4 / US4)

| Component | Mechanism | Values | Status |
|-----------|-----------|--------|--------|
| **Embedder** | SHA-256 checksum + size_bytes pin | `bge-small-en-v1.5` INT8: SHA `51f1bd0addd6e859e42c2c8021a5e5461385bb676a649f4b269aa445449f2431`, 66.5 MB, MIT | Phase 3 verified; real upstream digest |
| **Reranker** | SHA-256 checksum + size_bytes pin | `bge-reranker-base` INT8: SHA `46a1bb4cf46ff1e300d27589d620141fbf04fc0eaf8e7bb6dea5e044475ff387`, 279.3 MB, MIT (onnx-community mirror) | Phase 3 verified; real upstream digest |
| **Summariser (NEW)** | SHA-256 checksum + size_bytes pin (US4.d-1 C-B1 real hash) | `qwen2.5-0.5b-instruct` GGUF INT4: SHA `74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db`, 491,400,032 bytes, Apache 2.0 | US4.d-1 verified 2026-05-26 against canonical Hugging Face source; real upstream digest |
| **Placeholder detection** | `ModelEntry::has_placeholder_checksum()` guard | Rejects all-zero SHA (e.g., `0000000...000`) | Phase 4 F6 shipped placeholder; US4.d-1 C-B1 flipped to real hash; guard prevents regression (S-M3 belt-and-braces) |
| **Download gate** | Exit 31 (`ModelMissing`) if placeholder detected | Blocks install until real hash pinned; blocks `LlamaSummariser::new` with ModelMissing (silent trigger no-op per FR-420) | Phase 3–4 framework |
| **LlamaSummariser load** | SHA-256 verify + model load; gate on placeholder (S-M3 second-level check) | Return `SummariserFailure::ModelMissing` if registry pin is placeholder | US4.d-1 C-B1 + S-M3 defensive gate |
| **Model caching** | Loaded `LlamaModel` cached on `LlamaSummariser` instance; per-call `LlamaContext` (US4.d-1 S-M4) | SHA-256 + load run once at `new()`; subsequent `summarise()` calls reuse cached model | Performance: eliminates ~400 MB re-hash per trigger call |

### File Permissions

| File/Directory | Mode | Condition | Phase 4 / US4 Changes |
|---|---|---|---|
| `<home>/.tome/` | 0755 (created via `create_dir_all`) | Directory holding all Tome state | Single-root layout |
| `<home>/.tome/index.db` | Inherited from umask (typically 0644) | SQLite DB file; advisory lock prevents reader/writer races | Centralized DB (Phase 3 per-workspace → single central) |
| `<home>/.tome/index.lock` | 0644 | Advisory lockfile; write acquired by CLI commands and MCP preflight | Layout under single root |
| `<home>/.tome/mcp.log` | 0600 on Unix | MCP server log; chmod explicit per Phase 3 PR-F | Layout under single root |
| `<home>/.tome/models/` | Inherited from umask | Model downloads (embedder, reranker, summariser) | Layout under single root; summariser added in US4 |
| `<home>/.tome/cache/` | Inherited from umask | Catalog clones via SHA-256 addressing | Layout under single root; advisory lock now covers cache cleanup atomicity |
| `<workspace>/.tome/` | 0700 (before content) | Workspace marker directory | Phase 4 workspace binding |
| `<workspace>/.tome/config.toml` | Inherited from umask (typically 0644) | Workspace local config | Phase 4 workspace binding; mode preserved on rewrite |
| `<workspace>/.tome/settings.toml` | Inherited from original (if exists) or umask | Workspace layered settings + summaries; mode preserved on regen-summary rewrite (S-M3 via write_atomic) | New Phase 4 US4; unified mode preservation via write_atomic |
| `<project>/.tome/` | 0700 (atomic landing via `atomic_dir`) | Project marker directory; recovery branch on rename failure also chmods to 0o700 | Phase 4 project binding |
| `<project>/.tome/config.toml` | Inherited from original (if exists) or umask | Project binding pointer + optional RULES.md copy; mode preserved on rename and regen-summary rewrites | Phase 4 project binding; mode preservation via write_atomic |

**Unix-only hardening**: The 0600 chmod is applied explicitly to `mcp.log` via `std::os::unix::fs::OpenOptionsExt::mode()` on Unix; Windows' ACL model is not currently addressed.

### Symlink Handling

| Context | Control | Implementation | Location |
|---------|---------|----------------|----------|
| **Skill directory walk** | Skip symlinks (explicit rejection) | `entry.file_type()` + `is_symlink()` skip | `src/mcp/tools/get_skill.rs::walk_dir` (lines 272–289, FR-S-02) |
| **RULES.md write** | Refuse symlinks on write-back | `is_symlink()` check → exit 7 (FR-M-HRN-2) | `src/harness/rules_file.rs` line 79 |
| **MCP config write** | Refuse symlinks on write-back | `is_symlink()` check → exit 7 | `src/harness/mcp_config.rs` line 92 |
| **Settings file write** (Phase 4 / US3) | Refuse symlinks on write-back via `save_settings` | `is_symlink()` check in `catalog::store::write_atomic` → exit 7 | `src/settings/edit.rs::save_settings` → `src/catalog/store.rs::write_atomic` (lines 88–95) |
| **Atomic file writes** (US2.d-1 / US4) | Refuse symlinks in `catalog::store::write_atomic` | `is_symlink()` check on target before staging write → exit 7 | `src/catalog/store.rs::write_atomic` |
| **Purpose** | Prevent hostile catalog with `skills/foo/creds → ~/.ssh/id_rsa` or harness config pointing to sensitive files | Defence in depth: `lstat` (no follow) + explicit skip/refusal | Phase 3 PR #56; Phase 4 US1/US2/US4 extends to all atomic writes |

### Integrity & Verification

| Component | Mechanism | Enforcement |
|-----------|-----------|-------------|
| **Embedder downloads** | SHA-256 checksum + size_bytes pin | `src/embedding/download.rs::download_model` (exit 32 on mismatch) |
| **Reranker downloads** | SHA-256 checksum + size_bytes pin | `src/embedding/download.rs::download_model` (exit 32 on mismatch) |
| **Summariser downloads** | SHA-256 checksum + size_bytes pin (real hash US4.d-1 C-B1) | `src/embedding/download.rs::download_model` (exit 32 on mismatch) |
| **Registry pinning** | Compile-time constant `MODEL_REGISTRY` | `src/embedding/registry.rs::MODEL_REGISTRY` + `src/summarise/registry.rs::SUMMARISER_ENTRY` (real hash verified 2026-05-26) |
| **Placeholder detection** | `has_placeholder_checksum()` guard | `src/embedding/download.rs::download_model` (exit 31 if placeholder); `src/summarise/llama.rs::new` (ModelMissing if placeholder via S-M3) |
| **Atomic model persist** | `.partial/` → final rename | `src/embedding/download.rs::download_model`, step 4 |
| **Re-verification** | New `embedding::download::sha256_file()` helper | `src/embedding/download.rs::sha256_file`, invoked by `tome models list --verify` |
| **Summariser load verification** | SHA-256 re-checked at `LlamaSummariser::new()` before model load | `src/summarise/llama.rs::new` (lines 163–171); cached model avoids re-hash per trigger (US4.d-1 S-M4) |
| **Virtual table constraints** | `sqlite-vec` does not support `INSERT OR REPLACE`; uses `DELETE`-then-`INSERT` | `src/index/skills.rs::upsert_skill` |
| **Health check** | `tome status [--verify]` re-verifies installed models without re-downloading | `src/commands/status.rs::check_model()` |
| **MCP startup pre-flight** | SHA-256 verification of primary embedder file at every startup (FR-110) | `src/mcp/preflight.rs::verify_embedder_artefacts` |
| **Workspace initialization** | Atomic staging with permissions lock before content lands | `src/workspace/init.rs` |
| **Project binding** | Atomic `<project>/.tome/` landing via `atomic_dir::land_directory_with_replace`; UTF-8 path PK constraint | `src/workspace/binding.rs::bind_project` + `src/util/atomic_dir.rs` |
| **Catalog cache content trust** | Re-use existing clone on URL re-add; delete clone only when no scopes reference the URL (via DB table) | `src/index/workspace_catalogs.rs::refcount_by_url` (Phase 4, F11b) |
| **Schema migrations** | Forward-only migrations with per-step transaction atomicity under advisory lock | `src/index/migrations.rs::apply_pending` |
| **Central DB atomicity** | Advisory lock (`index.lock`) covers all DB writes; cache cleanup under lock (F11b FR-366); binding UPSERT + last_used_at bump atomic (R-M1 fix) | `src/index/lock.rs::with_lock()` |

## Phase 4 / US4 Security Enhancements (PR #94–#97)

### Summariser Model Integrity (US4.d-1 blockers + majors)

| Control | Implementation | Fix | Status |
|---------|----------------|-----|--------|
| **Real hash lands** (C-B1) | Qwen2.5-0.5B-Instruct SHA-256 `74a4da8c…d7a9db` + size 491,400,032 bytes verified 2026-05-26 against canonical Hugging Face; mirrored in `src/embedding/registry.rs` + `src/summarise/registry.rs` | Replace all-zero placeholder with real digest; add `tests/summariser_registry_no_placeholder.rs` (3 tests) to guard against regression | ✅ PR #97 (C-B1) |
| **Placeholder prevention** (S-M3) | `LlamaSummariser::new` explicitly rejects all-zero placeholder SHA with `ModelMissing`; second-level gate after download path's first-level gate | Belt-and-braces: surfaces as silent trigger no-op via FR-420 carve-out rather than running full ~400 MB SHA-256 + checksum-mismatch failure | ✅ PR #97 (S-M3) |
| **Model caching** (S-M4) | `LlamaSummariser` caches loaded `LlamaModel` on `self` after SHA-256 verify + `LlamaModel::load_from_file`; subsequent `summarise()` calls reuse cached model without re-hash/re-load | Eliminates 400 MB per-trigger SHA-256 hash + model load overhead; verified `LlamaModel: Send + Sync` upstream so `Summariser: Send + Sync` bound holds | ✅ PR #97 (S-M4) |
| **Length-window consolidation** (C-B3 / R-B1) | Unified `SHORT_MAX_CHARS = 800` and `LONG_MAX_CHARS = 2500` in `src/summarise/mod.rs` (previously split across `prompts.rs` with 2400 typo + `regen_summary` with 2500); LONG prompt instruction text bumped to match | Fix: warn boundaries now fire at same threshold (2500) matching contract; eliminates 100-char drift between warn trigger sites | ✅ PR #97 (C-B3 / R-B1) |
| **Exit code consistency** (C-B2) | Scrubbed all stale "exit 20" references (Phase 2 `PluginNotFound`); replaced with exit 24 (`TomeError::SummariserFailure`) across `src/error.rs`, `src/workspace/regen_summary.rs`, contracts | All summariser failures route through closed-enum `TomeError::SummariserFailure` with single exit code 24 | ✅ PR #97 (C-B2) |
| **Trigger test coverage** (T-B1) | Added `tests/summariser_triggers_end_to_end.rs` (2 tests) exercising `regenerate_for_trigger` through `SummariserOverrideGuard` thread-local slot; previously zero coverage of the override path | Test coverage: production trigger path now verified; `ModelMissing` silent-no-op + normal regen both tested | ✅ PR #97 (T-B1) |

### Summariser Prompt & Output (US4.d-1 majors)

| Control | Implementation | Fix | Status |
|---------|----------------|-----|--------|
| **Input formatting** (C-M1) | `format_input_descriptions` no longer prefixes each line with `"- "`; SHORT prompt explicitly tells model "no bullet points" | Removed contradictory bullet-example worked pattern that confused model inference | ✅ PR #97 (C-M1) |
| **Silent no-op carve-out** (C-M2) | Contract `summariser.md` now documents "ModelMissing" silent-no-op for trigger callers (enable/disable/reindex/catalog-update) vs. hard-fail for `regen-summary` per FR-420 corollary | Framework: production trigger callers explicitly treat `ModelMissing` as no-op; integration tests verify via `regenerate_for_trigger` | ✅ PR #97 (C-M2) |
| **Registry reference cleanup** (R-M2) | `LlamaSummariser::new` uses `summariser_entry()` from `src/summarise/registry.rs` instead of inline `MODEL_REGISTRY.iter().find(...).expect(...)` | Single source of truth for registry lookup; improves maintainability | ✅ PR #97 (R-M2) |
| **Cascade trigger pattern** (R-M6) | `tome catalog remove --force` cascade now calls `regenerate_for_trigger(scope.scope.name(), &paths)` after successful cascade-disable | Mirrors plugin-disable regen pattern; regenerate happens outside advisory lock (regen takes its own) | ✅ PR #97 (R-M6) |
| **Mutex poison recovery** (R-M7) | `backend()` recovers from mutex poisoning via `PoisonError::into_inner` instead of bubbling `BackendInitFailed` | Rationale: the init lock guards only one-shot `LlamaBackend::init()`; cross-thread panic shouldn't permanently disable summarisation | ✅ PR #97 (R-M7) |

### Summariser Test Coverage (US4.d-1 majors)

| Control | Test Location | Coverage | Status |
|---------|---|---|---|
| **Length-window warn** (T-M5) | `tests/workspace_regen_summary.rs::regen_summary_long_window_emits_warn_via_layer` | Custom `tracing-subscriber::Layer` captures warn on length exceedance without `tracing_test` dep or interest-cache hazard | ✅ PR #97 (T-M5) |
| **Silent no-op path** (T-M2) | `tests/summariser_triggers.rs::model_missing_trigger_is_silent_noop` | Production `regenerate_for_trigger` path explicitly asserts `ModelMissing` surfaces as silent no-op | ✅ PR #97 (T-M2) |
| **Placeholder regression** | `tests/summariser_registry_no_placeholder.rs` (3 tests) | Guard against re-introduction of all-zero placeholder; asserts real hash is pinned in both registry sources | ✅ PR #97 (C-B1 guard) |
| **Real summariser end-to-end** | `tests/summariser_real.rs` (refactored in PR #97) | Production `LlamaSummariser` with real model; tests input/output shapes + cache hit verification | ✅ Phase 4 US4 |

## Signal Handling & Interruption

| Control | Implementation | Location |
|---------|----------------|----------|
| **SIGINT handler** | Global `AtomicBool` flipped by `ctrlc` callback | `src/catalog/git.rs` (lines 25–29) |
| **In-flight cleanup** | Child processes killed on interrupt; `TomeError::Interrupted` returned | `src/catalog/git.rs` (FR-026a) |
| **MCP graceful shutdown** | SIGINT triggers cancellation token; 5-second timeout for in-flight handlers | `src/mcp/mod.rs` (lines 43–47, contracts/mcp-server.md) |
| **Error code** | Exit code 8 for interruption (Phase 1) | `src/error.rs` (TomeError::Interrupted) |

## Logging & Observability

### MCP Server Logging

| Component | Destination | Format | Rotation |
|-----------|-------------|--------|----------|
| **Structured logs** | `<home>/.tome/mcp.log` | JSON-lines per contract | 10 MiB cap, rotate to `.1` |
| **File permissions** | 0600 on Unix | N/A on Windows | `src/mcp/log.rs::open_appender` |
| **Stderr** | Fatal errors only | Human-readable | Filtered to `error!` level (FR-222) |
| **Scrubbing** | All user-sensitive fields | Via `scrub_to_string` | `src/mcp/tools/search_skills.rs`, `src/mcp/mod.rs` |

### Credential Scrubbing in MCP Logs

| Field | Scrubbing | Location |
|-------|-----------|----------|
| **workspace** (in startup event) | Scrubbed if Workspace scope via `scrub_to_string` | `src/mcp/mod.rs` (line 100) |
| **error** (in preflight failure) | Scrubbed via `scrub_to_string` before logging | `src/mcp/mod.rs` (line 88) |
| **error_message** (in tool error) | Scrubbed via `scrub_to_string` before logging | `src/mcp/tools/search_skills.rs` (line 272) |

## Dependency & Supply Chain Security

| Control | Implementation | Enforcement |
|---------|----------------|-------------|
| **Allowlist verification** | Explicit allowlist maintained in governance | `CONSTITUTION.md` (Principle VI) |
| **Audit scanning** | Weekly `cargo audit` in CI | `.github/workflows/` |
| **Deny rules** | Forbidden licenses and dep categories in `cargo-deny` | `Cargo.deny` |
| **MSRV pinning** | Minimum supported Rust version pinned and tested | `Cargo.toml` (rust-version = "1.93") |
| **Binary size cap** | 50 MB hard cap; currently ~30 MiB on macOS arm64 + Linux x86_64 (Phase 4 US4) | `CONSTITUTION.md` (NFR-001, revised 2026-05-13) |
| **LTO + strip** | `lto = "thin"`, `strip = "symbols"`, `panic = "abort"` | `Cargo.toml` (profile settings) |

### Phase 4 New Dependencies

| Crate | Version | Rationale | Security Notes |
|-------|---------|-----------|---|
| `llama-cpp-2` | 0.1.x (minor-pinned) | Summariser inference (Qwen2.5-0.5B); CPU-only static link | Pre-1.0; monitor for API changes; US4 first production use; model caching + placeholder gate defend against tampering |
| `toml_edit` | 0.25.x (minor-pinned) | Comment-preserving TOML edits for harness config + workspace settings preservation | Apache 2.0 / MIT; actively maintained; critical for US2 marker preservation + US3 settings-edit |

## Security Testing

| Category | Coverage | Location |
|----------|----------|----------|
| **Path validation** | 11 negative-case tests (URLs, absolute paths, traversal, symlinks) | `tests/path_validation.rs` |
| **Workspace name validation** | Phase 4 grammar tests | Phase 4 US1/US2 tests |
| **Project path validation** | UTF-8 enforcement, canonical path PK, dangerous-CWD refusal | `tests/workspace_use_binding.rs` |
| **Credential scrubbing** | 4 pattern rules + integration tests against real Git output | `tests/scrubbing.rs` |
| **Manifest strictness** | 100% grep assertion on `deny_unknown_fields`; Phase 4 US4 audit (T098n) includes summariser types | `tests/manifest_strictness.rs` |
| **Summariser model integrity** | Placeholder regression guard (3 tests); real-model SHA-256 verify + load; length-window warn; silent no-op carve-out | `tests/summariser_registry_no_placeholder.rs`, `tests/summariser_real.rs`, `tests/summariser_triggers.rs`, `tests/summariser_triggers_end_to_end.rs`, `tests/workspace_regen_summary.rs` |
| **Concurrency & atomicity** | Advisory lock + interrupt scenarios; cache cleanup under lock (F11b); binding + last_used_at atomic (R-M1) | `tests/atomicity.rs`, `tests/concurrency.rs` |
| **Exit codes** | Closed enumeration; all Phase 1/2/3/4 codes tested | `tests/exit_codes.rs` |
| **Security hardening** | File permissions, symlink handling, registry validation, mode preservation on rewrite, symlink refusal on atomic writes, settings-edit security | `tests/security_hardening.rs` |
| **MCP protocol purity** | No error leakage to stdout (FR-108) | `tests/mcp_server.rs` |
| **Workspace isolation** | Cross-workspace catalog enablement + reference-counting (Phase 3); project binding validation (Phase 4); settings composition (US3); summariser per-workspace (US4) | `tests/workspace_commands.rs`, `tests/catalog_cache_refcount.rs`, Phase 4 tests |
| **Sync idempotence** | Mtime stability across re-sync with all harness modules | `tests/sync_idempotence.rs` |
| **Harness concurrency** | Parallel HOME mutation via `HOME_MUTEX` (US3 PR #92) | `tests/harness_*.rs` + `tests/common/mod.rs::HomeGuard` |

---

*This document defines security controls. Update when security posture changes.*
