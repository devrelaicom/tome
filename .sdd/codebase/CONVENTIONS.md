# Coding Conventions

> **Purpose**: Document code style, naming conventions, error handling, and common patterns.
> **Generated**: 2026-05-26
> **Last Updated**: 2026-05-26

## Code Style

### Formatting Tools

| Tool | Configuration | Command |
|------|---------------|---------|
| rustfmt | `rustfmt.toml` (edition 2024) | `cargo fmt --check` |
| Clippy | `.clippy.toml` (MSRV check only) | `cargo clippy --all-targets --all-features -- -D warnings` |
| Typos | `_typos.toml` | `typos` |

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
| Capabilities | snake_case subdirs | `src/index/`, `src/embedding/`, `src/harness/` |
| Temp staging dirs | `.tome.tmp.*` prefix | Used by `atomic_dir::land_directory` for crash safety |

### Code Elements

| Type | Convention | Example |
|------|------------|---------|
| Variables | snake_case | `config_dir`, `embedder_seed`, `workspace_name`, `project_root` |
| Constants | SCREAMING_SNAKE_CASE | `GRACEFUL_SHUTDOWN_TIMEOUT`, `MIGRATIONS`, `SHORT_MAX_CHARS` |
| Functions | snake_case, verb prefix for actions | `apply_pending`, `open_read_only`, `land_directory` |
| Structs | PascalCase | `TomeError`, `WorkspaceInfo`, `LifecycleDeps`, `BindDeps`, `HarnessModule` |
| Enums | PascalCase, variant singular/context | `Scope`, `CatalogCacheState::Missing`, `RulesFileStrategy` |
| Traits | PascalCase | `Embedder`, `Reranker`, `Serializable`, `HarnessModule`, `Summariser` |
| Module docs | Doc comment on first line explaining role | `//! MCP server state and initialization` |
| Newtype wrappers | PascalCase wrapping type | `WorkspaceName(String)`, `PluginId { catalog, plugin }` |

## Error Handling

### Closed Error Enum Pattern

Tome uses a **closed enumeration** for all errors: `TomeError` in `src/error.rs` has no `Other`/`Unknown` variant. Every failure class maps to exactly one enumerated variant and a unique exit code. Adding a variant **forces edits** to:

- `tests/exit_codes.rs` — compiler enforces coverage
- `specs/*/contracts/exit-codes-*.md` — the spec's authority
- The PRD — the external contract

This design makes exit codes stable and discoverable; trade-off is that new failure modes require a deliberate variant addition.

### Error Variant Organization

Variants are grouped by **Phase** with comments naming the exit code range:

```rust
// Phase 1 (codes 2–8, plus Internal=1). Unchanged.
// Phase 2 — plugin lifecycle (codes 20–23).
// Phase 3 — MCP / workspace (codes 60–75).
// Phase 4 — workspace name + harness + summariser (codes 13–19, 24).
```

**Pre-allocated variants** are added in Foundational phases **before any consumer exists**. Phase 4 F3 allocated codes 13–19, 24 before project binding and harness composition were implemented. Benefit: no mid-feature enum rewrites; the compiler enforces all arms are handled.

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
#[error("workspace `{name}` not found in the central registry")]
WorkspaceNotFound { name: String },
```

**Display message includes recovery hints**:
```rust
#[error("model `{model}` is missing; run `tome models download`")]
ModelMissing { model: String },
```

## Validation & Parsing

### `WorkspaceName` Newtype Validation

All workspace names flow through `WorkspaceName::parse(s: &str)` at **every input boundary**: CLI flags, TOML deserialization, environment variables, and file markers. The type is a newtype `struct WorkspaceName(String)` with immutable access via `as_str()`.

Rules per FR-347:
- 1–64 chars from `[a-zA-Z0-9_-]`
- Must not begin or end with `-` or `_`
- Must not be `.`, `..`, or empty
- Reserved name `"global"` parses but is flagged by `is_reserved()` for lifecycle commands

Deserialization hook (`impl Deserialize for WorkspaceName`) calls `WorkspaceName::parse`, so TOML/JSON round-trips are automatically validated.

**Pattern**:
```rust
let name = WorkspaceName::parse(user_input)?;  // Rejects at boundary
if name.is_reserved() {
    return Err(TomeError::...);  // Refuse reserved name in lifecycle commands
}
```

### `PluginId` Identity Parsing

Plugin addresses `catalog/plugin` are validated by `impl FromStr for PluginId`:
```rust
let id: PluginId = "my-catalog/my-plugin".parse()?;
```

Rejects embedded slashes, parent traversal (`..`), dot-prefixes, and absolute paths in either segment.

## Atomic-Directory Landing Pattern

Phase 4 promotes the atomic-directory landing pattern from `workspace::init` into a reusable helper under `src/util/atomic_dir.rs`. Used by workspace binding (project markers) and harness commands.

### Key Invariants

1. **Same-filesystem staging**: `tempfile::Builder::new().prefix(".tome.tmp.").tempdir_in(parent)` creates a sibling staging dir on the same filesystem as the target, guaranteeing POSIX-atomic rename.

2. **Mode preservation (Unix)**: On Unix, if the target exists, its file mode is captured, applied to the staging tempfile, and preserved through the rename. If target is absent, libc-default (typically `0o600`) wins.

3. **Crash safety**:
   - Crash before `TempDir::keep()`: staging dir auto-cleaned
   - Crash after `keep()` but before final rename: orphan staging dir picked up by `doctor --fix` (matching prefix `.tome.tmp.`)
   - **Replace variant**: on final rename failure, roll back the `.old` sibling before bubbling error

4. **Fsync**: staging directory is synced before `keep()` (best-effort on platforms where directory fsync is a no-op; matters on Linux/macOS).

**Public API**:
```rust
pub fn land_directory<F>(target: &Path, mode_unix: u32, populate: F) 
    -> Result<PathBuf, TomeError>
where F: FnOnce(&Path) -> Result<(), TomeError>

pub fn land_directory_with_replace<F>(
    target: &Path, mode_unix: u32, populate: F
) -> Result<PathBuf, TomeError>
```

Used in `src/workspace/binding.rs` for project marker creation and harness sync paths.

## Sync & Concurrency Patterns

### Two-Phase Sync Orchestrator (Phase 4)

The workspace binding and harness sync orchestrators follow a two-phase pattern:

- **Phase A (brief, under advisory lock)**: Read from the central database to identify the workspace and project binding.
- **Phase B (unlocked)**: Perform filesystem operations (rules file modifications, MCP config edits) without holding the lock. If a filesystem operation fails, the lock is released and the error bubbles; subsequent invocations will detect and fix the state.

This allows writers to proceed while filesystem I/O is in flight, improving responsiveness when harness syncs (e.g., git operations on large repos) would otherwise block the index.

### Idempotence-by-Mtime (Phase 4)

Rules file and MCP config primitives (`rules_file::read`, `rules_file::write`, `mcp_config::read`, `mcp_config::write`) short-circuit when bytes match:

```rust
if existing_bytes == new_bytes {
    return Ok(());  // No mtime change, no fsync, no atomic write
}
```

Tests verify idempotence by capturing mtime before a write, sleeping 1.5 seconds (to ensure distinct mtime granularity on all filesystems), re-reading, and asserting mtime unchanged.

### Advisory Lock Around Settings File Mutations (Phase 4 US3)

Any command that mutates a Tome-owned config file outside the central DB (e.g., `settings.toml`, `config.toml`) must acquire the index advisory lock for the full read-modify-write window:

```rust
let _lock = index::lock::LockFile::acquire(&paths.index_lock_path)?;
// Read, modify, write settings file
```

This serializes with index writers and prevents concurrent TOCTOU races on the same file.

## Per-Project Effective List Resolution (Phase 4 US2)

Workspace lifecycle commands that need to know the effective harness list for a project (e.g., `workspace remove` cascade cleanup) call:

```rust
let effective_list = settings::resolver::resolve_effective_list(&StubScope::new(), &paths)?;
```

The `StubScope::new()` represents a "not yet bound" project; phase A of the sync algorithm later replaces it with a real `ResolvedScope(workspace_name)` read from the database. This pattern avoids DB access until the workspace is actually confirmed to exist.

In tests, use `StubScope` when the database isn't fully initialized; in production code, use the resolved scope from the sync orchestrator.

## Settings File Composition and Inheritance (Phase 4 US3)

Three layers of settings compose in priority order: global (`~/.tome/settings.toml`) < workspace (`~/.tome/workspaces/<name>/settings.toml`) < project (`<root>/.tome/config.toml`):

```rust
pub fn resolve_effective_list(scope: &Scope, paths: &Paths) -> Result<Vec<String>, TomeError> {
    // 1. Read global (always exists or defaults)
    // 2. Overlay workspace (if workspace exists in central DB)
    // 3. Overlay project (if project marker is present)
    // First scope to declare harnesses wins; remainder ignored.
}
```

Key rule (FR-441): An explicitly empty list (`harnesses = []`) is semantically distinct from no declaration. The resolver stops at the first scope where `harnesses` is `Some(_)`, even if empty.

### `CentralDbScopeProvider` for Harness Resolution (Phase 4 US3)

When a future command needs to resolve the harness list for a workspace (e.g., `tome harness list`), use the production provider:

```rust
let provider = CentralDbScopeProvider::new(paths);
let list = settings::resolver::resolve_effective_list(&scope, paths, &provider)?;
```

The provider distinguishes three failure modes:
- **Workspace exists, settings present/parsable** → `Ok(Some(list))`
- **Workspace exists, settings absent** → `Ok(None)` (no harnesses declared; legal)
- **Workspace exists, settings unreadable/unparsable** → `Err(SettingsReadFailure)` → exit 70 (`WorkspaceMalformed`)
- **Workspace not in central DB** → `Err(UnknownWorkspace)` → exit 13 (`WorkspaceNotFound`)

When central DB is absent, only the privileged `global` workspace is considered to exist.

## Lenient Parse & Order Preservation

### Third-Party Data (Phase 4)

Harness MCP configuration files (JSON, TOML) are treated as **third-party data** and are parsed leniently:
- Unknown fields are preserved on round-trip (not rejected)
- Comment and key-order preservation via `toml_edit` (TOML) and `serde_json` with `preserve_order` feature (JSON)
- Only Tome-owned entries (under key `"tome"` with `command == "tome" && args[0] == "mcp"`) are mutated

Contrast with **Tome-owned manifests** (`config.toml`, `settings.toml`, `plugin.json` within Tome-controlled catalogs) which use strict `#[serde(deny_unknown_fields)]`.

### Atomic Write with Mode Preservation (Phase 4)

Every file write follows the pattern:
1. Read existing file and capture mode (if it exists)
2. Write to sibling tempfile in same directory (or use `NamedTempFile`)
3. Apply captured mode via `symlink_metadata` + `chmod`
4. fsync the tempfile
5. Atomic rename

The mode preservation step is **critical**: on Unix, a naive tempfile write defaults to `0o644`, which would silently weaken the security posture of files that were intentionally restrictive (e.g., `0o600`).

Lifted to `catalog::store::write_atomic` in Phase 4 F2; also used by `settings::edit::save_settings` (Phase 4 US3) for order-preserving settings rewrites.

## Test-Injection Patterns

### `#[doc(hidden)] pub static` with RAII Guard (Phase 3+)

When a test needs to inject a thread-local value, use the pattern:
```rust
#[doc(hidden)]
pub static INJECTION_SLOT: std::sync::RwLock<Option<T>> = std::sync::RwLock::new(None);
```

In the test file, define an RAII guard:
```rust
pub struct InjectionGuard;
impl InjectionGuard {
    pub fn install(value: T) {
        *INJECTION_SLOT.write().unwrap() = Some(value);
    }
}
impl Drop for InjectionGuard {
    fn drop(&mut self) {
        *INJECTION_SLOT.write().unwrap() = None;
    }
}
```

Used for:
- `MIGRATIONS_OVERRIDE` in `tests/schema_migration_e2e.rs`
- `HARNESS_MODULES_OVERRIDE` in `src/harness/mod.rs` with guard in test files
- `SUMMARISER_OVERRIDE` in `src/summarise/trigger.rs` (Phase 4 US4.b)

**Why not `#[cfg(test)]`?** Integration tests in `tests/` don't see `#[cfg(test)]` code; only `#[doc(hidden)] pub` is visible across crate boundaries. The `#[doc(hidden)]` attribute signals that the slot is internal.

### SummariserOverrideGuard Pattern (Phase 4 US4.b)

In test files that verify summariser triggering, use the guard to inject a test summariser:

```rust
use std::sync::Arc;
use tome::summarise::{Summariser, StubSummariser, SummariserOverrideGuard};

#[test]
fn summariser_fires_after_enable() {
    let stub = StubSummariser::new();
    let stub_arc: Arc<dyn Summariser> = Arc::new(stub.clone());
    let _guard = SummariserOverrideGuard::install(stub_arc);
    
    // Trigger production code path
    lifecycle::enable(&id, &deps).expect("enable");
    
    // Verify the stub was invoked
    assert_eq!(stub.call_count(), 1);
    // Guard drops here, clearing SUMMARISER_OVERRIDE
}
```

**Key properties**:
- Install the guard once per test
- Both guard and stub handle share the same underlying state (via `Arc<dyn Summariser>`)
- Call count persists across the fixture's lifetime
- Guard's `Drop` clears the slot automatically, even on panic

Mirrors `MigrationsGuard` from `tests/schema_migration_e2e.rs` and `HarnessModulesGuard` from harness tests — same shape, domain-specific type.

### HarnessModulesGuard Pattern (Phase 4 US3)

In test files that use synthetic harness modules:

```rust
pub struct HarnessModulesGuard;
impl HarnessModulesGuard {
    pub fn install(modules: Vec<Box<dyn HarnessModule>>) {
        *tome::harness::HARNESS_MODULES_OVERRIDE
            .write()
            .expect("HARNESS_MODULES_OVERRIDE poisoned") = Some(modules);
    }
}
impl Drop for HarnessModulesGuard {
    fn drop(&mut self) {
        *tome::harness::HARNESS_MODULES_OVERRIDE
            .write()
            .expect("HARNESS_MODULES_OVERRIDE poisoned") = None;
    }
}
```

Per-test-file scope ensures cleanup on panic. Store in `tests/common/mod.rs` for reuse across test files that mutate harness discovery.

### HomeGuard Pattern for Environment Mutation (Phase 4 US3)

When a test mutates `std::env::set_var("HOME", ...)`, use the `HOME_MUTEX` pattern to serialize with other tests:

```rust
static HOME_MUTEX: Mutex<()> = Mutex::new(());

#[test]
fn my_test() {
    let _lock = HOME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _home = HomeGuard::install(new_home_path);
    // ... test code that reads $HOME ...
}
```

The `HomeGuard` struct (from `tests/common/mod.rs`) restores the previous `HOME` value on drop. **Field declaration order is critical**: declare `_previous` (the restore guard) **before** `_lock` (the mutex guard) so `_previous` drops first (restoring HOME while still holding the mutex), then `_lock` releases, preventing race windows where another test reads a half-restored HOME.

```rust
pub struct HomeGuard {
    _previous: PrevHome,  // Drops FIRST, restores HOME
    _lock: std::sync::MutexGuard<'static, ()>,  // Drops SECOND, releases mutex
}
```

### Per-Test-File `OVERRIDE_MUTEX` (Phase 4 US3)

For tests that use `HARNESS_MODULES_OVERRIDE` or other thread-local injection points, declare a process-wide `Mutex` in the test file:

```rust
static OVERRIDE_MUTEX: Mutex<()> = Mutex::new(());

fn install_synthetic() -> (HarnessModulesGuard, MutexGuard<'static, ()>) {
    let lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let guard = HarnessModulesGuard::install(...);
    (guard, lock)  // Return both; hold lock for entire test
}
```

Return both guard and lock from the setup helper, and hold them for the entire test body. **Tuple order matters**: return `(guard, lock)` so guard drops before lock (standard RAII unwinding).

## Summariser & Inference Patterns (Phase 4 US4)

### Single Source of Truth for Length Constants

Hard upper bounds for summariser output are defined **only in `src/summarise/mod.rs`**:

```rust
pub const SHORT_MAX_CHARS: usize = 800;
pub const LONG_MAX_CHARS: usize = 2500;
```

All consumers import and use these constants directly:
- `src/summarise/prompts.rs` — re-exports and asserts bounds
- `src/summarise/llama.rs` — uses for inference loop breaks
- `src/workspace/regen_summary.rs` — uses for warn predicates

**Why one place?** Before consolidation (US4.d-1), the constants were duplicated in multiple files with divergent values (`LONG_MAX_CHARS = 2400` vs 2500), causing warn predicates to fire at different boundaries. A single edit now moves all consumers. Internal advisory windows (`SHORT_TARGET_*`, `LONG_TARGET_*`) remain private to `prompts.rs`.

### Model Cache at Constructor Time

The `LlamaSummariser` struct caches the loaded GGUF model and context environment:

```rust
pub struct LlamaSummariser {
    model: LlamaModel,
    // ... context fields ...
}

impl LlamaSummariser {
    pub fn new() -> Result<Self, TomeError> {
        // Expensive work happens ONCE here:
        // 1. Verify SHA-256 against registry
        // 2. Load the GGUF into ONNX Runtime
        // 3. Deserialize into LlamaModel
        let model = load_and_verify_model()?;
        Ok(Self { model, ... })
    }

    pub fn summarise(&self, input: &PluginSummariesInput) -> Result<SummariserOutput, TomeError> {
        // Fast path: create fresh context per invocation, reuse cached model
        let context = self.model.create_context()?;
        // ... inference loop ...
        Ok(SummariserOutput { short, long })
    }
}
```

**Pattern**: Expensive immutable resource setup (model files, large allocations) happens in the constructor; per-invocation work (context creation, forward passes) is cheap. Generalizable to any singleton service (embedder, reranker).

### OnceLock + Mutex Poison Recovery (Phase 4 US4.d-1, R-M7)

The process-wide `LlamaBackend` singleton uses double-checked locking with poison recovery:

```rust
static BACKEND: OnceLock<LlamaBackend> = OnceLock::new();
static INIT_LOCK: Mutex<()> = Mutex::new(());
static INIT_RESULT: OnceLock<Result<(), String>> = OnceLock::new();

pub fn backend() -> Result<&'static LlamaBackend, TomeError> {
    if let Some(backend) = BACKEND.get() {
        return Ok(backend);  // Fast path after first init
    }
    
    // Slow path: acquire mutex, but recover from poison
    let _guard = INIT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    
    // Re-check after acquiring lock
    if let Some(backend) = BACKEND.get() {
        return Ok(backend);
    }
    
    // Attempt init; cache result
    match LlamaBackend::init() {
        Ok(backend) => {
            let _ = INIT_RESULT.set(Ok(()));
            BACKEND.set(backend)
        }
        Err(e) => {
            let msg = e.to_string();
            let _ = INIT_RESULT.set(Err(msg.clone()));
            Err(TomeError::SummariserFailure { /* ... */ })
        }
    }
}
```

**Key discipline**: Use `unwrap_or_else(PoisonError::into_inner)` instead of `?`. A panicking allocator inside the lock guard poisons the mutex; the next caller recovers and attempts init again (the cached `INIT_RESULT` discriminates between clean failure and poisoning). This keeps the process alive for the OS lifetime, not permanently disabled by a transient panic.

### Silent Model Missing in Trigger Paths (Phase 4 US4, M2)

When the summariser model is absent, trigger paths (`plugin enable`, `disable`, `reindex`, `catalog update`) silently return `Ok(())` and skip the regeneration step:

```rust
pub fn regenerate_for_trigger(workspace_name: &WorkspaceName, paths: &Paths) -> Result<(), TomeError> {
    let summariser = match LlamaSummariser::new() {
        Ok(s) => s,
        Err(TomeError::SummariserFailure { kind: SummariserFailureKind::ModelMissing, .. }) => {
            // Silent no-op: model not yet downloaded via `tome models download`
            return Ok(());
        }
        Err(e) => return Err(e),  // Other failures bubble
    };
    // ... regenerate with summariser ...
}
```

Contrast with the **explicit `tome workspace regen-summary` command**, which hard-fails with exit 24 if the model is missing — users explicitly invoking summary regeneration expect the feature to be available.

**Rationale**: Triggers are implicit (side effects of enable/disable); implicit triggers shouldn't break unrelated user actions. Explicit invocations that name the feature must enforce its availability.

### Defence-in-Depth: Registry Placeholder Prevention

The summariser registry entry for Qwen is guarded against the all-zero SHA placeholder at both runtime and test time:

```rust
// src/summarise/llama.rs
pub fn new() -> Result<Self, TomeError> {
    let entry = MODEL_REGISTRY.iter()
        .find(|e| e.name == "qwen2.5-0.5b-instruct")
        .expect("registry missing qwen entry");
    
    if entry.sha256 == "0000000000000000000000000000000000000000000000000000000000000000" {
        return Err(TomeError::SummariserFailure { kind: SummariserFailureKind::ModelMissing, /* ... */ });
    }
    // ... proceed ...
}
```

Regression test (`tests/summariser_registry_no_placeholder.rs`):
```rust
#[test]
fn registry_qwen_sha256_is_not_placeholder() {
    let entry = MODEL_REGISTRY.iter()
        .find(|e| e.name == "qwen2.5-0.5b-instruct")
        .expect("qwen entry");
    
    assert_ne!(entry.sha256, "0000000000000000000000000000000000000000000000000000000000000000",
        "registry placeholder must be replaced with real hash before Phase 4 US4.a ships");
}
```

**Pattern**: When a hard-coded value must never be X (e.g., a placeholder must be replaced before shipping), use a defensive runtime check PLUS an automated regression test. The test will fail if the placeholder sneaks back into a future update.

## Dependency Boundaries

### Crate Feature Flags

- `serde_json/preserve_order` — globally enabled for order-preserving JSON serialization
- `toml_edit` — used only in `src/harness/mcp_config.rs` for TOML comment/order preservation
- Phase 3: `tokio` and `rmcp` scoped to `src/mcp/` only; enforced by `tests/sync_boundary.rs`
- Phase 4: `llama-cpp-2` scoped to `src/summarise/llama.rs` (and Phase 4 US4 tests); exact-pinned at `=0.1.146` per research §R-2 (upstream breaking C ABI on every minor)

### Not Used

- `tokio` outside `src/mcp/` (sync codebase discipline)
- `libgit2` / `git2` (shell out to system `git`)
- `directories` crate (Phase 4 F2 replacement: `std::env`)
- `atty`, `colored`, `lazy_static`, `once_cell` (Rust std covers or not needed)

## Common Patterns

### Silent Compute + Emit Wrapper (Phase 3+)

When a command's logic is reused by non-CLI surfaces (MCP, library API), split into:

1. **`assemble_*(args, deps) -> Result<Outcome, Error>`** — pure compute, no I/O side-effects (renamed from `pipeline` for CLI clarity)
2. **`run(args, deps, mode) -> Result<(), Error>`** — calls `assemble_*`, then emits per `mode` and exits

Tests target the `assemble_*` function and assert on returned outcomes; the CLI dispatcher uses `run` which handles emission. This pattern is now pinned for **every CLI subcommand** (US3.d-1 T-B2 compliance).

Example from `harness::info::run`:
```rust
pub fn run(args: HarnessInfoArgs, scope: &ResolvedScope, paths: &Paths, mode: Mode) -> Result<(), TomeError> {
    let outcome = assemble(args, scope, paths)?;
    output::write_json(mode, &outcome)?;
    Ok(())
}
```

Applied to `workspace::info::run`, `workspace::list::run`, `harness::list::run`, and all CLI commands added in Phase 4 US3+.

### Helper Visibility Promotion (Phase 3+)

When two surfaces (e.g., `tome status` + `tome doctor`) must report the same value, promote the shared helper from `private` to `pub` rather than duplicating compute:

```rust
pub fn check_model(paths, embedder, verify) -> ModelHealth { /* ... */ }
// Called by both status and doctor
```

### Content-Addressed Shared Resources with Reference Counting (Phase 3+)

When an on-disk resource is shared across scopes (e.g., catalog clone cache), build a reference-count lookup by enumerating every scope that could reference the URL:

```rust
let refs = catalog::store::reference_count(url, paths) -> Vec<Scope>;
if refs.is_empty() {
    fs::remove_dir_all(&cache_dir)?;
}
```

Prevents dangling references when scopes are deleted. TOCTOU is unlocked (benign race with other removes).

### Reuse Existing Closed-Set Variants (Phase 3+)

When a new failure mode maps semantically to an existing `TomeError` variant + exit code, prefer reuse over a dedicated variant. Trade-off: slightly off-message Display (e.g., `CatalogAlreadyExists("workspace at /path/.tome")`). Benefit: zero enum churn, stable exit codes.

Example: "workspace already initialised" reuses `CatalogAlreadyExists` (code 4) per the contract's explicit permission.

### Optional-Registry Append (Phase 3+)

For opt-in tracking surfaces, write a helper like `inventory::append_if_registry_exists(path, item)` that no-ops when the registry file is absent:

```rust
pub fn append_if_registry_exists(path: &Path, item: &str) -> Result<(), TomeError> {
    if !path.exists() {
        return Ok(());  // Registry not yet created; skip
    }
    // Append with dedup by exact-string match
}
```

User touches the file once to opt in; subsequent operations append.

### Workspace Name as Opaque Value (Phase 4)

When threading workspace identity, pass `&Scope` (which wraps `WorkspaceName`) rather than loose `&str`:

```rust
pub fn resolve_plugin_dir(id: &PluginId, scope: &Scope, config: &Config) -> Result<PathBuf, TomeError>
```

Provides compile-time safety: you can't accidentally pass `"global"` as a string and expect it to be validated.

## Git Conventions

### Commit Messages

Format: `type(scope): subject`

| Type | Usage |
|------|-------|
| feat | New feature |
| fix | Bug fix |
| docs | Documentation |
| style | Formatting (not code style violations) |
| refactor | Code restructure |
| test | Adding tests |
| chore | Maintenance |

Enforced by `cocogitto` in `.githooks/commit-msg`. Use `git commit --no-verify` only with explicit justification in the message body.

### Branch Naming

Trunk-based; short-lived branches off `main`. No formal requirement, but convention is descriptive.

### PR Size

Soft cap of ~400 lines or 2 modules per PR to keep reviews focused.

---

## What Does NOT Belong Here

- Test strategies → TESTING.md
- Security practices → SECURITY.md
- Architecture decisions → ARCHITECTURE.md
- Technology choices → STACK.md

---

*This document defines HOW to write code. Update when conventions change. Last refreshed 2026-05-26 against Phase 4 / US4-complete source (862 passing tests, 16 ignored, 117 suites).*
