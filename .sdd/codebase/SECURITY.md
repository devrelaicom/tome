# Security

> **Purpose**: Document authentication, authorization, security controls, and vulnerability status.
> **Generated**: 2026-05-26
> **Last Updated**: 2026-05-26 (Phase 4 v0.4.0 Polish complete via `/sdd:map incremental`)

## Overview

Tome is a Rust CLI (and MCP server) for managing plugin catalogs, embeddings, workspace settings, project bindings, workspace summarisation, and harness synchronisation across multiple coding harnesses. As a synchronous, file-based tool without user authentication, security focuses on:

1. Preventing path traversal and directory-escape attacks via plugin source paths, plugin identities, workspace names, project paths, and harness configurations
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
| **`--force` flag scoping** | Only applies `--fix` repairs; `--force` without `--fix` → exit 2 (Usage) per R-M1 | `src/commands/doctor.rs` validates upfront |
| **Read-only diagnostic paths** | `check_model`, `check_index`, `check_drift` use `open_read_only` (R-M7 fix) | `src/doctor/checks.rs`, `src/doctor/binding.rs` |
| **Home path validation (Polish)** | `home_root()` validates absolute, canonical, exists; relative/unset `$HOME` → exit 2 Usage (PR-E additions) | `src/util/io.rs::home_root` (new canonical validator) |

### Workspace Scope Provider (Phase 4 / US3)

| Control | Implementation | Purpose |
|---------|----------------|---------|
| **Central DB membership check** | `CentralDbScopeProvider::workspace_is_registered` queries central `workspaces` table | Verifies workspace name is registered before attempting to read settings |
| **Replacement of PathsScopeProvider** | Production `harness::sync::sync_project` + `tome harness *` commands now use `CentralDbScopeProvider` instead of `PathsScopeProvider` | Critical fix: production no longer uses `StubScope::new()` which always returned `UnknownWorkspace` for non-global workspaces |
| **Three-way classification** | (1) workspace not in registry → `UnknownWorkspace` (exit 13); (2) workspace exists, settings absent → `Ok(None)`; (3) workspace exists, settings present → `Ok(Some(list))` | Distinguishes "workspace doesn't exist" (exit 13) from "workspace exists but settings unreadable" (exit 70 `WorkspaceMalformed`) |
| **Bootstrap fallback** | When central DB absent (fresh install), only `WorkspaceName::global()` is considered registered | Allows production to function before first `tome workspace init` creates the central DB |
| **Implementation location** | `src/commands/harness/mod.rs::CentralDbScopeProvider` | Used by `harness::sync::sync_project` (line 178) and `harness list` subcommand |

### Bounded String Reading (Polish Addition)

| Control | Implementation | Limits by Class |
|---------|----------------|------------------|
| **Per-class caps** | `util::bounded_read_to_string(path, limit)` with per-caller configurable limit | Index DB XML (10 KiB), project markers (16 KiB), settings files (256 KiB), catalog cache (unlimited for now) |
| **Applied across ~26 sites** | Progressively applied to file-read operations during Polish phase (PR-E) | Reduces unbounded memory consumption from hostile files; defaults are conservative |
| **Testing** | Integration tests for over-limit rejection per call site | E2e tests verify correct error on oversized files |

## Data Protection

### Sensitive Data Handling

| Data Type | Protection | Storage | Phase 4 / Polish Changes |
|-----------|-----------|---------|---|
| Git credentials | Inherited from system Git config | Credential helper, not Tome | Unchanged |
| Model artefacts (embedder, reranker) | SHA-256 verification on download | `<home>/.tome/models/<name>/` | Layout unchanged; central registry enforced |
| Model artefacts (summariser) | SHA-256 verification on download + on-load (US4.d-1 C-B1 real hash) | `<home>/.tome/models/qwen2.5-0.5b-instruct/` | New Phase 4 US4; SHA-256 `74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db` verified 2026-05-26; size 491,400,032 bytes pinned |
| Configuration file | Atomic writes, chmod inherited from original on rewrite via mode-preservation | `<home>/.tome/` | Single-root layout |
| Workspace settings | Layered composition with override semantics; atomic writes with mode preservation | `<workspace>/.tome/settings.toml` | New Phase 4 settings model; mode preserved on regen-summary |
| Project marker config | Atomic writes, chmod inherited from existing; workspace binding pointer at `<project>/.tome/config.toml` | `<project>/.tome/config.toml` | Consolidated to one type `ProjectMarkerConfig` via `settings::parser::read_project_marker` (PR-C fix) |
| Central index DB | SQLite with `PRAGMA foreign_keys = ON` and `journal_mode = WAL` | `<home>/.tome/index.db` | Centralized DB (Phase 3 per-workspace → Phase 4 single central) |
| Catalog cache | Atomic refresh, ref-counted across workspaces/projects via DB table, re-used on same URL | `<home>/.tome/cache/<sha256-of-url>/` | Layout unchanged; ref-counting via `workspace_catalogs` table |
| Git stderr output | Scrubbed before tracing/display | `src/catalog/git.rs::scrub_credentials` | Unchanged |
| HTTP error output | Scrubbed before surfacing | `src/embedding/download.rs::scrub_for_diag` | Unchanged |
| MCP server logs | JSON-lines to file (0600 chmod), error-only stderr | `<home>/.tome/mcp.log` | New layout under single root |
| Workspace paths in logs | Scrubbed via `scrub_to_string` before emission | `src/mcp/tools/search_skills.rs` | Unchanged |
| Error messages in logs | Scrubbed via `scrub_to_string` before emission | `src/mcp/tools/search_skills.rs` | Unchanged |
| Workspace summaries | Cached short + long text; short embedded in MCP tool description broadcast to clients | `<workspace>/.tome/settings.toml` (under `[summaries]`) | New Phase 4 US4; length-capped (SHORT: 800 chars, LONG: 2500 chars per FR-425); warn-level logging on exceedance; values still cached |
| Doctor harness list (Phase 4 / US5) | Local-only report; six well-known harness directories probed for existence; hyphenated names in output (PR-B fix) | Never transmitted; documented boundary | New diagnostic surface; privacy gate before any transmission feature |
| Orphan staging directories (Phase 4 / US5) | Cleaned by mtime-based filter (1h) after removal of parent workspace; symlink-skipped during cleanup | `<home>/.tome/` | New cleanup phase; STAGING_PREFIX gate + mtime guard + symlink-skipping compose correctly |
| Project binding marker state (Phase 4 / US1-US5) | Workspace name stored in `<project>/.tome/config.toml` under `[binding]` section | Project-local config; preserved on marker renames (US2.d-1 toml_edit fix) | New workspace binding; mode preserved on all atomic rewrites |

### Model Registry & Integrity (Phase 4 / US4-US5)

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

| File/Directory | Mode | Condition | Phase 4 / Polish Changes |
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
| `<home>/.tome/.tome.tmp.*` (staging dirs, US5) | 0700 (created by `atomic_dir`) | Temporary staging dirs; cleaned by orphan-cleanup mtime filter (Polish: five-layer defence-in-depth) | New Phase 4 US5; STAGING_PREFIX + 1h mtime + symlink-skip + is_dir() + 0o700 perms (PR-E extended) |

**Unix-only hardening**: The 0600 chmod is applied explicitly to `mcp.log` via `std::os::unix::fs::OpenOptionsExt::mode()` on Unix; Windows' ACL model is not currently addressed.

### Symlink Handling

| Context | Control | Implementation | Location |
|---------|---------|----------------|----------|
| **Skill directory walk** | Skip symlinks (explicit rejection) | `entry.file_type()` + `is_symlink()` skip | `src/mcp/tools/get_skill.rs::walk_dir` (lines 272–289, FR-S-02) |
| **RULES.md write** | Refuse symlinks on write-back | `is_symlink()` check → exit 7 (FR-M-HRN-2) | `src/harness/rules_file.rs` line 79 |
| **MCP config write** | Refuse symlinks on write-back | `is_symlink()` check → exit 7 | `src/harness/mcp_config.rs` line 92 |
| **Settings file write** (Phase 4 / US3) | Refuse symlinks on write-back via `save_settings` | `is_symlink()` check in `catalog::store::write_atomic` → exit 7 | `src/settings/edit.rs::save_settings` → `src/catalog/store.rs::write_atomic` (lines 88–95) |
| **Atomic file writes** (US2.d-1 / US4) | Refuse symlinks in `catalog::store::write_atomic` | `is_symlink()` check on target before staging write → exit 7 | `src/catalog/store.rs::write_atomic` |
| **Orphan cleanup walk (US5 + Polish)** | Skip symlinks; never traverse via symlink; rejects symlink-to-dir (layer 3 + 4) | `entry.metadata()` (lstat, no follow) + skip if `is_symlink()` + `is_dir()` check | `src/doctor/orphan_cleanup.rs::cleanup_staging_dirs` (PR-E extended) |
| **Purpose** | Prevent hostile catalog with `skills/foo/creds → ~/.ssh/id_rsa` or harness config pointing to sensitive files | Defence in depth: `lstat` (no follow) + explicit skip/refusal | Phase 3 PR #56; Phase 4 US1/US2/US4 extends to all atomic writes; US5 extends to orphan cleanup; Polish (PR-E) adds 5-layer cleanup defence |

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
| **Doctor repairs (US5)** | Five repair classes: embedder download, reranker download, catalog re-clone, binding rules-copy, summariser redownload. Read-only diagnostic checks; repairs run under advisory lock where state-mutating | `src/doctor/fixes.rs` + `src/doctor/checks.rs` |
| **Orphan cleanup (US5 + Polish)** | Staging-dir cleanup via STAGING_PREFIX match + 1h mtime gate + `is_dir()` check + symlink-skip + top-level parent cleanup (five layers, PR-E extended) | `src/doctor/orphan_cleanup.rs::cleanup_staging_dirs` |

## Phase 4 / US5 Security Hardening (PR #99–#101)

### Doctor Command Post-Review Fixes

| Control | Implementation | Fix | Status |
|---------|----------------|-----|--------|
| **Index health graceful collapse (C-M2)** | `check_index` returning `SchemaTooNew` previously short-circuited `assemble_report` with error. Now collapses gracefully via `unwrap_or_else` → `IndexHealth::Broken` | Doctor never crashes per FR-561 | ✅ PR #101 (C-M2) |
| **Project-local sync semantics (C-M3 + R-M2)** | `repair_binding_rules_copy` previously iterated EVERY bound project of workspace; now calls `sync_one_project` (new) targeting ONE project. Harness sync deduped: collect all suggested fixes, run sync ONCE per project, clear fixes list | Eliminates workspace-broadcast when project-local fix intended; reduces redundant sync passes from 10 to 1 | ✅ PR #101 (C-M3 + R-M2) |
| **SourceMissing vs. CopyMissing distinction (R-M5)** | `binding::compare_rules` now returns typed enum distinguishing "workspace source RULES.md absent" from "project copy absent". Absence of workspace source prevents infinite-loop attempt on `--fix` | Prevents `--fix` attempting to copy when source is absent; forwards descriptive message | ✅ PR #101 (R-M5) |
| **Read-only index access for diagnostics (R-M7)** | `repair_catalog` enrolment lookup uses `open_read_only` instead of writable `open`; only uses writable handle when actually deleting catalog cache rows | Diagnostic checks stay read-only per design; write operations explicit | ✅ PR #101 (R-M7) |
| **User-owned MCP override filtering (S-M2)** | Doctor `--fix --force` now filters user-owned harness MCP rewrites to ONLY those with active SuggestedFix entries in this run (filters by harness name). No blanket rewrite of every user-owned entry | Precisely scopes destructive override to triggered fixes; doesn't rewrite unrelated entries | ✅ PR #101 (S-M2) |
| **Cache-path invariant assertion (S-M4)** | `repair_catalog::remove_dir_all` now includes `debug_assert!(cache_dir.starts_with(&paths.catalogs_dir))` documenting safety invariant | Defence in depth; catches future invariant violations in debug builds | ✅ PR #101 (S-M4) |
| **Summariser repair test coverage (T-B1)** | Added `tests/doctor_fix_p4.rs::summariser_fix_redownloads` via SummariserOverrideGuard or pre-created placeholder model file; asserts `--fix` invokes the download branch | Covers fifth repair class (embedder/reranker/catalog/binding/summariser) in doctor flow | ✅ PR #101 (T-B1) |
| **`--force` without `--fix` rejection (R-M1)** | `commands/doctor.rs` now validates `args.force && !args.fix` upfront → `TomeError::Usage("--force requires --fix")` (exit 2) | Clear UX; exit 2 signals usage error, not system Io error | ✅ PR #101 (R-M1) |
| **NotApplicable health state (C-M1)** | `assemble_report` emits per-harness `SubsystemHealth::NotApplicable` entries when workspace is global-fallback (no project context). Wire distinguishes "no harnesses declared" (empty Vec) from "harnesses declared but outside project" (NotApplicable) | Diagnostic clarity; report shape stabilizes for wire consumers | ✅ PR #101 (C-M1) |

### Orphan Staging-Directory Cleanup (Phase 4 / US5, New Attack Surface)

| Control | Implementation | Status |
|---------|----------------|--------|
| **STAGING_PREFIX match** | Only `.tome.tmp.*` directories are cleanup candidates; matches atomic_dir prefix from Phase 4 | Prevents accidental removal of legitimate user directories |
| **Mtime-based gate** | Files/dirs older than 1h are candidates; prevents removing in-flight staging directories from concurrent writers | TOCTOU-safe: concurrent writers keep their staging current; crashed writers' orphans age out | ✅ Audited |
| **Symlink-skipping walk** | `entry.metadata()` (lstat, no follow) + skip if `is_symlink()` before any recursion | Hostile staging dir with `data → ~/.ssh/` won't be traversed | ✅ Audited |
| **`is_dir()` check before recursion** | Rejects `is_symlink()` AND files, only recurses actual directories | Symlink-to-dir attack rejected | ✅ Audited |
| **Staging-dir permissions (0700)** | Created by `atomic_dir` with explicit `0o700` mode on Unix | Prevents other users from reading staged content mid-operation | ✅ Audited |
| **Test coverage** | `STAGING_AGE_GATE` boundary tested; orphan cleanup exercised with real `.tome.tmp.*` dirs in tempdir | US5 test surface includes cleanup scenarios | ✅ PR #101 |

## Phase 4 / Polish Security Additions (PR-A through PR-G)

### Bounded String Reads (PR-E)

| Enhancement | Implementation | Scope |
|-------------|-----------------|-------|
| **Per-class limits** | `util::bounded_read_to_string(path, limit)` introduced | Applied to ~26 file-read call sites |
| **Index DB reads** | 10 KiB cap on XML/schema reads | `src/index/db.rs` + related |
| **Project marker reads** | 16 KiB cap on `ProjectMarkerConfig` deserialization | `src/settings/parser.rs::read_project_marker` |
| **Settings file reads** | 256 KiB cap on layered settings + composition | `src/settings/mod.rs` + `src/settings/edit.rs` |
| **Catalog manifest reads** | Conservative limits per caller context | Catalog clones remain unbounded (for now) |

### Home Path Validation (PR-E)

| Requirement | Implementation | Exit Code |
|-------------|-----------------|-----------|
| **Absolute path** | `path.is_absolute()` gate | 2 (Usage) |
| **Canonical path** | `canonicalize()` succeeds without symlink escape | 2 (Usage) |
| **Directory exists** | `is_dir()` check on result | 2 (Usage) |
| **Applied location** | `src/util/io.rs::home_root()` (new canonical validator) | Harness sync + doctor + project binding |
| **Relative/unset handling** | Relative or env-unset `$HOME` → exit 2 Usage | Clear error for misconfiguration |

### Consolidated Project Marker Type (PR-C)

| Change | Before | After | Impact |
|--------|--------|-------|--------|
| **Marker deserialization** | Multiple `deserialize` callers ad-hoc | Single `settings::parser::read_project_marker` | Consistency; enables bounded reads |
| **Type definition** | `ProjectMarkerConfig` struct in `workspace::binding.rs` | Unified with bounded-read integration | One canonical path for project config |
| **Error handling** | Per-caller error conversion | Centralized in `read_project_marker` | Consistent error reporting |

### Doctor Harness List Naming (PR-B)

| Update | Before | After | Context |
|--------|--------|-------|---------|
| **Harness name output** | Underscored (e.g., `claude_code`) | Hyphenated (e.g., `claude-code`) | Matches CLI harness identity grammar |
| **Database schema** | No change | Still stores underscored names | Wire format only; internal PK unchanged |
| **Test coverage** | Implicit | Explicit assertion on wire shape | `tests/doctor_subsystem_serialize.rs` |

## Signal Handling & Interruption

| Control | Implementation | Location |
|---------|----------------|----------|
| **SIGINT handler** | Global `AtomicBool` flipped by `ctrlc` callback | `src/catalog/git.rs` (lines 25–29) |
| **In-flight cleanup** | Child processes killed on interrupt; `TomeError::Interrupted` returned | `src/catalog/git.rs` (FR-026a) |
| **MCP graceful shutdown** | SIGINT triggers cancellation token; 5-second timeout for in-flight handlers | `src/mcp/mod.rs` (lines 43–47, contracts/mcp-server.md) |
| **Doctor fix atomicity** | Per-fix atomicity under advisory lock; SIGINT mid-fix leaves partially-applied state; doctor classification recomputes on next run | `src/doctor/fixes.rs`; no per-fix rollback (state persists for manual recovery) |
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
| `encoding_rs` | 0.8.x (direct, US4 new) | Decode llama-cpp-2 token output to UTF-8 | MPL 2.0; no known security issues; used for model output decoding only |

## Security Testing

| Category | Coverage | Location |
|----------|----------|----------|
| **Path validation** | 11 negative-case tests (URLs, absolute paths, traversal, symlinks) | `tests/path_validation.rs` |
| **Workspace name validation** | Phase 4 grammar tests | Phase 4 US1/US2 tests |
| **Project path validation** | UTF-8 enforcement, canonical path PK, dangerous-CWD refusal | `tests/workspace_use_binding.rs` |
| **Credential scrubbing** | 4 pattern rules + integration tests against real Git output; Polish: summariser URL + harness MCP scrubbing | `tests/scrubbing.rs` (extended PR-D) |
| **Manifest strictness** | 100% grep assertion on `deny_unknown_fields`; Phase 4 US4 audit (T098n) includes summariser types | `tests/manifest_strictness.rs` |
| **Summariser model integrity** | Placeholder regression guard (3 tests); real-model SHA-256 verify + load; length-window warn; silent no-op carve-out | `tests/summariser_registry_no_placeholder.rs`, `tests/summariser_real.rs`, `tests/summariser_triggers.rs`, `tests/summariser_triggers_end_to_end.rs`, `tests/workspace_regen_summary.rs` |
| **Concurrency & atomicity** | Advisory lock + interrupt scenarios; cache cleanup under lock (F11b); binding + last_used_at atomic (R-M1) | `tests/atomicity.rs`, `tests/concurrency.rs` |
| **Exit codes** | Closed enumeration; all Phase 1/2/3/4/US5 codes tested | `tests/exit_codes.rs` |
| **Security hardening** | File permissions, symlink handling, registry validation, mode preservation on rewrite, symlink refusal on atomic writes, settings-edit security, bounded reads (Polish: PR-E), home path validation (Polish: PR-E) | `tests/security_hardening.rs` (extended) |
| **MCP protocol purity** | No error leakage to stdout (FR-108) | `tests/mcp_server.rs` |
| **Workspace isolation** | Cross-workspace catalog enablement + reference-counting (Phase 3); project binding validation (Phase 4); settings composition (US3); summariser per-workspace (US4) | `tests/workspace_commands.rs`, `tests/catalog_cache_refcount.rs`, Phase 4 tests |
| **Sync idempotence** | Mtime stability across re-sync with all harness modules | `tests/sync_idempotence.rs` |
| **Harness concurrency** | Parallel HOME mutation via `HOME_MUTEX` (US3 PR #92); consolidated via `HomeGuard` RAII (Polish: PR-E) | `tests/harness_*.rs` + `tests/common/mod.rs::HomeGuard` |
| **Doctor repairs (US5)** | Five repair classes (embedder/reranker/catalog/binding/summariser) via library API + CLI binary + e2e scenarios | `tests/doctor_fix_p4.rs`, `tests/exit_codes_e2e.rs` |
| **Orphan cleanup (US5)** | Mtime filtering, STAGING_PREFIX matching, symlink-skip, dir-check, cleanup of empty parents (Polish: extended to five layers) | `tests/doctor_orphan_tmp_cleanup.rs` (Polish: PR-E extended) |
| **MCP input validation (US5)** | Query length cap 4096 chars enforced at input boundary | `tests/mcp_input_length_caps.rs` |
| **Bounded reads (Polish)** | Per-class limits verified for project markers, settings files, index reads | Integration tests for oversized file rejection (PR-E) |

---

*This document defines security controls. Update when security posture changes.*
