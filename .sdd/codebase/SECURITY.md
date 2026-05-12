# Security

> **Purpose**: Document authentication, authorization, security controls, and vulnerability status.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-11

## Overview

Tome is a Rust CLI for managing plugin catalogs. As a synchronous, file-based tool without user authentication, security focuses on:
1. Preventing path traversal and directory-escape attacks via plugin source paths
2. Scrubbing credentials from captured Git output at the boundary
3. Atomic writes to prevent partial state corruption
4. Signal handling for clean interruption
5. Dependency-allowlist enforcement and weekly vulnerability scanning
6. Binary-size constraints to limit attack surface

Security controls are enforced in code, tests, and CI—documented in `CONSTITUTION.md` and `specs/001-phase-1-foundations/spec.md`.

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
| **No credential storage** | Inherit user's Git config entirely | Constitution XII |
| **No credential prompting** | Only system Git handles auth | Constitution XII, FR-026 |

The credential scrubber applies four ordered regex patterns to every byte stream from `git`:
1. URL-embedded credentials: `https?://[^/@\s]+@` → `https://` (drops `user:token@`)
2. SSH login info: `git@[^\s:]+:` → `git@<host>:` (preserves host, scrubs login)
3. Key-value pairs: `(token|password|api[-_]?key|bearer|authorization)\s*[:=]\s*\S+` → `<scrubbed>`
4. Long hex (40+ chars outside safe context): `[0-9a-fA-F]{40,}\b` → `<scrubbed>` (except in `:` or `=` contexts where SHAs are preserved)

**Verification**: Comprehensive test coverage in `tests/scrubbing.rs` covers all four rules with worked examples.

## Input Validation

### Plugin Source Path Validation

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

**Test Coverage**: Every variant of `ManifestInvalid` has explicit test cases:
- `https://example.com/repo` → rejected
- `git@host:owner/repo` → rejected
- `/etc/passwd` → rejected
- `C:\plugins` → rejected (Windows drive)
- `../escape` → rejected (syntactic `..`)
- `./plugins/../escape` → rejected (embedded `..`)
- Symlinks outside catalog → rejected (semantic escape via canonicalize)

### Manifest Strictness

| Rule | Implementation | Enforcement |
|------|----------------|-------------|
| **Unknown fields banned** | `#[serde(deny_unknown_fields)]` on all Deserialize structs | `src/catalog/manifest.rs`, `src/config.rs` |
| **Compile-time check** | Every Deserialize struct preceded by attribute | Verified by structural grep test |
| **Test enforcement** | `tests/manifest_strictness.rs` — assertion on 100% coverage | Test fails if any struct lacks attribute |
| **Coverage** | All deserialization targets: `CatalogManifest`, `Owner`, `PluginDeclaration`, `Config`, `CatalogEntry` | Mandatory, no exceptions |

**Semantic validation** (manifest.rs):
- `name`, `description` must be non-empty (trimmed)
- `version` must parse as semver
- `owner.email` must contain exactly one `@`, non-empty local and domain, domain has `.`
- `plugins[].name` must be unique within catalog
- `plugins[].source` must pass 6-step path validation (above)

## Data Protection

### Sensitive Data Handling

| Data Type | Protection | Storage |
|-----------|-----------|---------|
| Git credentials | Inherited from system Git config | Credential helper, not Tome |
| Configuration file | Atomic writes, permissive POSIX defaults | `~/.config/tome/config.toml` |
| Catalog cache | Atomic refresh, tool-owned | `~/.cache/tome/<sha256-of-url>/` |
| Git stderr output | Scrubbed before tracing/display | `src/catalog/git.rs::scrub_credentials` |

### Encryption

| Type | Status |
|------|--------|
| At rest | Not implemented (cache is local, untrusted) |
| In transit | Inherited from system Git (TLS/SSH) |
| Application-level | No application-managed secrets |

## Error Reporting & Exit Codes

### Closed Error Set

| Exit Code | Category | Meaning | Specification |
|-----------|----------|---------|----------------|
| 0 | Success | Operation completed successfully | — |
| 1 | Internal | Programmer error, panic caught | FR-022 |
| 2 | Usage | Invalid command-line arguments | FR-022 |
| 3 | CatalogNotFound | Catalog not registered | FR-022 |
| 4 | CatalogAlreadyExists | Catalog already registered | FR-022 |
| 5 | ManifestInvalid | Manifest parse or validation failed | FR-022 |
| 6 | GitFailed | Git operation failed (clone, fetch, reset) | FR-022 |
| 7 | Io | Filesystem or I/O error | FR-022, PRD amendment §2.2 |
| 8 | Interrupted | User interrupted (Ctrl-C) | FR-026a |

**Enforcement**: Closed enum `TomeError` in `src/error.rs`. Adding a variant forces edits to:
1. `TomeError` enum
2. `exit_code()` match arm
3. `category()` match arm
4. `tests/exit_codes.rs` assertions

No generic "Other" or "Unknown" variant. Every path maps to a named category.

### Error Messaging

| Requirement | Implementation | Location |
|-------------|----------------|----------|
| **Name what failed** | Error variant includes resource name | `TomeError::CatalogNotFound(String)` |
| **Name where it failed** | File paths in `ManifestInvalid` variants | `TomeError::ManifestInvalid(ManifestInvalid)` |
| **Suggest next action** | Schema URI in unknown-field errors | `ManifestInvalid::UnknownField { expected_schema_uri }` |
| **Surface upstream errors** | Git stderr passed through scrubber | `TomeError::GitFailed { detail: String }` |

Example error (path traversal):
```
`plugins[].source = "../escape"` in /path/to/tome-catalog.toml contains `..` — must be a normalised relative path
```

## Signal Handling & Cancellation

### SIGINT Handling

| Component | Implementation | Location |
|-----------|----------------|----------|
| **Handler install** | One-time `ctrlc::set_handler` | `src/catalog/git.rs::install_signal_handler` |
| **Cancellation flag** | Global `AtomicBool` with `SeqCst` ordering | `src/catalog/git.rs::CANCELLED` |
| **Child cleanup** | `Child::kill()` on flag flip | `src/catalog/git.rs::Git::run` |
| **Cache integrity** | Atomic write via temp-dir RAII | `src/catalog/store.rs::write_atomic` |
| **Exit code** | 8 (documented, non-zero) | `TomeError::Interrupted` |

**Guarantees** (FR-026a, SC-011):
- Long-running Git operations (clone, fetch) are pollable for cancellation
- On cancellation, child process is killed and control returns to main
- No orphaned child processes
- Per-catalog cache atomicity is preserved (temp dir dropped via RAII)

**Test**: `tests/atomicity.rs` and signal-handling verification in integration tests.

## Atomic Writes

### Registry & Cache Persistence

| Operation | Atomicity Guarantee | Implementation |
|-----------|-------------------|-----------------|
| **Registry write** | Atomic replace via temp + rename | `src/catalog/store.rs::write_atomic` |
| **Cache refresh** | Atomic temp dir swap per catalog | `src/catalog/store.rs::clone_and_validate` |
| **Temp file cleanup** | RAII via `tempfile::NamedTempFile` + `TempDir` | Rust Drop trait |

**Mechanism**:
1. Write to a temporary file in the same directory as the target
2. Rename (POSIX `rename(2)` is atomic on same filesystem)
3. On error, temp file is cleaned up; target is unchanged

**Guarantees** (FR-017a, FR-017b, SC-012):
- A partial or interrupted write leaves the on-disk file in either pre-state or post-state, never partial
- Multiple concurrent invocations see either the old version or the new version, never a mixture
- Test coverage: `tests/atomicity.rs` with concurrent writes and simulated interruption

## Dependency Management

### Licence Allowlist

| Category | Allowed Licences |
|----------|------------------|
| Permissive | MIT, Apache-2.0, MIT-0, BSD-2-Clause, BSD-3-Clause, ISC, Unicode-DFS-2016, Zlib |
| Explicitly banned | GPL, AGPL, LGPL (all versions) |
| Configuration | `deny.toml` — enforced by `cargo-deny` in CI |

**Enforcement**:
- `deny.toml` in repository root with `licenses.allow` list
- `cargo-deny check` runs on every PR and weekly
- GitHub Actions workflow: `.github/workflows/security.yml`
- Confidence threshold: 0.93 (handles ambiguous license text)

### Vulnerability Scanning

| Tool | Frequency | Configuration | Enforcement |
|------|-----------|---|--------------|
| `cargo-audit` | Weekly + on every PR | Installed and run via CI | Fails workflow on vulnerability |
| `cargo-deny` | Weekly + on every PR | `deny.toml` (see above) | Fails workflow on disallowed licence |
| MSRV verification | On every PR | `rust-version` in Cargo.toml | Tested on pinned MSRV |
| Binary size check | On release builds | 10 MB stripped limit | Fails if exceeded |

**Workflow**: `.github/workflows/security.yml`
- Runs on: PR, push to main, weekly schedule (Mondays 04:17 UTC)
- Parallel jobs: `cargo-audit` and `cargo-deny`

### Dependency Updates

| Policy | Implementation |
|--------|-----------------|
| **Minimal set** | Only dependencies solving concrete Phase-N problems |
| **Justified additions** | New deps require written justification in PR |
| **Renovate** | Automated proposal on updates; human review required before merge |
| **MSRV compatibility** | Dependency MSRV constraints propagate to project MSRV |

**Current dependencies** (Phase 1):
- `clap` 4 (CLI parsing) — already used for colour
- `serde`, `serde_json` (serialization)
- `toml` (manifest format)
- `anyhow`, `thiserror` (error handling)
- `tracing`, `tracing-subscriber` (logging)
- `sha2`, `hex` (cache naming)
- `tempfile` (atomic writes, test fixtures)
- `ctrlc` (signal handling)
- `regex` (credential scrubbing)
- `semver` (version validation)
- `time` (timestamp serialization)
- `directories` (XDG-aware paths)

All fall within permissive licences.

## Binary Size & Deployment

### Size Constraint

| Metric | Limit | Current | Status |
|--------|-------|---------|--------|
| Stripped release binary | 10 MB | ~2.7 MB | ✓ Well within budget |
| Binary size growth | Must justify | N/A | Checked on every release build (CI) |

**Enforcement**: CI job `Release binary size check` in `.github/workflows/ci.yml`:
```bash
cargo build --release
size=$(stat -c%s target/release/tome)
if [ "$size" -ge 10485760 ]; then
  echo "::error::Release binary exceeds the 10 MB limit (SC-010)"
  exit 1
fi
```

New dependencies that grow the binary significantly require written justification and may require architectural reconsideration.

## Output Security

### JSON Output Scrubbing

| Mode | Output Stream | Scrubbing |
|------|---|---|
| Human-readable | Stdout | Via `anstream` / `anstyle` (colour only) |
| Structured (`--json`) | Stdout | All error `detail` strings passed through `scrub_credentials` |
| Diagnostic logs | Stderr (always) | All tracing records passed through `scrub_credentials` |

**Implementation**: Scrubbing happens at the capture point (Git stderr → `scrub_to_string`), ensuring no downstream path leaks credentials.

### Colour & Accessibility

| Feature | Implementation |
|---------|-----------------|
| **NO_COLOR support** | Honoured by `anstream` wrapper |
| **CLICOLOR=0** | Honoured by `anstream` wrapper |
| **TTY detection** | `std::io::IsTerminal` (stable in Rust 1.70+) |
| **Auto-disable** | Colour disabled when stdout is not a terminal |

## Concurrency Model

### Locking & Advisory Locking

| Resource | Locking Strategy |
|----------|-----------------|
| Registry file | Atomic rename (not mutex) — no advisory lock |
| Catalog cache | Per-catalog atomic swap — no cross-catalog lock |
| Global state | None (sync-only, CLI is per-invocation) |

**Rationale**: Phase 1 is synchronous and single-process. Concurrent invocations are safe because:
- Registry writes are atomic (rename)
- Cache writes are atomic per catalog
- Concurrent readers see either the old state or the new state

**Future consideration**: Phase 2 MCP server will need to introduce mutex-based locking or advisory file locks if concurrent harness access becomes possible.

## Security Testing

### Test Categories

| Category | Coverage | Files |
|----------|----------|-------|
| **Path validation** | Exhaustive negative corpus (URLs, absolutes, `..`, escapes, symlinks) | `tests/path_validation.rs` (11 cases) |
| **Scrubbing** | All four regex rules + ordering + edge cases | `tests/scrubbing.rs` (8 cases) |
| **Strictness** | Every `Deserialize` struct has `deny_unknown_fields` | `tests/manifest_strictness.rs` (2 assertions) |
| **Atomicity** | Concurrent writes, partial writes, interruption | `tests/atomicity.rs` (4 cases) |
| **Exit codes** | Every `TomeError` variant maps to documented code | `tests/exit_codes.rs` |
| **Integration** | Real Git repos, real fixtures, real filesystems | `tests/catalog_*.rs` (5 files) |

**Success criteria**:
- SC-005: 100% of malformed inputs rejected with helpful errors
- SC-006: No credential material observable in any output
- SC-011: Interruption leaves no orphaned processes
- SC-012: Mid-write interruption leaves recoverable state

## Known Gaps & Future Work

| Concern | Phase | Note |
|---------|-------|------|
| Advisory locking for concurrent access | Phase 2 | MCP server needs robust concurrency model |
| Encryption at rest for sensitive caches | Phase 3+ | Deferred until use case demands it |
| Audit logging | Phase 3+ | Not required in Phase 1 (single-user CLI) |
| Rate limiting | Not applicable | CLI tool, not a service |

---

*This document defines security controls. Update when security posture changes.*
