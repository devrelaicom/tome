# Coding Conventions

> **Purpose**: Document code style, naming conventions, error handling, and common patterns.
> **Generated**: 2026-05-14
> **Last Updated**: 2026-05-14

## Code Style

### Formatting Tools

| Tool | Configuration | Command |
|------|---------------|---------|
| rustfmt | `rustfmt.toml` (edition 2024) | `cargo fmt --check` |
| Clippy | `.clippy.toml` (implied by -D warnings) | `cargo clippy --all-targets --all-features -- -D warnings` |
| Typos | Implicit (no config file) | `typos` |

All three gates are enforced locally via `.githooks/pre-commit` and in CI. The hook runs them sequentially; use `git commit --no-verify` to bypass with explicit justification in the commit message.

### Style Rules

| Rule | Convention |
|------|------------|
| Indentation | 4 spaces (Rust default) |
| Edition | Rust 2024 |
| MSRV | 1.93 (locked in `Cargo.toml` `rust-version`; verified in CI) |
| Line length | No hard limit; rustfmt uses defaults |
| Trailing commas | Automatic via rustfmt |

## Naming Conventions

### Files & Directories

| Type | Convention | Example |
|------|------------|---------|
| Modules | snake_case | `src/plugin/lifecycle.rs` |
| Tests (separate dir) | snake_case + descriptive | `tests/plugin_enable.rs` |
| Fixtures | lowercase | `tests/fixtures/sample-catalog/` |
| Capabilities | snake_case subdirs | `src/index/`, `src/embedding/` |

### Code Elements

| Type | Convention | Example |
|------|------------|---------|
| Variables | snake_case | `config_dir`, `embedder_seed` |
| Constants | SCREAMING_SNAKE_CASE | `GRACEFUL_SHUTDOWN_TIMEOUT`, `MIGRATIONS` |
| Functions | snake_case, verb prefix for actions | `apply_pending`, `open_read_only` |
| Structs | PascalCase | `TomeError`, `WorkspaceInfo`, `LifecycleDeps` |
| Enums | PascalCase, variant singular/context | `Scope::Global`, `Scope::Workspace`, `CatalogCacheState::Missing` |
| Traits | PascalCase | `Embedder`, `Reranker`, `Serializable` |
| Module docs | Doc comment on first line explaining role | `//! MCP server state and initialization` |

## Error Handling

### Closed Error Enum Pattern

Tome uses a **closed enumeration** for all errors: `TomeError` in `src/error.rs` has no `Other`/`Unknown` variant. Every failure class maps to exactly one enumerated variant and a unique exit code. Adding a variant **forces edits** to:

- `tests/exit_codes.rs` — compiler enforces coverage
- `specs/*/contracts/exit-codes-*.md` — the spec's authority
- The PRD — the external contract

This design makes exit codes stable and discoverable; trade-off is that new failure modes require a deliberate variant addition.

### Error Pattern Examples

**Single `#[from]` source per enum variant** (when the source is semantically equivalent):
```rust
#[error("io: {0}")]
Io(#[from] std::io::Error),
```

**Tuple variants for rich context**:
```rust
#[error("git failed for `{catalog}`: {detail}")]
GitFailed { catalog: String, detail: String },
```

**Nested enum for context-specific variants**:
```rust
#[error("plugin `{plugin}` is already {state}")]
PluginAlreadyInState { plugin: String, state: PluginState },
```

**Display message includes recovery hints**:
```rust
#[error("model `{model}` is missing; run `tome models download`")]
ModelMissing { model: String },
```

### Error Propagation

- Library functions return `Result<T, TomeError>`.
- The CLI layer in `src/main.rs` maps `TomeError` to `std::process::exit(code)` via `error::into_exit_code()`.
- Tests use `expect()` / `unwrap()` on library calls; integration tests check the exit code via `Command::status()`.

### Logging Conventions

Logging uses `tracing` (sync path) + `tracing-subscriber` with JSON output on stderr (Phase 2). MCP async handlers use a file log (`mcp.log`) in JSON-lines format to preserve stdout as the MCP protocol channel (contract FR-221).

| Level | When to Use |
|-------|-------------|
| error | Failures that should never happen (logic bugs, disk corruption, signal handler events) |
| warn | Recoverable issues (model fetch retries, catalog re-clone fallbacks) |
| info | Important state transitions (schema migration applied, MCP server started) |
| debug | Development details (query result counts, embedder vector dimensions) |
| trace | Not used in production; left for future internal diagnostics |

Credential scrubbing (from git URLs, model download URLs, error chains) is applied at the boundary via `catalog::git::scrub_credentials` and `scrub_to_string()`.

## Common Patterns

### Silent Compute + Emit Wrapper

When a CLI command's compute path is reused by a non-CLI surface (MCP, library API), split the implementation:

```rust
// Silent compute — no I/O side-effects, returns structured result
pub fn pipeline(args, deps) -> Result<Outcome, Error> {
    // Do the work
}

// Emit wrapper — calls pipeline, then emits per mode (human/JSON)
pub fn run_with_deps(args, deps, mode) -> Result<Outcome, Error> {
    let outcome = pipeline(args, deps)?;
    emit(&outcome, mode)?;
    Ok(outcome)
}

// CLI calls run_with_deps; tests call pipeline directly
```

**Examples**:
- `commands/query.rs` — `pipeline()` for KNN+rerank, `run_with_deps()` for CLI emit
- `commands/workspace/info.rs` — `assemble()` for compute, `run()` for emit
- `commands/status.rs` — `assemble_report()` for compute, `run()` for emit

### Sync Boundary (Phase 3)

Async/await is strictly confined to `src/mcp/` (the MCP server module). Every file under `src/mcp/` may use:
- `async fn`, `.await`, `tokio::task::spawn_blocking`, `tokio::sync::`
- The `tokio` runtime is single-threaded (`rt` feature only; no `rt-multi-thread`)

Every file outside `src/mcp/` is synchronous. **Structural test** `tests/sync_boundary.rs` enforces this: it fails the build if any non-MCP module references `tokio`.

**Rationale**: The MCP server needs async for the protocol handler loop, but the core library (catalog, plugin, index, embedding) stays sync for simplicity and binary size. Tool handlers that need sync work (rusqlite queries, ONNX inference) use `spawn_blocking` to avoid blocking the reactor.

### spawn_blocking in MCP Tool Handlers

When an MCP tool handler (which is `async fn`) needs to invoke sync code (index queries, embedder inference):

```rust
pub async fn handle(state: Arc<McpState>, input: Input) -> Result<Output, McpError> {
    // Cheap async work (validation, early checks)
    if input.is_invalid() {
        return Err(error);
    }

    // Heavy sync work → spawn_blocking
    let outcome = tokio::task::spawn_blocking(move || {
        // rusqlite, fastembed, etc. work here
        do_expensive_sync_work()
    }).await?;

    Ok(output_from(outcome))
}
```

The runtime stays responsive; sync inference latency doesn't hold up other protocol messages. See `src/mcp/tools/search_skills.rs` and `src/mcp/tools/get_skill.rs` for examples.

### Atomic Populated-Directory Landing

When multiple files must appear either completely or not at all (e.g., `.tome/` with `config.toml` + `index.db`), use staging + rename on the **same filesystem**:

```rust
// 1. Create staging dir **inside the target parent** so rename is atomic
let staging = tempfile::Builder::new()
    .prefix(".tome.tmp.")
    .tempdir_in(&absolute)?;  // Same filesystem

// 2. Populate the staging dir
std::fs::write(staging.path().join("config.toml"), config_body)?;
// ... more writes ...

// 3. Consume the TempDir guard → drop auto-cleanup
let staged_path = staging.keep()?;

// 4. Atomic rename (POSIX-atomic when source and target are on same FS)
std::fs::rename(&staged_path, &target)?;
```

**Why this design**:
- Rename is atomic when both paths are on the same filesystem
- A SIGINT/crash mid-populate leaves the staging dir (which starts with `.tmp.`) unvisited by readers
- No temporary files spill into `$TMPDIR` on a different filesystem

See `src/workspace/init.rs` for the full pattern, including rollback semantics (`--force` renames an existing target to `.old` first).

### Opt-In Registry File Pattern

For "track this for me, but only if I ask" features (e.g., workspace registry at `${state_dir}/workspaces.txt`), use:

```rust
pub fn append_if_registry_exists(path: &Path, item: &str) -> Result<(), TomeError> {
    if !path.is_file() {
        return Ok(()); // No-op if file doesn't exist
    }
    // File exists → append (deduped by exact-match)
    let mut existing = std::fs::read_to_string(path)?;
    if !existing.lines().any(|line| line == item) {
        existing.push_str("\n");
        existing.push_str(item);
        std::fs::write(path, existing)?;
    }
    Ok(())
}
```

The user must touch the registry file **once** to opt in; subsequent operations append. No special initialization.

See `src/workspace/inventory.rs`.

### Content-Addressed Shared Resource Reference Counting

When a resource is shared across scopes (e.g., on-disk catalog clones at `catalogs/<sha256>/`), use ad-hoc enumeration instead of unconditional deletion:

```rust
pub fn reference_count(url: &str, paths: &Paths) -> Result<Vec<Scope>, TomeError> {
    let mut scopes = Vec::new();
    
    // Global config
    if global_config_refs_url(url, paths)? {
        scopes.push(Scope::Global);
    }
    
    // Each workspace in the opt-in registry
    for ws_root in scopes_from_registry(paths)? {
        if workspace_config_refs_url(url, paths, ws_root)? {
            scopes.push(Scope::Workspace(ws_root));
        }
    }
    
    Ok(scopes)
}

// Then in catalog::remove:
let refs = reference_count(&entry.url, &paths)?;
if refs.is_empty() {
    fs::remove_dir_all(&entry.path)?; // Safe to delete
}
```

**TOCTOU profile**: The enumeration is not locked. A concurrent remove + add race benignly (one wins; the other may leave a dangling reference recoverable by re-fetching). Same profile as Phase 2's `cascade_disable_for_catalog` pre-check.

See `src/catalog/store.rs` for the full implementation.

### Reuse Closed-Set Variants Over Promoting New Ones

When a new failure mode maps semantically onto an existing `TomeError` variant + exit code, prefer reuse:

```rust
// New failure: workspace already initialised
// Existing variant: CatalogAlreadyExists (code 4)
// Decision: reuse

return Err(TomeError::CatalogAlreadyExists(format!(
    "workspace at {}",
    marker.display()
)));
```

**Cost**: The Display message may be slightly off (e.g., "catalog `workspace at /path/.tome` is already registered"). **Benefit**: Zero variants added, zero exit-code churn, zero JSON envelope shape change.

Promote a new variant only if a specific failure mode needs to be distinguished from the existing one. Examples:
- Phase 3 added `WorkspaceMalformed` (70) because workspace-specific errors (malformed config, unopenable index) differ from catalog errors
- Phase 3 added `SchemaVersionTooNew` (73) because it's a distinct refusal from all prior database errors

See the `TomeError` enum and the P3/P4 retros for examples.

### home: &Path Test Isolation Hook

When a library function would otherwise read `$HOME` directly, accept it as a parameter:

```rust
pub fn assemble_report(
    scope: &ResolvedScope,
    paths: &Paths,
    home: &Path,  // ← Parameter, not env
    verify: bool,
) -> Result<DoctorReport, TomeError> {
    // Harness detection: probe six well-known dirs under home
    let harness_dirs = vec![
        home.join(".claude"),
        home.join(".cursor"),
        // ...
    ];
    // ...
}
```

**Tests** pass a `TempDir`-rooted path; **production** passes `std::env::var("HOME")`. Drops the dependency on `serial_test` for env-isolated tests.

See `src/doctor/harness_detect.rs`.

### Subsystem String Routing for Tagged Repair Lists

When a dispatch list needs per-item routing (e.g., suggested fixes by subsystem), encode the key as a string field:

```rust
pub struct SuggestedFix {
    pub auto_fixable: bool,
    pub subsystem: String,  // "embedder" / "reranker" / "catalog:name" / "schema"
    pub description: String,
}

// Dispatch in doctor::fixes::apply:
for fix in report.suggested_fixes.iter() {
    if fix.subsystem == "embedder" {
        // Repair embedder
    } else if let Some(catalog_name) = fix.subsystem.strip_prefix("catalog:") {
        // Repair catalog_name
    } else if fix.subsystem == "schema" {
        // Forward-migrate schema
    }
}
```

**Trade-off**: Less type-safe than an enum, but simpler for wire serialization and per-instance data extraction (e.g., `.strip_prefix("catalog:")`). Flat if-else dispatch ladder works up to ~6 arms.

See `src/doctor/mod.rs`.

### re_assemble Post-Mutation Pattern

When an "apply" function mutates per-field state in place (e.g., `fixes::apply_one` updates `report.embedder` / `report.catalogs` after repairs), expose a sibling `re_assemble` that recomputes derived state without re-running expensive checks:

```rust
pub fn apply(report: &mut DoctorReport, paths: &Paths, scope: &Scope) -> Result<(), TomeError> {
    for fix in report.suggested_fixes.clone() {
        apply_one(report, fix, paths, scope)?;
        // Expensive FS work inside apply_one
    }
    Ok(())
}

pub fn re_assemble(report: &mut DoctorReport) {
    // Rebuild suggested_fixes + overall from the updated embedder/catalogs/etc.
    // WITHOUT re-probing catalogs, re-running integrity_check, etc.
    report.suggested_fixes = build_suggested_fixes(report);
    report.overall = classify(report);
}

// Caller:
doctor::fixes::apply(&mut report, paths, scope)?;
doctor::fixes::re_assemble(&mut report);
```

Halves the FS cost on repeat reads. See `src/doctor/fixes.rs`.

### Test-Only Injection via #[doc(hidden)] pub static

Integration tests (under `tests/`) run outside the crate and don't see `#[cfg(test)]` items. For test-only injection points, use `#[doc(hidden)] pub static`:

```rust
// src/index/migrations.rs
#[doc(hidden)]
pub static MIGRATIONS_OVERRIDE: RefCell<Option<&'static [Migration]>> =
    const { RefCell::new(None) };

// tests/schema_migration_e2e.rs
#[test]
fn synthetic_migration_succeeds() {
    static MIGRATIONS: &[Migration] = &[/* synthetic */];
    let _guard = MigrationsGuard::install(MIGRATIONS);
    // Test body
}
```

The `#[doc(hidden)]` keeps it out of the published API; the doc comment explains it's test-only.

### RAII MigrationsGuard for Thread-Local Injection

Pair a `thread_local!` injection point with a guard struct in the test file:

```rust
struct MigrationsGuard;

impl MigrationsGuard {
    fn install(migrations: &'static [Migration]) -> Self {
        MIGRATIONS_OVERRIDE.with(|slot| *slot.borrow_mut() = Some(migrations));
        Self
    }
}

impl Drop for MigrationsGuard {
    fn drop(&mut self) {
        MIGRATIONS_OVERRIDE.with(|slot| *slot.borrow_mut() = None);
    }
}

#[test]
fn test_migration() {
    let _guard = MigrationsGuard::install(&[/* ... */]);
    // Use MIGRATIONS_OVERRIDE; guard clears it on drop
}
```

Survives panics; no manual teardown; no cross-test leakage. See `tests/schema_migration_e2e.rs`.

### Emit-Only Serialize Records

Wire-shape types (e.g., `WorkspaceInfo`, `InitOutcome`, `DoctorReport`) are never deserialized — they exist to render `--json` for stdout. Omit `#[serde(deny_unknown_fields)]` (the strictness boundary applies only to Tome-owned **inputs**). Pin the wire format with a JSON serialization test instead:

```rust
#[test]
fn workspace_info_json_shape_is_byte_stable() {
    let info = WorkspaceInfo { /* ... */ };
    let json = serde_json::to_string(&info).unwrap();
    assert_eq!(json, expected_json);  // Field order, null vs absent, etc.
}
```

See `tests/workspace_info.rs` for examples.

### Library/CLI Test Boundary

Tests are split into **library API** (test state via `StubEmbedder` / `StubReranker`) and **CLI binary** (real process invocation via `Command`):

- **Heavy-state paths** (enable plugin, download model, reindex) → library API + `StubEmbedder`; exercising the full embedder path is deferred to manual verification or integration tests with real models.
- **Light/error paths** (list, show, disable, remove with `--force`) → CLI binary; no embedder load needed.
- **Interactive paths** (bare `tome plugin`, non-TTY refusals) → CLI binary via `rexpect` (pty harness).

**Rationale**: The full `FastembedEmbedder` loads multi-MB ONNX models; CI is fast when we use `StubEmbedder` for correctness and defer real-model testing to manual passes. See PR #3 (Phase 3 / User Story 1) for the established split.

## Import Ordering

Standard import order in Rust modules:

1. Standard library (`std::`, `core::`)
2. External crates (alphabetical)
3. Internal crate modules
4. Relative imports
5. Type-only imports (`use X as Y` for disambiguation)

Example:
```rust
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tempfile::TempDir;

use crate::config::Config;
use crate::paths::Paths;

use super::store;
use super::manifest::CatalogManifest;
```

## Comments & Documentation

| Type | When to Use | Format |
|------|-------------|--------|
| Module docs | Top of every `mod.rs` or single-file module | `//! Explanation of module purpose` |
| Doc comments | Public APIs, complex types, important fields | `/// Explanation` or `/** Explanation */` |
| Inline comments | Complex logic, why not what | `// Explanation` |
| TODO | Planned work with context | `// TODO: <issue>: description` |
| FIXME | Known issues needing attention | `// FIXME: description` |

**Doc comment convention**: Explain the *why*. Readers understand Rust syntax; explain design decisions, contract guarantees, and invariants.

Example:
```rust
/// Atomic landing of `.tome/` via staging + rename on the same filesystem.
/// 
/// A SIGINT/crash mid-populate leaves a staging dir starting with `.tmp.`
/// unvisited by readers. The final rename is POSIX-atomic when both paths
/// are on the same FS, so `.tome/` either appears complete or doesn't appear.
pub fn init(target_root: &Path, /* ... */) -> Result<InitOutcome, TomeError> {
    // ...
}
```

## Git Conventions

### Commit Messages

Format: `type(scope): description`

| Type | Usage | Example |
|------|-------|---------|
| feat | New feature | `feat(workspace): add tome workspace init` |
| fix | Bug fix | `fix(doctor): embedder drift detection was inverted` |
| refactor | Code restructure | `refactor(query): extract silent compute path` |
| test | Adding/improving tests | `test(migrations): add e2e coverage for forward schema steps` |
| docs | Documentation updates | `docs: refresh CONVENTIONS.md for Phase 3 patterns` |
| chore | Maintenance | `chore: bump tokio to 1.40` |

Enforced locally by `.githooks/pre-commit` hook running `cog verify` (Cocogitto).

**Soft cap**: ~400 lines per commit or 2 modules max. Small PRs are easier to review.

**Commit body**: Explain *why* the change is needed, not *what* changed (that's in the diff). Reference issue numbers, contracts, design rationale.

Example:
```
feat(doctor): suggest fixes for broken catalog caches

The doctor now emits SuggestedFix records for catalogs in Missing or
NotARepo states. The fix applies a re-clone via Git::clone_shallow under
the advisory lock.

Closes US4.a per contracts/doctor.md §Repair classes.
Ref: specs/003-phase-3-mcp-workspaces/research.md §R-13 (repair dispatch)
```

### Branch Naming

Short-lived feature branches off `main`:
- `feature/something-descriptive`
- `fix/issue-number-description`
- `docs/topic`

Delete after merge.

### PRs

- One user story slice per PR (or one foundational slice).
- If a commit + tests fit in ~250 lines AND nothing needs deeper review than tests provide, combine into one commit.
- Use PR descriptions to link specs, contracts, and prior art.
- Request review from 1–2 domain experts per PR (Rust-lens, security, contracts, etc. as needed).

## Architectural Constraints

From `CONSTITUTION.md`:

- **Sync only** outside `src/mcp/`. No async/tokio outside the MCP module.
- **Closed error set**. No `Other`/`Unknown` variant in `TomeError`.
- **Strictness boundary**. Tome-owned config/manifest files use `#[serde(deny_unknown_fields)]`; third-party inputs (`plugin.json`, `SKILL.md` frontmatter) parse leniently.
- **Atomic writes**. All multi-file operations use staging + rename or SQLite transactions.
- **Credential scrubbing at the boundary**. Git URLs, model download URLs, error chains are scrubbed before logging/display.
- **50 MB binary cap** (revised from 10 MB on 2026-05-13). Profile is `lto = "thin"`, `panic = "abort"`, `strip = "symbols"`. Non-waivable (NFR-001).

See `CONSTITUTION.md` for the full rationale.

---

*This document defines HOW to write code. Update when conventions change. Last refreshed 2026-05-14 against Phase 3 polish complete source.*
