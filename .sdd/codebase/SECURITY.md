# Security

> **Purpose**: Document authentication, authorization, security controls, and vulnerability status.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-25 (Phase 4 / US1 + US2 + US3 completed; all security audit findings applied)

## Overview

Tome is a Rust CLI (and MCP server) for managing plugin catalogs, embeddings, workspace settings, and project bindings across multiple harnesses. As a synchronous, file-based tool without user authentication, security focuses on:

1. Preventing path traversal and directory-escape attacks via plugin source paths, plugin identities, workspace names, and project path canonicalization
2. UTF-8 validation for project paths stored in the central DB (primary key constraint)
3. Integrity verification for downloaded model artefacts (SHA-256 checksums)
4. Symlink refusal on all workspace/project/harness file writes (defence in depth)
5. File mode preservation on atomic rewrites to prevent silent permission downgrades (S2-M1 fix)
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
26. Workspace removal cascade per-project effective list narrowing (US2.d-1 C-B1 fix)
27. Workspace rename with DB transaction wrapping marker rewrites (US2.d-1 C-B2 fix)
28. Workspace settings and rules file preservation of forward-compat fields (US2.d-1 T-B1 fix, `toml_edit`)
29. Credential scrubbing on MCP log fields and error chains
30. **Phase 4 US1 additions**: Single-root `<home>/.tome/` layout with all state under one directory; central SQLite DB replacing per-scope files; workspace + project binding model with PK on canonical project_path (TEXT); harness MCP config management with symlink-aware writes; summariser inference with Qwen2.5-0.5B; settings composition with layered override; atomic-dir helper for populated-directory landing; mode-preserving atomic file writes
31. **Phase 4 US2 additions**: Workspace removal effective-list narrowing; rename with transaction wrapping; marker field preservation via `toml_edit`
32. **Phase 4 US3 additions (PR #92)**: Production sync now uses `CentralDbScopeProvider` instead of `StubScope`; advisory lock covers harness use/remove read-modify-write window; `PathsScopeProvider` replaced by `CentralDbScopeProvider` for central-DB workspace membership checks with distinct error handling (IO errors vs. unknown workspace); settings-edit mode preservation + symlink refusal regression tests added

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
| **Unknown fields banned** | `#[serde(deny_unknown_fields)]` on all Tome-owned Deserialize structs | `src/catalog/manifest.rs`, `src/config.rs`, `src/embedding/registry.rs::ModelManifest`, `src/settings/mod.rs`, `src/workspace/binding.rs::ProjectMarkerConfig`, etc. |
| **Compile-time check** | Every Tome-owned Deserialize struct preceded by attribute | Verified by structural grep test |
| **Test enforcement** | `tests/manifest_strictness.rs` — assertion on 100% coverage | Test fails if any struct lacks attribute |
| **Phase 4 audit** | T098n extended to `WorkspaceSettings`, `CachedSummaries`, `CatalogEntry` (settings), `ProjectMarkerConfig`, `GlobalSettings` | All Phase 4 Tome-owned types verified; zero missing |
| **Lenient third-party inputs** | `plugin.json` and `SKILL.md` frontmatter parsed without `deny_unknown_fields` (FR-013a) | Forward-compatible with upstream schema additions |
| **Coverage** | Strict targets: `CatalogManifest`, `Owner`, `PluginDeclaration`, `Config`, `CatalogEntry`, `ModelManifest`, `ModelKind`, and all Phase 4 additions | Mandatory, no exceptions |

### Harness Configuration Validation (Phase 4 / US1.b)

| Control | Implementation | Location |
|---------|----------------|----------|
| **MCP config read-modify-write** | `toml_edit` for comment preservation on third-party TOML configs; `serde_json` with `preserve_order` for JSON | `src/harness/mcp_config.rs` |
| **JSON config validation** | `serde_json` with `preserve_order` feature for order preservation | Phase 4 harness modules |
| **Symlink rejection on write** | Refuses symlinks on RULES.md and MCP config write-back via `is_symlink()` check → exit 7 (FR-M-HRN-2) | `src/harness/rules_file.rs` line 79, `src/harness/mcp_config.rs` line 92 |
| **Ownership marker** | Entry is Tome-owned iff `command == "tome" && args[0] == "mcp"` (FR-501) | `src/harness/mcp_config.rs::is_tome_owned` |
| **Config clash detection** | Harness clash errors surface on `tome workspace use` with hint to use `--force` | `src/error.rs::HarnessClash` (code 19); amended contract `mcp-config-integration.md` for env preservation semantics |
| **Mode preservation on rewrite** | Read existing target's mode before write; chmod staged tempfile to that mode before persist (S2-M1 fix) | `src/catalog/store.rs::write_atomic` (unified surface) + all callers (harness modules, workspace, project) |

### Workspace Scope Provider (Phase 4 / US3 fix — PR #92)

| Control | Implementation | Purpose |
|---------|----------------|---------|
| **Central DB membership check** | `CentralDbScopeProvider::workspace_is_registered` queries central `workspaces` table | Verifies workspace name is registered before attempting to read settings |
| **Replacement of PathsScopeProvider** | Production `harness::sync::sync_project` + `harness list <workspace>` now use `CentralDbScopeProvider` instead of `PathsScopeProvider` | C-B1 + S-M1 fix: production no longer uses `StubScope::new()` which always returned `UnknownWorkspace` for non-global workspaces |
| **Three-way classification** | (1) workspace not in registry → `UnknownWorkspace` (exit 13); (2) workspace exists, settings absent → `Ok(None)`; (3) workspace exists, settings present → `Ok(Some(list))` | C-B2 fix: distinguishes "workspace doesn't exist" (exit 13) from "workspace exists but settings unreadable" (exit 70 `WorkspaceMalformed`) |
| **Bootstrap fallback** | When central DB absent (fresh install), only `WorkspaceName::global()` is considered registered | Allows production to function before first `tome workspace init` creates the central DB |
| **Implementation location** | `src/commands/harness/mod.rs::CentralDbScopeProvider` | Used by `harness::sync::sync_project` (line 178) and `harness list` subcommand |

## Data Protection

### Sensitive Data Handling

| Data Type | Protection | Storage | Phase 4 Changes |
|-----------|-----------|---------|---|
| Git credentials | Inherited from system Git config | Credential helper, not Tome | Unchanged |
| Model artefacts | SHA-256 verification on download | `<home>/.tome/models/<name>/` | New layout under single root |
| Configuration file | Atomic writes, chmod inherited from original on rewrite via mode-preservation (S2-M1) | `<home>/.tome/` | Consolidated under single root; enrolment now in DB |
| Workspace settings | Layered composition with override semantics; atomic writes with mode preservation | `<workspace>/.tome/settings.toml` | New Phase 4 settings model; mode preserved on regen-summary |
| Project marker config | Atomic writes, chmod inherited from existing; workspace binding pointer at `<project>/.tome/config.toml` | `<project>/.tome/config.toml` | New Phase 4 project binding; mode preserved on rename |
| Central index DB | SQLite with `PRAGMA foreign_keys = ON` and `journal_mode = WAL` | `<home>/.tome/index.db` | New centralized DB (Phase 3 per-workspace → Phase 4 single central) |
| Catalog cache | Atomic refresh, ref-counted across workspaces/projects via DB table, re-used on same URL | `<home>/.tome/cache/<sha256-of-url>/` | New layout; ref-counting via `workspace_catalogs` table |
| Git stderr output | Scrubbed before tracing/display | `src/catalog/git.rs::scrub_credentials` | Unchanged |
| HTTP error output | Scrubbed before surfacing | `src/embedding/download.rs::scrub_for_diag` | Unchanged |
| MCP server logs | JSON-lines to file (0600 chmod), error-only stderr | `<home>/.tome/mcp.log` | New layout under single root |
| Workspace paths in logs | Scrubbed via `scrub_to_string` before emission | `src/mcp/tools/search_skills.rs` | Unchanged |
| Error messages in logs | Scrubbed via `scrub_to_string` before emission | `src/mcp/tools/search_skills.rs` | Unchanged |
| Summariser model artefacts | SHA-256 verification on download | `<home>/.tome/models/qwen2.5-0.5b-instruct/` | New Phase 4 summariser; sentinel SHA-256 in F6, real hash in US4 |

### File Permissions

| File/Directory | Mode | Condition | Phase 4 Changes |
|---|---|---|---|
| `<home>/.tome/` | 0755 (created via `create_dir_all`) | Directory holding all Tome state | New single-root layout |
| `<home>/.tome/index.db` | Inherited from umask (typically 0644) | SQLite DB file; advisory lock prevents reader/writer races | New centralized DB (Phase 3 per-workspace → single central) |
| `<home>/.tome/index.lock` | 0644 | Advisory lockfile; write acquired by CLI commands and MCP preflight | New layout under single root |
| `<home>/.tome/mcp.log` | 0600 on Unix | MCP server log; chmod explicit per Phase 3 PR-F | New layout under single root |
| `<home>/.tome/models/` | Inherited from umask | Model downloads | New layout under single root |
| `<home>/.tome/cache/` | Inherited from umask | Catalog clones via SHA-256 addressing | New layout under single root; advisory lock now covers cache cleanup atomicity (FR-366) |
| `<workspace>/.tome/` | 0700 (before content) | Workspace marker directory | New Phase 4 workspace binding |
| `<workspace>/.tome/config.toml` | Inherited from umask (typically 0644) | Workspace local config | New Phase 4 workspace binding; mode preserved on rewrite |
| `<project>/.tome/` | 0700 (atomic landing via `atomic_dir`) | Project marker directory; recovery branch on rename failure also chmods to 0o700 (S2-M4 fix) | New Phase 4 project binding |
| `<project>/.tome/config.toml` | Inherited from original (if exists) or umask | Project binding pointer + optional RULES.md copy; mode preserved on rename and regen-summary rewrites (S2-M1 fix) | New Phase 4 project binding |

**Unix-only hardening**: The 0600 chmod is applied explicitly to `mcp.log` via `std::os::unix::fs::OpenOptionsExt::mode()` on Unix; Windows' ACL model is not currently addressed.

**Phase 4 US2 note**: File mode preservation (S2-M1) lifted into `catalog::store::write_atomic` — the single atomic-write surface used by all workspace/project file writers (`init`, `rename`, `regen_summary`, `sync` project-marker rewrites). Recovery branch on rename failure also chmods central workspace dir to 0o700 (S2-M4 fix).

### Symlink Handling

| Context | Control | Implementation | Location |
|---------|---------|----------------|----------|
| **Skill directory walk** | Skip symlinks (explicit rejection) | `entry.file_type()` + `is_symlink()` skip | `src/mcp/tools/get_skill.rs::walk_dir` (lines 272–289, FR-S-02) |
| **RULES.md write** | Refuse symlinks on write-back | `is_symlink()` check → exit 7 (FR-M-HRN-2) | `src/harness/rules_file.rs` line 79 |
| **MCP config write** | Refuse symlinks on write-back | `is_symlink()` check → exit 7 | `src/harness/mcp_config.rs` line 92 |
| **Settings file write** (Phase 4 / US3) | Refuse symlinks on write-back via `save_settings` | `is_symlink()` check in `catalog::store::write_atomic` → exit 7 | `src/settings/edit.rs::save_settings` → `src/catalog/store.rs::write_atomic` (lines 88–95) |
| **Atomic file writes** (US2.d-1 S2-M2) | Refuse symlinks in `catalog::store::write_atomic` | `is_symlink()` check on target before staging write → exit 7 | `src/catalog/store.rs::write_atomic` |
| **Purpose** | Prevent hostile catalog with `skills/foo/creds → ~/.ssh/id_rsa` or harness config pointing to sensitive files | Defence in depth: `lstat` (no follow) + explicit skip/refusal | Phase 3 PR #56; Phase 4 US1.d-2a + US2.d-1 + US3 extends to all atomic writes |

### Integrity & Verification

| Component | Mechanism | Enforcement |
|-----------|-----------|-------------|
| **Model downloads** | SHA-256 checksum + size_bytes pin | `src/embedding/download.rs::download_model` (exit 32 on mismatch) |
| **Registry pinning** | Compile-time constant `MODEL_REGISTRY` | `src/embedding/registry.rs::MODEL_REGISTRY` (verified real for BGE; Qwen SHA-256 placeholder in F6, real in US4) |
| **Placeholder detection** | `has_placeholder_checksum()` guard | `src/embedding/download.rs::download_model` (exit 31 if placeholder) |
| **Atomic model persist** | `.partial/` → final rename | `src/embedding/download.rs::download_model`, step 4 |
| **Re-verification** | New `embedding::download::sha256_file()` helper | `src/embedding/download.rs::sha256_file`, invoked by `tome models list --verify` |
| **Virtual table constraints** | `sqlite-vec` does not support `INSERT OR REPLACE`; uses `DELETE`-then-`INSERT` | `src/index/skills.rs::upsert_skill` |
| **Health check** | `tome status [--verify]` re-verifies installed models without re-downloading | `src/commands/status.rs::check_model()` |
| **MCP startup pre-flight** | SHA-256 verification of primary embedder file at every startup (FR-110) | `src/mcp/preflight.rs::verify_embedder_artefacts` |
| **Workspace initialization** | Atomic staging with permissions lock before content lands | `src/workspace/init.rs` |
| **Project binding** | Atomic `<project>/.tome/` landing via `atomic_dir::land_directory_with_replace`; UTF-8 path PK constraint | `src/workspace/binding.rs::bind_project` + `src/util/atomic_dir.rs` |
| **Catalog cache content trust** | Re-use existing clone on URL re-add; delete clone only when no scopes reference the URL (via DB table) | `src/index/workspace_catalogs.rs::refcount_by_url` (Phase 4, F11b) |
| **Schema migrations** | Forward-only migrations with per-step transaction atomicity under advisory lock | `src/index/migrations.rs::apply_pending` |
| **Central DB atomicity** | Advisory lock (`index.lock`) covers all DB writes; cache cleanup under lock (F11b FR-366); binding UPSERT + last_used_at bump atomic (R-M1 fix) | `src/index/lock.rs::with_lock()` |

**Model Registry** (Phase 3 + Phase 4):
- `bge-small-en-v1.5` INT8: SHA-256 `51f1bd0addd6e859e42c2c8021a5e5461385bb676a649f4b269aa445449f2431`, 66.5 MB, MIT
- `bge-reranker-base` INT8: SHA-256 `46a1bb4cf46ff1e300d27589d620141fbf04fc0eaf8e7bb6dea5e044475ff387`, 279.3 MB, MIT (sourced from `onnx-community` mirror)
- `qwen2.5-0.5b-instruct` GGUF INT4: SHA-256 **placeholder** (sentinel in F6; real hash lands in US4), ~400 MB, Apache 2.0

Both BGE checksums are real upstream digests verified at Phase 3 slice 1. Downloads enforce both hash and size; drift surfaces as `ModelChecksumMismatch` (exit 32) rather than silently installing whatever upstream serves.

### Phase 4 / US2 Security Enhancements

| Control | Implementation | Location | Fix |
|---------|----------------|----------|-----|
| **File mode preservation** (S2-M1) | Unified in `catalog::store::write_atomic`; all workspace/project file writers use this surface; mode inherited from original target or umask | `src/catalog/store.rs::write_atomic` (lines 81–110) | Lifted from US1.d-2a harness-only fix; now covers workspace settings, project markers, rules files |
| **Symlink refusal** (S2-M2) | Extended to `catalog::store::write_atomic`; all atomic writes refuse symlinks | `src/catalog/store.rs::write_atomic` (lines 88–95) | Unified defence across all file write surfaces |
| **Cascade effective-list narrowing** (S2-C-B1) | `workspace remove --force` cascade now reads per-project marker, resolves effective harness list via `settings::resolver::resolve_effective_list`, and only tears down harnesses in that list | `src/workspace/remove.rs::teardown_integration_for_project` (PR US2.d-1) | Security: prevents unconditional iteration of all harness dirs on every remove |
| **Marker rewrite under transaction** (S2-C-B2) | `workspace rename` now opens `conn.transaction()` BEFORE marker rewrite loop; SQL UPDATE failure still leaves DB inconsistent w.r.t. marker state, but DB row and old markers stay consistent | `src/workspace/rename.rs::rename` (PR US2.d-1) | Partial-failure mode documented; atomicity guarantee at transaction boundary |
| **Workspace dir chmod on recovery** (S2-M4) | `workspace rename` recovery branch (old dir absent) now chmods the new workspace dir to 0o700 before populating, matching init's security posture | `src/workspace/rename.rs::rename` (lines 269–274, PR US2.d-1) | Closes permission-downgrade vulnerability during recovery |
| **Marker field preservation** (S2-T-B1) | `workspace rename` now uses `toml_edit::DocumentMut` to preserve optional `harnesses` field + comments in project markers during rewrite | `src/workspace/rename.rs::rename` (PR US2.d-1) | Forward-compat: marker round-trip preserves user-supplied fields |

### Phase 4 / US3 Security Enhancements (PR #92)

| Control | Implementation | Location | Fix |
|---------|----------------|----------|-----|
| **Production sync uses central-DB provider** (C-B1 + S-M1) | `harness::sync::sync_project` now constructs `CentralDbScopeProvider` instead of `StubScope::new()`; workspace-reference composition works correctly in production | `src/harness/sync.rs` lines 178–185 | Critical: production previously bypassed workspace resolution, returning `UnknownWorkspace` for all non-global workspaces even when they existed |
| **ScopeProvider replaced** (C-B2 + S-M4 + R-M1) | `PathsScopeProvider` replaced by `CentralDbScopeProvider` across all production call sites; central DB queries confirm workspace membership; IO/parse errors return distinct error codes | `src/commands/harness/mod.rs::CentralDbScopeProvider` (lines 120–150) | Fix: distinguishes "workspace doesn't exist" (exit 13) from "IO/parse error" (exit 70) — errors no longer masked as `UnknownWorkspace` |
| **Concurrent harness use/remove safe** (C-M5 + R-M2 + S-M2) | Advisory lock at `paths.index_lock` held across entire read-modify-write window in `harness use` and `harness remove` | `src/commands/harness/use_.rs` lines 69–70, `src/commands/harness/remove.rs` lines 53–54 | Race-safe: concurrent edits to settings.toml are serialised; contention returns `IndexBusy` (exit 50) |
| **Settings write mode preservation** (S-M3) | `save_settings` routes through `write_atomic` which preserves file mode on rewrite; tested in `tests/security_hardening.rs::preserve_file_mode_on_workspace_settings_via_settings_edit` | `src/settings/edit.rs::save_settings` → `src/catalog/store.rs::write_atomic` | Unified defence: same mode-preservation policy as all other atomic writes |
| **Settings write symlink refusal** (S-M3) | `save_settings` refuses symlinks via `write_atomic::is_symlink()` check; tested in `tests/security_hardening.rs::refuses_symlink_on_settings_edit` | `src/settings/edit.rs::save_settings` → `src/catalog/store.rs::write_atomic` (lines 88–95) | Unified defence: prevents hostile redirection of settings writes to sensitive files |
| **Test isolation on HOME mutation** (T-B1) | Process-global `HOME` mutation serialised via `HOME_MUTEX` + `HomeGuard` RAII in `tests/common/mod.rs`; harness tests acquire mutex before mutation | `tests/common/mod.rs::HOME_MUTEX`, used by `harness_bare.rs`, `harness_info.rs`, `harness_use_scope.rs` | Race-safe: parallel tests no longer collide on `std::env::set_var("HOME", ...)`|

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
| **Binary size cap** | 50 MB hard cap; currently ~30 MiB on macOS arm64 + Linux x86_64 (Phase 4) | `CONSTITUTION.md` (NFR-001, revised 2026-05-13) |
| **LTO + strip** | `lto = "thin"`, `strip = "symbols"`, `panic = "abort"` | `Cargo.toml` (profile settings) |

### Phase 4 New Dependencies

| Crate | Version | Rationale | Security Notes |
|-------|---------|-----------|---|
| `llama-cpp-2` | 0.1.x (minor-pinned) | Summariser inference (Qwen2.5-0.5B); CPU-only static link | Pre-1.0; monitor for API changes; no CUDA/Metal features enabled |
| `toml_edit` | 0.25.x (minor-pinned) | Comment-preserving TOML edits for harness config + workspace settings preservation | Apache 2.0 / MIT; actively maintained |

## Security Testing

| Category | Coverage | Location |
|----------|----------|----------|
| **Path validation** | 11 negative-case tests (URLs, absolute paths, traversal, symlinks) | `tests/path_validation.rs` |
| **Workspace name validation** | Phase 4 grammar tests | Phase 4 US1/US2 tests |
| **Project path validation** | UTF-8 enforcement, canonical path PK, dangerous-CWD refusal | `tests/workspace_use_binding.rs` |
| **Credential scrubbing** | 4 pattern rules + integration tests against real Git output | `tests/scrubbing.rs` |
| **Manifest strictness** | 100% grep assertion on `deny_unknown_fields`; Phase 4 audit (T098n) on 5 new types | `tests/manifest_strictness.rs` |
| **Concurrency & atomicity** | Advisory lock + interrupt scenarios; cache cleanup under lock (F11b); binding + last_used_at atomic (R-M1) | `tests/atomicity.rs`, `tests/concurrency.rs` |
| **Exit codes** | Closed enumeration; all Phase 1/2/3/4 codes tested | `tests/exit_codes.rs` |
| **Security hardening** | File permissions, symlink handling, registry validation, mode preservation on rewrite (S2-M1), symlink refusal on atomic writes (S2-M2), settings-edit mode + symlink tests (US3 PR #92) | `tests/security_hardening.rs` |
| **MCP protocol purity** | No error leakage to stdout (FR-108) | `tests/mcp_server.rs` |
| **Workspace isolation** | Cross-workspace catalog enablement + reference-counting (Phase 3); project binding validation (Phase 4); settings composition (US3) | `tests/workspace_commands.rs`, `tests/catalog_cache_refcount.rs`, Phase 4 tests |
| **Sync idempotence** | Mtime stability across re-sync with all harness modules | `tests/sync_idempotence.rs` |
| **Harness concurrency** | Parallel HOME mutation via `HOME_MUTEX` (US3 PR #92) | `tests/harness_*.rs` + `tests/common/mod.rs::HomeGuard` |

---

*This document defines security controls. Update when security posture changes.*
