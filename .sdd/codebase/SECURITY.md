# Security

> **Purpose**: Document authentication, authorization, security controls, and vulnerability status.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-13 (Phase 9 incremental)

## Overview

Tome is a Rust CLI for managing plugin catalogs and embeddings. As a synchronous, file-based tool without user authentication, security focuses on:
1. Preventing path traversal and directory-escape attacks via plugin source paths and plugin identities
2. Integrity verification for downloaded model artefacts (SHA-256 checksums)
3. Scrubbing credentials from captured Git output and HTTP errors at the boundary
4. Atomic writes to prevent partial state corruption
5. Signal handling for clean interruption
6. TTY enforcement on interactive flows to prevent prompt injection and non-interactive misuse
7. Dependency-allowlist enforcement and weekly vulnerability scanning
8. Binary-size constraints to limit attack surface

Security controls are enforced in code, tests, and CI—documented in `CONSTITUTION.md` and `specs/001-phase-1-foundations/spec.md` (Phase 1), `specs/002-phase-2-plugins-index/spec.md` (Phase 2), and `specs/002-phase-2-plugins-index/contracts/plugin-commands.md` (Phase 4–5).

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
| **No credential storage** | Inherit user's Git config entirely | Constitution XII |
| **No credential prompting** | Only system Git handles auth | Constitution XII, FR-026 |

The credential scrubber applies four ordered regex patterns to every byte stream from `git` and HTTP operations:
1. URL-embedded credentials: `https?://[^/@\s]+@` → `https://` (drops `user:token@`)
2. SSH login info: `git@[^\s:]+:` → `git@<host>:` (preserves host, scrubs login)
3. Key-value pairs: `(token|password|api[-_]?key|bearer|authorization)\s*[:=]\s*\S+` → `<scrubbed>`
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

**Test Coverage**: Every variant of `ManifestInvalid` has explicit test cases:
- `https://example.com/repo` → rejected
- `git@host:owner/repo` → rejected
- `/etc/passwd` → rejected
- `C:\plugins` → rejected (Windows drive)
- `../escape` → rejected (syntactic `..`)
- `./plugins/../escape` → rejected (embedded `..`)
- Symlinks outside catalog → rejected (semantic escape via canonicalize)

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

**Semantic validation** (manifest.rs, registry.rs):
- `name`, `description` must be non-empty (trimmed)
- `version` must parse as semver
- `owner.email` must contain exactly one `@`, non-empty local and domain, domain has `.`
- `plugins[].name` must be unique within catalog
- `plugins[].source` must pass 6-step path validation (above)
- Model `sha256` must not be placeholder (all-zero string)
- Model `size_bytes` must match pinned registry value on download completion

## Data Protection

### Sensitive Data Handling

| Data Type | Protection | Storage |
|-----------|-----------|---------|
| Git credentials | Inherited from system Git config | Credential helper, not Tome |
| Model artefacts | SHA-256 verification on download | `~/.local/share/tome/models/<name>/` |
| Configuration file | Atomic writes, permissive POSIX defaults | `~/.config/tome/config.toml` |
| Catalog cache | Atomic refresh, tool-owned | `~/.cache/tome/<sha256-of-url>/` |
| Git stderr output | Scrubbed before tracing/display | `src/catalog/git.rs::scrub_credentials` |
| HTTP error output | Scrubbed before surfacing | `src/embedding/download.rs::scrub_for_diag` |

### Integrity & Verification

| Component | Mechanism | Enforcement |
|-----------|-----------|-------------|
| **Model downloads** | SHA-256 checksum + size_bytes pin | `src/embedding/download.rs::download_model` (exit 32 on mismatch) |
| **Registry pinning** | Compile-time constant `MODEL_REGISTRY` | `src/embedding/registry.rs::MODEL_REGISTRY` (verified real at Phase 3 slice 1) |
| **Placeholder detection** | `has_placeholder_checksum()` guard | `src/embedding/download.rs::download_model` (exit 31 if placeholder) |
| **Atomic model persist** | `.partial/` → final rename | `src/embedding/download.rs::download_model`, step 4 |
| **Re-verification** | New `embedding::download::sha256_file()` helper | `src/embedding/download.rs::sha256_file`, invoked by `tome models list --verify` (Phase 6) |
| **Virtual table constraints** | `sqlite-vec` does not support `INSERT OR REPLACE`; uses `DELETE`-then-`INSERT` | `src/index/skills.rs::upsert_skill` (Phase 7, lines 282–294) |
| **Health check** | `tome status [--verify]` re-verifies installed models without re-downloading | `src/commands/status.rs::check_model()` (Phase 8, PR #29) |

**Model Registry** (Phase 3 update):
- `bge-small-en-v1.5` INT8: SHA-256 `51f1bd0addd6e859e42c2c8021a5e5461385bb676a649f4b269aa445449f2431`, 66.5 MB, MIT
- `bge-reranker-base` INT8: SHA-256 `46a1bb4cf46ff1e300d27589d620141fbf04fc0eaf8e7bb6dea5e044475ff387`, 279.3 MB, MIT (sourced from `onnx-community` mirror)

Both checksums are real upstream digests verified at Phase 3 slice 1. Downloads enforce both hash and size; drift surfaces as `ModelChecksumMismatch` (exit 32) rather than silently installing whatever upstream serves. The `--verify` flag in `tome models list` (Phase 6) allows users to audit installed models against pinned checksums without re-downloading.

**Phase 8 Status Command** (`src/commands/status.rs`, PR #29):
- `tome status` is a supported pre-flight health check before filing bug reports
- Read-only: never acquires the advisory lock (FR-056), works even when a writer is running
- Classifies health as Healthy, Degraded, or Unhealthy based on model state + index integrity + drift detection
- Embedder drift or index corruption → Unhealthy (exit 1); reranker drift only → Degraded (exit 1); all Ok → Healthy (exit 0)
- `--verify` flag rehashes models via `sha256_file()` against registry-pinned SHA-256s (no re-download)
- `--json` output includes model names/versions, index state, and drift diagnostics
- No secrets exposed; model identities are public constants from `MODEL_REGISTRY`

### Encryption

| Type | Status |
|------|--------|
| At rest | Not implemented (cache is local, untrusted) |
| In transit | Inherited from system Git (TLS/SSH) and `reqwest` (TLS 1.2+) |
| Application-level | No application-managed secrets |

## Error Reporting & Exit Codes

### Closed Error Set

| Exit Code | Category | Meaning | Phase | Specification |
|-----------|----------|---------|-------|----------------|
| 0 | Success | Operation completed successfully | — | — |
| 1 | Internal | Programmer error, panic caught | All | FR-022 |
| 2 | Usage | Invalid command-line arguments | All | FR-022 |
| 3 | CatalogNotFound | Catalog not registered | 1 | FR-022 |
| 4 | CatalogAlreadyExists | Catalog already registered | 1 | FR-022 |
| 5 | ManifestInvalid | Manifest parse or validation failed | 1 | FR-022 |
| 6 | GitFailed | Git operation failed (clone, fetch, reset) | 1 | FR-022 |
| 7 | Io | Filesystem or I/O error | All | FR-022 |
| 8 | Interrupted | User interrupted (Ctrl-C or Ctrl-D in prompt) | All | FR-026a |
| 20 | PluginNotFound | Plugin not found under any registered catalog | 2 | — |
| 21 | PluginAlreadyInState | Plugin already in target state (enabled/disabled) | 2 | — |
| 22 | PluginManifestParseError | `plugin.json` parse or validation failed | 2 | FR-013b |
| 23 | SkillFrontmatterParseError | `SKILL.md` frontmatter parse failed | 2 | — |
| 30 | ModelMissing | Model files not found on disk | 2 | — |
| 31 | ModelCorrupt | Model metadata invalid or placeholder checksum | 2 | — |
| 32 | ModelChecksumMismatch | SHA-256 or size mismatch on download | 2 | — |
| 33 | ModelRegistrationParseError | Model manifest.json invalid | 2 | — |
| 53 | CatalogHasEnabledPlugins | Catalog has enabled plugins; remove with `--force` to cascade disable | 9 | FR-045 |
| 54 | NotATerminal | Interactive command run without terminal (stdin/stdout not TTY) | 4 | FR-051 |

**Enforcement**: Closed enum `TomeError` in `src/error.rs`. Adding a variant forces edits to:
1. `TomeError` enum
2. `exit_code()` match arm
3. `category()` match arm
4. `tests/exit_codes.rs` assertions

No generic "Other" or "Unknown" variant. Every path maps to a named category.

**Phase 8 Status Exit Semantics** (`src/commands/status.rs::run`, PR #29):
- Exit 0: Overall health is Ok
- Exit 1: Overall health is Degraded (reranker drift only) or Unhealthy (embedder drift, model corrupt, index corrupt)
- Non-zero cases emit `std::process::exit(1)` directly after reporting, bypassing `TomeError` propagation

### Error Messaging

| Requirement | Implementation | Location |
|-------------|----------------|----------|
| **Name what failed** | Error variant includes resource name | `TomeError::CatalogNotFound(String)` |
| **Name where it failed** | File paths in error variants | `TomeError::ModelChecksumMismatch { model, expected, got }` |
| **Suggest next action** | Remediation in error message | `ModelChecksumMismatch` suggests `--force` retry |
| **Surface upstream errors** | Git and HTTP stderr passed through scrubber | `TomeError::GitFailed`, HTTP errors in `Io` with scrubbed detail |

Example error (model checksum):
```
model `bge-small-en-v1.5` SHA-256 mismatch: expected 51f1bd0..., got a1b2c3...; run `tome models download --force` to retry
```

## Interactive Flows & Terminal Enforcement

### TTY Requirement (Phase 4)

| Control | Implementation | Exit Code |
|---------|----------------|-----------|
| **TTY check** | `presentation::prompt::require_terminal()` checks both stdin and stdout | 54 (NotATerminal) |
| **Flow entry** | `tome plugin` (no subcommand) enforces TTY before any prompt (FR-051) | Exit 54 if no TTY |
| **Prompt functions** | Every `select()`, `multiselect()`, `confirm()` repeats the check | Exit 54 per call |
| **Reason** | `inquire` library writes prompt and reads echo on stdout; piped/redirected stdout causes mangled prompts and mismatched inputs | Security + UX |

**Implementation** (`src/presentation/prompt.rs::require_terminal()`):
```rust
pub fn require_terminal() -> Result<(), TomeError> {
    if output::stdin_is_tty() && output::stdout_is_tty() {
        Ok(())
    } else {
        Err(TomeError::NotATerminal)
    }
}
```

**Non-interactive alternatives** (FR-052):
- `tome plugin enable|disable|show` — CLI flags / positional args, no prompt
- `tome plugin list` — non-interactive listing with filters
- Model download within enable: refused with pointer to `--force` flag if no TTY

### TTY Enforcement in Plugin Disable (Phase 5)

| Control | Implementation | Location | Exit Code |
|---------|----------------|----------|-----------|
| **Confirmation prompt** | User must approve disable action via TTY prompt; decline returns 0 (no state change) | `src/commands/plugin/disable.rs:52–62` | — |
| **Non-TTY without flag** | If `--force` not supplied and stdin/stdout not TTY, refuse before prompt | `src/commands/plugin/disable.rs:36–49` | 54 |
| **Pointer message** | Emit documented message to stderr to guide users to `--force` | `src/commands/plugin/disable.rs:44–47` | — |
| **Decline semantics** | User declining the prompt is clean exit with no error; state unchanged | `src/commands/plugin/disable.rs:54–61` | 0 |

**Guarantee** (FR-051):
- Interactive flows will not execute in non-TTY contexts (CI, pipes, background)
- Exit code 54 is a clear signal that the caller needs an interactive context or a non-interactive alternative (`--force`)

**Test coverage**: `tests/plugin_disable.rs::disable_without_force_in_non_tty_context_exits_54_with_pointer_message()` verifies non-TTY refusal.

## Signal Handling & Cancellation

### SIGINT Handling

| Component | Implementation | Location |
|-----------|----------------|----------|
| **Handler install** | One-time `ctrlc::set_handler` | `src/catalog/git.rs::install_signal_handler` |
| **Cancellation flag** | Global `AtomicBool` with `SeqCst` ordering | `src/catalog/git.rs::CANCELLED` |
| **Child cleanup** | `Child::kill()` on flag flip | `src/catalog/git.rs::Git::run` |
| **Cache integrity** | Atomic write via temp-dir RAII | `src/catalog/store.rs::write_atomic` |
| **Model download safety** | `.partial/` cleanup on cancellation | `src/embedding/download.rs::download_model` (lines 77–87) |
| **Exit code** | 8 (documented, non-zero) | `TomeError::Interrupted` |

**Guarantees** (FR-026a, SC-011):
- Long-running Git operations (clone, fetch) are pollable for cancellation
- On cancellation, child process is killed and control returns to main
- No orphaned child processes
- Per-catalog cache atomicity is preserved (temp dir dropped via RAII)
- Model download `.partial/` directory cleaned up on interruption

**Interactive cancellation** (Phase 4):
- `inquire` library converts Ctrl-C / Ctrl-D to `InquireError::OperationCanceled` or `OperationInterrupted`
- `src/presentation/prompt.rs::prompt_error_to_tome` maps both to `TomeError::Interrupted` (exit 8)
- Semantically: user-initiated interruption is indistinguishable from system SIGINT (both exit 8)

**Test**: `tests/atomicity.rs` and signal-handling verification in integration tests.

## Atomic Writes

### Registry, Cache, Model, and Index Persistence

| Operation | Atomicity Guarantee | Implementation |
|-----------|-------------------|-----------------|
| **Registry write** | Atomic replace via temp + rename | `src/catalog/store.rs::write_atomic` |
| **Catalog cache refresh** | Atomic temp dir swap per catalog | `src/catalog/store.rs::clone_and_validate` |
| **Model download persist** | Atomic `.partial/` → final dir rename | `src/embedding/download.rs::download_model`, step 4 |
| **Model manifest write** | Atomic write via temp + rename | `src/embedding/download.rs::write_manifest` |
| **Index database enable** | Per-plugin transaction (all-or-nothing skill upsert) | `src/index/skills.rs::enable_plugin_atomic` |
| **Index database reindex** | Per-plugin transaction (snapshot diff → add/modify/remove/unchanged) | `src/index/skills.rs::reindex_plugin_atomic` (Phase 7) |
| **Catalog cascade disable** | Single lock acquisition for multi-plugin batch disable (Phase 9) | `src/plugin/lifecycle.rs::cascade_disable_for_catalog` + `src/index/skills.rs::delete_by_plugin` |
| **Temp file cleanup** | RAII via `tempfile::NamedTempFile` + `TempDir` | Rust Drop trait |

**Mechanism**:
1. Write to a temporary file/directory in the same directory as the target
2. Rename (POSIX `rename(2)` is atomic on same filesystem)
3. On error, temp file/directory is cleaned up; target is unchanged

**Guarantees** (FR-017a, FR-017b, SC-012):
- A partial or interrupted write leaves the on-disk file in either pre-state or post-state, never partial
- Multiple concurrent invocations see either the old version or the new version, never a mixture
- Test coverage: `tests/atomicity.rs` with concurrent writes and simulated interruption

**Note on per-plugin atomicity** (Phase 7): When reindexing multiple plugins (e.g., via `tome catalog update`), each plugin's reindex runs in its own transaction. A SIGINT between plugins leaves earlier plugins committed and later plugins unchanged. This is intentional (see CONCERNS.md for design rationale); the index is always in a valid state with no partial rows. The per-plugin boundary is where atomicity breaks for multi-plugin operations.

**Phase 9 catalog cascade disable** (PR #32): When removing a catalog with `--force`, all enabled plugins are cascade-disabled under a single advisory-lock acquisition. Each plugin's deletion runs as its own transaction, so a SIGINT between plugins leaves earlier plugins' rows dropped and later plugins' rows intact. The index remains consistent (no partial rows), and the operation is atomic at the lock-window boundary. See CONCERNS.md for trade-off notes.

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
| MSRV verification | On every PR | `rust-version` in Cargo.toml (pinned at 1.93) | Tested on pinned MSRV |
| Binary size check | On release builds | 50 MB stripped limit (Constitution amendment) | Fails if exceeded |

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

**Current dependencies**:
- Phase 1: `clap` (CLI), `serde`/`toml` (config), `thiserror`/`anyhow` (errors), `tracing` (logging), `sha2`/`hex` (hashing), `tempfile` (atomicity), `ctrlc` (signals), `regex` (scrubbing), `semver` (versions), `time` (timestamps), `directories` (paths)
- Phase 2: `rusqlite` (bundled SQLite), `sqlite-vec` (vendored vector extension), `fastembed-rs` (inference), `reqwest` (HTTP), `indicatif` (progress), `comfy-table` (tables), `owo-colors` (colour), `inquire` (prompts)
- Phase 4–5: No new dependencies (interactive flow and disable use existing `inquire`)

All fall within permissive licences. Phase 2 deps licensed: `fastembed-rs` (MIT), `ort` (MIT, transitive via fastembed), BGE models (MIT). Phase 4–5 deps: `inquire` (MIT).

## Binary Size & Deployment

### Size Constraint

| Metric | Limit | Current | Status |
|--------|-------|---------|--------|
| Stripped release binary | 50 MB | ~29.56 MB (Phase 3 slice 1b) | ✓ Within budget |
| Binary size growth | Must justify | N/A | Checked on every release build (CI) |

**Amendment** (CONSTITUTION.md v1.2.0): Original 10 MB cap at Phase 1 ratification. Phase 2 integration of ONNX Runtime (via `fastembed` → `ort`) measured ~29.56 MB on Linux. The worst-case projection in Phase 2 research (§Binary size budget) underestimated `ort`'s impact. Current cap of 50 MB is sized to Phase 3 reality with 20.4 MB headroom for query, reindex, and the MCP server. Discipline holds; only the number changed. Justification is recorded in the research doc and decision log.

**Enforcement**: CI job `Release binary size check` in `.github/workflows/ci.yml`:
```bash
cargo build --release
size=$(stat -c%s target/release/tome)
if [ "$size" -ge 52428800 ]; then  # 50 MB
  echo "::error::Release binary exceeds the 50 MB limit"
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

**Implementation**: Scrubbing happens at the capture point (Git stderr → `scrub_to_string`, HTTP errors → `scrub_for_diag`), ensuring no downstream path leaks credentials.

### Version Output (Phase 8)

| Feature | Mechanism | Purpose |
|---------|-----------|---------|
| **Pre-parse hook** | Env args scanned before clap dispatch | Allows `--version` to include model identities before CLI setup |
| **Embedder identity** | Emitted from `MODEL_REGISTRY` | Public constant, reproducibility set when filing bugs |
| **Reranker identity** | Emitted from `MODEL_REGISTRY` | Public constant, reproducibility set when filing bugs |
| **`--json` support** | Structured output per `contracts/version-output.md` | Automation-friendly serialization |
| **No secrets exposed** | Model names/versions are public registry entries | Safe to emit to stdout |

**Implementation** (`src/main.rs:13–16`, `src/commands/status.rs::print_version`):
```rust
// Pre-parse: scan for --version / -V before clap dispatch
let raw: Vec<String> = std::env::args().collect();
if raw.iter().skip(1).any(|a| a == "--version" || a == "-V") {
    let json = raw.iter().any(|a| a == "--json");
    commands::status::print_version(json);
    std::process::exit(0);
}
```

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
| Index database | Advisory lockfile `index.lock` (Phase 2) | `src/index/lock.rs` |
| Catalog cache | Per-catalog atomic swap — no cross-catalog lock |
| Global state | None (sync-only, CLI is per-invocation) |
| Status command | No lock (read-only, non-invasive health check) | Designed per FR-056 |

**Rationale**: Phase 1 is synchronous and single-process. Concurrent invocations are safe because:
- Registry writes are atomic (rename)
- Cache writes are atomic per catalog
- Concurrent readers see either the old state or the new state

Phase 2 introduces index database with WAL + advisory lockfile (FR-040) to coordinate concurrent access across harness instances.

**Phase 8 Status Lock-Free** (PR #29): `tome status` never acquires the advisory lock. This allows it to run as a pre-flight health check even when another writer is holding the lock, supporting its use as a non-invasive doctor command.

**Phase 9 Cascade Disable** (PR #32): The pre-check that queries `enabled_plugins_for_catalog` runs WITHOUT the lock. This is intentional — readers don't block writers, and the only risk is a TOCTOU where another process enables a plugin between the check and the cascade. In that case the cascade simply drops the additional plugin's rows too (still correct), or the user re-runs after seeing the refuse error. The cascade itself runs under a single lock acquisition; each plugin's deletion is its own transaction.

**Future consideration**: Phase 2 MCP server concurrency model is locked down in spec (FR-040); Phase 3 testing against real BGE models is pending (SC-001/SC-002, T088).

## Security Testing

### Test Categories

| Category | Coverage | Files |
|----------|----------|-------|
| **Path validation** | Exhaustive negative corpus (URLs, absolutes, `..`, escapes, symlinks) | `tests/path_validation.rs` (11 cases) |
| **Plugin identity** | Shape validation (no `/`, no `..`, no leading `.`) | `tests/plugin_*.rs` integration suites |
| **Scrubbing** | All four regex rules + ordering + edge cases | `tests/scrubbing.rs` (8 cases) |
| **Strictness** | Every Tome-owned Deserialize struct has `deny_unknown_fields` | `tests/manifest_strictness.rs` (2 assertions) |
| **Model integrity** | SHA-256 verification, placeholder detection, atomic persist, re-verification | `tests/models_download.rs`, `tests/models_list.rs` |
| **Atomicity** | Concurrent writes, partial writes, interruption | `tests/atomicity.rs` (4 cases) |
| **Exit codes** | Every `TomeError` variant maps to documented code | `tests/exit_codes.rs` |
| **TTY enforcement** | Non-TTY refusal at interactive flow entry and confirmation prompts; pointer messages | `tests/plugin_interactive.rs`, `tests/plugin_disable.rs` |
| **Integration** | Real Git repos, real fixtures, real filesystems | `tests/catalog_*.rs`, `tests/models_*.rs`, `tests/plugin_*.rs` |
| **Status health check** | Report assembly, drift detection, overall classification | `tests/status.rs` (Phase 8) |
| **Catalog cascade** | Enabled-plugin detection, per-plugin deletion under lock, error handling | `tests/catalog_remove.rs` (Phase 9) |

**Success criteria**:
- SC-005: 100% of malformed inputs rejected with helpful errors
- SC-006: No credential material observable in any output
- SC-011: Interruption leaves no orphaned processes
- SC-012: Mid-write interruption leaves recoverable state
- SC-051: Interactive flows refuse to run in non-TTY contexts

## Known Gaps & Future Work

| Concern | Phase | Note |
|---------|-------|------|
| Real BGE model testing (SC-001/SC-002) | Phase 3 | T088 — requires developer-machine pass |
| Model-download byte-progress callback | Phase 3 onward | Currently wrapped in indeterminate spinner; both `plugin enable` and `models download` would benefit from byte-progress refactor (TD-010) |
| User-declines-model-download exit code | Phase 3+ | Currently reuses 8 (user-initiated abort); worth locking down in future iteration |
| Encryption at rest for sensitive caches | Phase 3+ | Deferred until use case demands it |
| Audit logging | Phase 3+ | Not required in Phase 1 (single-user CLI) |
| Rate limiting | Not applicable | CLI tool, not a service |

---

*This document defines security controls. Update when security posture changes.*
