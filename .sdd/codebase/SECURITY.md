# Security

> **Purpose**: Document authentication, authorization, security controls, and vulnerability status.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-14 (Phase 3 complete: US1–US5 shipped; PR #56 security hardening applied)

## Overview

Tome is a Rust CLI (and MCP server) for managing plugin catalogs and embeddings. As a synchronous, file-based tool without user authentication, security focuses on:
1. Preventing path traversal and directory-escape attacks via plugin source paths and plugin identities
2. Integrity verification for downloaded model artefacts (SHA-256 checksums)
3. Scrubbing credentials from captured Git output and HTTP errors at the boundary
4. Atomic writes to prevent partial state corruption
5. Signal handling for clean interruption
6. TTY enforcement on interactive flows to prevent prompt injection and non-interactive misuse
7. Dependency-allowlist enforcement and weekly vulnerability scanning
8. Binary-size constraints to limit attack surface
9. MCP server protocol purity (stdout reserved for MCP protocol, errors to stderr)
10. Structured logging with size-based rotation for long-running MCP server
11. MCP startup pre-flight validation with SHA-256 verification and drift detection
12. No domain-error leakage in MCP tool responses (structured codes only)
13. Workspace initialization with secure directory permissions and atomic staging
14. Catalog cache content-trust via ref-counting and re-use on same URL
15. Doctor command harness detection without config parsing; network access gated behind `--fix`
16. Forward-only schema migrations with per-migration transaction atomicity
17. Symlink-aware skill-directory walks with explicit rejection (FR-S-02)
18. Workspace registry validation with size cap, entry limit, NUL rejection, and parent-dir rejection (FR-S-03)
19. Workspace init refusal of non-directory `.tome` markers (FR-S-04)
20. Credential scrubbing on MCP log fields and error chains

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
| **MCP log scrubbing** | Workspace paths and error messages scrubbed before JSON logging | `src/mcp/mod.rs::workspace_path` (line 100), tool handlers (line 272) |
| **Model URL scrubbing** | Download URLs with presigned params scrubbed in error chains | `src/embedding/download.rs` (lines 71–73 comment) |
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

### Manifest Strictness

| Rule | Implementation | Enforcement |
|------|----------------|-------------|
| **Unknown fields banned** | `#[serde(deny_unknown_fields)]` on all Tome-owned Deserialize structs | `src/catalog/manifest.rs`, `src/config.rs`, `src/embedding/registry.rs::ModelManifest` |
| **Compile-time check** | Every Tome-owned Deserialize struct preceded by attribute | Verified by structural grep test |
| **Test enforcement** | `tests/manifest_strictness.rs` — assertion on 100% coverage | Test fails if any struct lacks attribute |
| **Lenient third-party inputs** | `plugin.json` and `SKILL.md` frontmatter parsed without `deny_unknown_fields` (FR-013a) | Forward-compatible with upstream schema additions |
| **Coverage** | Strict targets: `CatalogManifest`, `Owner`, `PluginDeclaration`, `Config`, `CatalogEntry`, `ModelManifest`, `ModelKind` | Mandatory, no exceptions |

### Workspace Registry Validation (Phase 3 US2)

| Control | Implementation | Enforcement |
|---------|----------------|-------------|
| **Size cap** | 1 MiB maximum file size | `src/workspace/inventory.rs::MAX_REGISTRY_BYTES` (line 39) |
| **Entry cap** | 10,000 maximum entries | `src/workspace/inventory.rs::MAX_REGISTRY_ENTRIES` (line 40) |
| **NUL rejection** | Lines containing NUL byte silently dropped | `src/workspace/inventory.rs::read_registry` (line 75) |
| **Parent dir rejection** | Lines containing `..` components silently dropped | `src/workspace/inventory.rs::read_registry` (lines 87–89) |
| **Absolute path required** | Non-absolute paths silently dropped | `src/workspace/inventory.rs::read_registry` (line 79) |
| **Deduplication** | By canonicalize equality, not string match | `src/workspace/inventory.rs::append_if_registry_exists` (lines 120–128) |

**Purpose**: Prevent hostile or malformed workspace registry entries from escaping their scope or DoS-ing the reader.

### Workspace Init Marker Validation (Phase 3 US2)

| Control | Implementation | Location |
|---------|----------------|----------|
| **Directory-only check** | Reject non-directory `.tome` marker (regular file, symlink, FIFO, etc.) | `src/workspace/init.rs` (lines 78–89) |
| **Specific error** | Surface exact file type (file/symlink/non-directory) | `src/workspace/init.rs::describe_file_type` (FR-S-04) |
| **Purpose** | Prevent `--force` from leaving user-created files behind | `src/workspace/init.rs` (lines 71–95 comments) |

## Data Protection

### Sensitive Data Handling

| Data Type | Protection | Storage |
|-----------|-----------|---------|
| Git credentials | Inherited from system Git config | Credential helper, not Tome |
| Model artefacts | SHA-256 verification on download | `~/.local/share/tome/models/<name>/` |
| Configuration file | Atomic writes, chmod 0600 on Unix | `~/.config/tome/config.toml` (global) and `<workspace>/.tome/config.toml` (workspace) |
| Catalog cache | Atomic refresh, ref-counted across scopes, re-used on same URL | `~/.cache/tome/<sha256-of-url>/` |
| Git stderr output | Scrubbed before tracing/display | `src/catalog/git.rs::scrub_credentials` |
| HTTP error output | Scrubbed before surfacing | `src/embedding/download.rs::scrub_for_diag` |
| MCP server logs | JSON-lines to file (0600 chmod), error-only stderr | `src/mcp/log.rs` (10 MiB cap, rotates to `.1`) |
| Workspace paths in logs | Scrubbed via `scrub_to_string` before emission | `src/mcp/mod.rs` (line 100) |
| Error messages in logs | Scrubbed via `scrub_to_string` before emission | `src/mcp/tools/search_skills.rs` (line 272) |
| Workspace registry | Opt-in, deduplicated list, validated at read | `${XDG_STATE_HOME}/tome/workspaces.txt` (not created unless user explicitly uses it) |
| Harness detection list | Local-only, never transmitted; indexed by directory existence | `src/doctor/harness_detect.rs::probe` |

### File Permissions

| File Type | Unix Permissions | Windows | Location |
|-----------|-----------------|---------|----------|
| `config.toml` (global) | 0600 | N/A | `src/catalog/store.rs::write_atomic` (lines 100–105) |
| `config.toml` (workspace) | 0600 | N/A | `src/catalog/store.rs::write_atomic` (lines 100–105) |
| Workspace `.tome/` directory | 0700 (before content) | N/A | `src/workspace/init.rs` (lines 107–112) |
| MCP log file | 0600 (creation + tighten if exists) | N/A | `src/mcp/log.rs` (lines 74–91) |

### Symlink Handling

| Context | Control | Implementation | Location |
|---------|---------|----------------|----------|
| **Skill directory walk** | Skip symlinks (explicit rejection) | `entry.file_type()` + `is_symlink()` skip | `src/mcp/tools/get_skill.rs::walk_dir` (lines 272–289, FR-S-02) |
| **Purpose** | Prevent hostile catalog with `skills/foo/creds → ~/.ssh/id_rsa` | Defence in depth: `lstat` (no follow) + explicit skip | `src/mcp/tools/get_skill.rs` (lines 261–267 comment) |

### Integrity & Verification

| Component | Mechanism | Enforcement |
|-----------|-----------|-------------|
| **Model downloads** | SHA-256 checksum + size_bytes pin | `src/embedding/download.rs::download_model` (exit 32 on mismatch) |
| **Registry pinning** | Compile-time constant `MODEL_REGISTRY` | `src/embedding/registry.rs::MODEL_REGISTRY` (verified real at Phase 3 slice 1) |
| **Placeholder detection** | `has_placeholder_checksum()` guard | `src/embedding/download.rs::download_model` (exit 31 if placeholder) |
| **Atomic model persist** | `.partial/` → final rename | `src/embedding/download.rs::download_model`, step 4 |
| **Re-verification** | New `embedding::download::sha256_file()` helper | `src/embedding/download.rs::sha256_file`, invoked by `tome models list --verify` |
| **Virtual table constraints** | `sqlite-vec` does not support `INSERT OR REPLACE`; uses `DELETE`-then-`INSERT` | `src/index/skills.rs::upsert_skill` |
| **Health check** | `tome status [--verify]` re-verifies installed models without re-downloading | `src/commands/status.rs::check_model()` |
| **MCP startup pre-flight** | SHA-256 verification of primary embedder file at every startup (FR-110) | `src/mcp/preflight.rs::verify_embedder_artefacts` |
| **Workspace initialization** | Atomic staging with permissions lock before content lands | `src/workspace/init.rs` |
| **Catalog cache content trust** | Re-use existing clone on URL re-add; delete clone only when no scopes reference the URL | `src/catalog/store.rs::reference_count` |
| **Schema migrations** | Forward-only migrations with per-step transaction atomicity under advisory lock | `src/index/migrations.rs::apply_pending` |

**Model Registry** (Phase 3 update):
- `bge-small-en-v1.5` INT8: SHA-256 `51f1bd0addd6e859e42c2c8021a5e5461385bb676a649f4b269aa445449f2431`, 66.5 MB, MIT
- `bge-reranker-base` INT8: SHA-256 `46a1bb4cf46ff1e300d27589d620141fbf04fc0eaf8e7bb6dea5e044475ff387`, 279.3 MB, MIT (sourced from `onnx-community` mirror)

Both checksums are real upstream digests verified at Phase 3 slice 1. Downloads enforce both hash and size; drift surfaces as `ModelChecksumMismatch` (exit 32) rather than silently installing whatever upstream serves.

### Phase 3 Polish Security Hardening (PR #56)

Phase 3 Polish PR #56 applied systematic security hardening across MCP and workspace subsystems:

| Control | Implementation | Location |
|---------|----------------|----------|
| **MCP log chmod 0600** | File opened with explicit 0600 mode; existing files tightened on startup | `src/mcp/log.rs::open_appender` (lines 74–91) |
| **Skill symlink defence** | Directory walk explicitly skips symlinks (silent, no log) | `src/mcp/tools/get_skill.rs::walk_dir` (lines 277–281) |
| **Workspace registry validation** | Size cap (1 MiB), entry cap (10k), NUL rejection, `..` rejection | `src/workspace/inventory.rs` (lines 39–40, 75, 87–89) |
| **Workspace init marker check** | Refuse non-directory `.tome` entries with specific error | `src/workspace/init.rs` (lines 78–89) |
| **Workspace init error propagation** | `--force` pre-cleanup errors propagate (FR-M-WKS-2) | `src/workspace/init.rs` (lines 132–139) |
| **Registry deduplication** | By `canonicalize` equality, not raw string match | `src/workspace/inventory.rs` (lines 120–128) |

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
| **Structured logs** | `${XDG_STATE_HOME}/tome/mcp.log` | JSON-lines per contract | 10 MiB cap, rotate to `.1` |
| **File permissions** | 0600 on Unix | N/A on Windows | `src/mcp/log.rs::open_appender` |
| **Stderr** | Fatal errors only | Human-readable | Filtered to `error!` level (FR-222) |
| **Scrubbing** | All user-sensitive fields | Via `scrub_to_string` | `src/mcp/mod.rs`, tool handlers |

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
| **Binary size cap** | 50 MB hard cap; currently 22 MiB on macOS arm64 | `CONSTITUTION.md` (NFR-001, revised 2026-05-13) |
| **LTO + strip** | `lto = "thin"`, `strip = "symbols"`, `panic = "abort"` | `Cargo.toml` (profile settings) |

## Security Testing

| Category | Coverage | Location |
|----------|----------|----------|
| **Path validation** | 11 negative-case tests (URLs, absolute paths, traversal, symlinks) | `tests/path_validation.rs` |
| **Credential scrubbing** | 4 pattern rules + integration tests against real Git output | `tests/scrubbing.rs` |
| **Manifest strictness** | 100% grep assertion on `deny_unknown_fields` | `tests/manifest_strictness.rs` |
| **Concurrency & atomicity** | Advisory lock + interrupt scenarios | `tests/atomicity.rs`, `tests/concurrency.rs` |
| **Exit codes** | Closed enumeration; all Phase 1/2/3 codes tested | `tests/exit_codes.rs` |
| **Security hardening** | File permissions, symlink handling, registry validation | `tests/security_hardening.rs` |
| **MCP protocol purity** | No error leakage to stdout (FR-108) | `tests/mcp_server.rs` |
| **Workspace isolation** | Cross-scope catalog enablement + reference-counting | `tests/workspace_commands.rs`, `tests/catalog_cache_refcount.rs` |

---

*This document defines security controls. Update when security posture changes.*
