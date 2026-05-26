# Coding Conventions

> **Purpose**: Document code style, naming conventions, error handling, and common patterns.
> **Generated**: 2026-05-26
> **Last Updated**: 2026-05-26 (Phase 4 / US5 complete)

## Code Style

### Formatting Tools

| Tool | Configuration | Command |
|------|---------------|---------|
| rustfmt | `rustfmt.toml` (edition = "2024") | `cargo fmt --check` |
| Clippy | Enforced via hook; strict `-D warnings` | `cargo clippy --all-targets --all-features -- -D warnings` |
| typos | No config file; tree-wide | `typos` |

All three gates run locally in `.githooks/pre-commit` before every commit; CI re-runs on every PR. Bypass with `git commit --no-verify` only with explicit justification in the message body.

### Style Rules

| Rule | Convention |
|------|------------|
| Indentation | 4 spaces (Rust default) |
| Edition | Rust 2024 |
| MSRV | 1.93 (locked in `Cargo.toml` `rust-version`; verified in CI via dtolnay/rust-toolchain) |
| Line length | No hard limit; rustfmt defaults |
| Trailing commas | Automatic via rustfmt |

**Key invariant**: Zero compiler warnings under Clippy `-D warnings`. If a pattern would generate a warning, it doesn't ship.

## Naming Conventions

### Files & Directories

| Type | Convention | Example |
|------|------------|---------|
| Modules | snake_case, capability-organized | `src/doctor/`, `src/summarise/`, `src/harness/` |
| Test files | Integration test prefix + `_` | `tests/doctor_p4.rs`, `tests/workspace_use_atomicity.rs` |
| Fixtures | Lowercase, descriptive | `tests/fixtures/sample-catalog/` |
| Config files | TOML for Tome-owned, inherit third-party names | `config.toml`, `settings.toml`, `.mcp.json` (upstream format) |
| Temporary directories | `.tome.tmp.*` prefix | Used by `atomic_dir` helper for crash safety |

### Code Elements

| Type | Convention | Example |
|------|------------|---------|
| Variables | snake_case, descriptive | `config_dir`, `embedder_seed`, `workspace_name`, `project_root`, `subsystem` |
| Constants | SCREAMING_SNAKE_CASE | `GRACEFUL_SHUTDOWN_TIMEOUT`, `MIGRATIONS`, `SHORT_MAX_CHARS`, `SCHEMA_VERSION` |
| Functions | snake_case, verb-forward | `apply_pending`, `open_read_only`, `land_directory`, `regenerate_for_trigger` |
| Structs | PascalCase | `TomeError`, `WorkspaceInfo`, `LifecycleDeps`, `BindDeps`, `HarnessModule` |
| Enums | PascalCase; variants PascalCase | `Scope`, `Subsystem { Embedder, Catalog(...) }`, `RulesFileStrategy` |
| Traits | PascalCase | `Embedder`, `Reranker`, `HarnessModule`, `Summariser` |
| Module doc comments | First line explains module role | `//! MCP server state and initialization` |
| Newtype wrappers | PascalCase + unit type | `WorkspaceName(String)`, `PluginId { catalog, plugin }` |

## Error Handling

### Closed Error Enum Pattern

Tome uses a **closed enumeration** for all errors: `TomeError` in `src/error.rs` has no `Other`/`Unknown` arm. Every failure class maps to exactly one variant and a unique exit code. Adding a variant **forces updates** to:

1. `tests/exit_codes.rs` — compiler exhaustiveness
2. `specs/*/contracts/exit-codes-*.md` — spec authority
3. PRD / release notes — external contract

**Design benefit**: Exit codes are stable and discoverable. **Trade-off**: New failure modes require deliberate variant addition (usually pre-allocated in Foundational phases).

### Error Variant Organization

Variants are grouped by **Phase** with exit code ranges as comments:

```rust
// Phase 1 (codes 2–8, plus Internal=1). Unchanged.
// Phase 2 — plugin lifecycle (codes 20–23).
// Phase 3 — MCP / workspace (codes 60–75).
// Phase 4 — workspace name + harness + summariser (codes 13–19, 24).
```

**Pre-allocated variants** are added in Foundational phases **before any consumer exists**. Phase 4 F3 pre-allocated codes 13–19, 24 before project binding and harness composition were implemented. Benefit: zero mid-feature enum churn; compiler enforces all arms are covered.

### Error Variants with Rich Context

**Single `#[from]` source** (when semantically equivalent):
```rust
#[error("io: {0}")]
Io(#[from] std::io::Error),
```

**Named fields for structured context**:
```rust
#[error("git failed for `{catalog}`: {detail}")]
GitFailed { catalog: String, detail: String },
```

**Nested enum for domain-specific variants**:
```rust
#[error("workspace `{name}` not found in the central registry")]
WorkspaceNotFound { name: String },
```

**Recovery hints in Display messages**:
```rust
#[error("model `{model}` is missing; run `tome models download` to fetch it")]
ModelMissing { model: String },
```

## Core Type Patterns (Phase 4)

### WorkspaceName Newtype with Validation

All workspace names flow through `WorkspaceName::parse(s: &str)` at **every input boundary**: CLI flags, TOML deserialization, environment variables, file markers. The type is a newtype `struct WorkspaceName(String)` with immutable access via `as_str()`.

**Validation rules** (FR-347):
- 1–64 chars from `[a-zA-Z0-9_-]`
- Must not begin or end with `-` or `_`
- Must not be `.`, `..`, or empty
- Reserved name `"global"` parses OK but `is_reserved()` flags it for lifecycle commands

**Deserialization automatically validates**:
```rust
impl Deserialize for WorkspaceName {
    fn deserialize(...) -> Result<Self, D::Error> {
        WorkspaceName::parse(s).map_err(...)
    }
}
```

### PluginId Identity Parsing

Plugin addresses `catalog/plugin` validate via `impl FromStr for PluginId`:
```rust
let id: PluginId = "my-catalog/my-plugin".parse()?;
```

Rejects:
- Embedded slashes in either segment
- Parent traversal (`..`) in any segment
- Dot-prefixes (`.` / `..` or hidden entries)
- Absolute paths

### Subsystem Enum with Byte-Stable Wire Shape (Phase 4 / US5)

When dispatch keys reach >6 distinct values, promote from `String` to typed enum:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Subsystem {
    Embedder,
    Reranker,
    Index,
    Drift,
    Catalog(String),           // wire: "catalog:<name>"
    Schema,
    Summariser,
    Binding,
    BindingRulesCopy,
    HarnessRules(String),      // wire: "harness-rules:<name>"
    HarnessMcp(String),        // wire: "harness-mcp:<name>"
}

impl Subsystem {
    pub fn to_wire_string(&self) -> String { /* ... */ }
    pub fn parse_wire(s: &str) -> Option<Self> { /* ... */ }
}

// Backward-compat: compare against &str
impl PartialEq<str> for Subsystem {
    fn eq(&self, other: &str) -> bool {
        self.to_wire_string() == other
    }
}
```

**Benefits**:
- Type safety (exhaustiveness checking at dispatch points)
- Backward-compatible wire format (Phase 3 tests still work)
- Colon-separated variants like `"catalog:name"` stored as `Variant(String)` internally

**Test pattern** — pin the byte-stable wire shape:
```rust
#[test]
fn every_variant_round_trips_via_documented_wire_string() {
    let cases = vec![
        (Subsystem::Embedder, "\"embedder\""),
        (Subsystem::Catalog("upstream".into()), "\"catalog:upstream\""),
        // ...
    ];
    for (variant, wire) in cases {
        let serialised = serde_json::to_string(&variant).unwrap();
        assert_eq!(serialised, wire);
        let parsed: Subsystem = serde_json::from_str(wire).unwrap();
        assert_eq!(parsed, variant);
    }
}
```

Used in: `src/doctor/report.rs::Subsystem` (11 variants covering embedder, reranker, index, drift, catalogs, schema, summariser, binding, rules/MCP per harness).

### SubsystemHealth::NotApplicable (Phase 4 / US5)

When a per-row health check legitimately doesn't apply in a context (e.g., harness subsystems when no project is bound), emit `NotApplicable` rather than omitting the row:

```rust
pub enum SubsystemHealth {
    Ok,
    Drift,
    Broken,
    UserOwned,
    NotApplicable,  // "operation doesn't apply here"
}
```

Wire distinguishes "inapplicable" from "not checked" so callers can tailor UI. Used in: `src/doctor/report.rs` — harness-health rows emit `NotApplicable` when doctor runs outside a workspace project.

## File Operations & Atomicity

### Atomic-Directory Landing Pattern

Phase 4 promotes the atomic-directory landing pattern from `workspace::init` into a reusable helper under `src/util/atomic_dir.rs`. Used for workspace markers, harness commands, and any multi-file state that must appear complete or not-at-all to concurrent readers.

**Key invariants**:

1. **Same-filesystem staging**: `tempfile::Builder::new().prefix(".tome.tmp.").tempdir_in(parent)` creates a sibling staging directory on the same filesystem, guaranteeing POSIX-atomic rename.

2. **Mode preservation (Unix)**: If the target exists, capture its file mode. Apply to the staging tempfile, then preserve through the rename. If target is absent, libc default (typically `0o600`) wins.

3. **Crash safety**:
   - Crash before `TempDir::keep()` → staging auto-cleaned
   - Crash after `keep()` but before final rename → orphan staging picked up by `doctor --fix` (matches `.tome.tmp.*` prefix)
   - Replace variant: on final rename failure, rollback the `.old` sibling before bubbling error

4. **Fsync before keep**: Call `nix::fcntl::fsync()` on the staging directory (or best-effort no-op on platforms that don't support dir fsync).

**Public API**:
```rust
pub fn land_directory<F>(target: &Path, mode_unix: u32, populate: F) 
    -> Result<PathBuf, TomeError>
where F: FnOnce(&Path) -> Result<(), TomeError>

pub fn land_directory_with_replace<F>(
    target: &Path, mode_unix: u32, populate: F
) -> Result<PathBuf, TomeError>
where F: FnOnce(&Path) -> Result<(), TomeError>
```

Used in: `src/workspace/binding.rs` (project marker creation), harness sync paths.

### Write-Through Mode Preservation

Every file write (via `catalog::store::write_atomic`, `settings::edit::save_settings`) follows:
1. Read existing file and capture mode (Unix) if it exists
2. Write to sibling tempfile in same directory
3. Apply captured mode via `symlink_metadata` + `chmod`
4. fsync the tempfile
5. Atomic rename

**Critical**: On Unix, naive tempfile writes default to `0o644`, silently weakening files that were intentionally restrictive (e.g., `0o600`). Mode preservation preserves security posture.

## Sync & Concurrency Patterns

### Two-Phase Sync Orchestrator (Phase 4)

Workspace binding and harness sync use a two-phase pattern:

- **Phase A** (brief, under advisory lock): Read from central DB to identify workspace and binding.
- **Phase B** (unlocked): Perform filesystem operations (rules file mods, MCP config edits) without holding the lock. On failure, error bubbles; subsequent runs detect and fix.

This allows writers to proceed while filesystem I/O is in flight, improving responsiveness when large operations (e.g., git checks on huge repos) would otherwise block the index.

### Idempotence-by-Mtime (Phase 4)

Rules file and MCP config primitives (`rules_file::read`, `mcp_config::write`, etc.) short-circuit when bytes match:

```rust
if existing_bytes == new_bytes {
    return Ok(());  // No mtime change, no fsync, no atomic write
}
```

Tests verify by capturing mtime before write, sleeping 1.5 seconds (filesystem granularity), re-reading, and asserting mtime unchanged.

### Advisory Lock Around Settings Mutations (Phase 4 US3)

Any command mutating Tome-owned config files outside the central DB (e.g., `settings.toml`, `config.toml`) must acquire the index advisory lock for the full read-modify-write window:

```rust
let _lock = index::lock::LockFile::acquire(&paths.index_lock_path)?;
// Read, modify, write — operations serialize with index writers
```

Prevents TOCTOU races on the same file.

## Diagnostic & Repair Patterns

### Per-Field State Mutations + Re-assemble (Phase 4 / US5)

When a repair function mutates state in place per subsystem, expose a sibling `re_assemble()` that recomputes derived state without re-running expensive checks:

```rust
pub fn apply(&mut report: DoctorReport, paths, scope) -> usize {
    // For each suggested fix: apply repair, update per-field state in-place
}

pub fn re_assemble(report: &mut DoctorReport) {
    // Recompute suggested_fixes + overall from per-field state (no re-probes)
}
```

Saves half the filesystem cost on catalog-enumeration + harness-probe sides after applying fixes.

Used in: `src/doctor/fixes.rs` — repairs run per-subsystem (embedder, reranker, catalogs, schema, harnesses), each updates targeted fields. After all repairs, `re_assemble()` recomputes the derived summary.

### Coalesce Repair Invocations (Phase 4 / US5)

When multiple suggested fixes share a repair function (e.g., 10 harness rules files all from the same source), collect all matching suggestions, run the repair **once**, then clear all affected from the residual list:

```rust
// Group all "binding-rules-copy" fixes by source
let all_binding_fixes: Vec<_> = report.suggested_fixes
    .iter()
    .filter(|f| f.subsystem == Subsystem::BindingRulesCopy)
    .collect();

// Run repair once
if let Some(first) = all_binding_fixes.first() {
    repair_binding_rules_copy(&first.details)?;
}

// Remove all in one shot
report.suggested_fixes
    .retain(|f| f.subsystem != Subsystem::BindingRulesCopy);
```

Avoids 10× redundant operations when 10 harnesses reference the same source.

### Project-Local vs Workspace-Broadcast Helpers (Phase 4 / US5)

When a sync/repair function has both single-project and all-projects semantics, expose as **distinct named functions**:

```rust
// Single project
pub fn sync_one_project(project_root: &Path, source: &Path, paths: &Paths) 
    -> Result<(), TomeError>

// All projects in workspace
pub fn sync_all(workspace_name: &WorkspaceName, paths: &Paths) 
    -> Result<usize, TomeError>
```

Don't pass an `Option` parameter; the caller knows which it wants.

### SourceMissing vs Missing Health States (Phase 4 / US5)

When a copy-from-source operation can fail for two reasons, distinguish them in state:

```rust
pub enum Health {
    Ok,
    Missing,           // Destination absent or corrupted
    SourceMissing,     // Source file gone; manual regeneration needed
}
```

`--fix` for `Missing` copies from source. `--fix` for `SourceMissing` surfaces a manual-action `SuggestedFix`.

### Read-Only by Default, Fix-on-Explicit-Flag (Phase 4 / FR-563)

Diagnostic commands separate report from repair:

- **Report path** (no flags): Read-only, never mutates state, safe for scripts
- **Repair path** (`--fix`): Explicitly flagged, acquires advisory lock, changes disk state

Invariant: a read-only pass must NOT mutate mtimes, DB rows, or filesystem state. Tests verify by checking mtime before and after a read-only invocation.

Used in: `tome doctor [--verify]` (read-only) vs `tome doctor --fix` (repairs only when flag present).

### --force Precise Scoping (Phase 4 / US5)

The `--force` flag should rewrite **only the conflicting fixes in the current run's list**, not bulk-rewrite every user-owned entity globally:

```rust
pub fn apply(&mut report: DoctorReport, force: bool) -> usize {
    for fix in report.suggested_fixes.iter() {
        if auto_fixable && (force || !user_owned) {
            // Apply only if auto_fixable AND (force OR not user-owned)
        }
    }
}
```

Prevents accidental data loss when a user-owned file clashes with a Tome-owned one.

### Debug Assertions on Safe-Root Invariants (Phase 4 / US5)

For operations like `remove_dir_all()` on derived paths, assert safety invariants:

```rust
pub fn cleanup_orphan(root: &Path) -> std::io::Result<()> {
    debug_assert!(root.starts_with(&safe_parent), 
        "orphan root {root:?} escaped sandbox {safe_parent:?}");
    std::fs::remove_dir_all(root)?;
    Ok(())
}
```

Documents the invariant + catches future refactors that break it.

Used in: `src/doctor/orphan_cleanup.rs` (ensures orphan cleanup never escapes the expected root).

## Common Command Patterns

### Silent Compute + Emit Wrapper (Phase 3+)

When a command's logic must be reused by non-CLI surfaces (MCP tools, library APIs), split into two functions:

```rust
// Silent compute — no I/O side-effects, unit-testable
pub fn assemble(args: Args, deps: &Deps) -> Result<Outcome, TomeError> {
    // ... business logic ...
    Ok(outcome)
}

// CLI emit wrapper — calls assemble, then formats per mode and exits
pub fn run(args: Args, deps: &Deps) -> Result<(), TomeError> {
    let outcome = assemble(args, deps)?;
    output::emit(&outcome);  // formats as human/JSON
    Ok(())
}
```

Tests use `assemble` and assert on outcomes. CLI uses `run` which handles emission. MCP tools use `assemble` without the emit step.

Applied to: Every Phase 4 subcommand (`workspace::info::run`, `harness::list::run`, etc.).

### Helper Visibility Promotion (Phase 3+)

When two surfaces (e.g., `tome status` + `tome doctor`) must report the same value, promote the shared helper from `private` to `pub` rather than duplicating:

```rust
pub fn check_model(paths, embedder, verify) -> ModelHealth { /* ... */ }
// Called by both status and doctor
```

Cost: three-character edit. Benefit: zero divergence risk.

## Test-Injection & Mock Patterns

### `#[doc(hidden)] pub static` with RAII Guard (Phase 3+)

For integration tests that need thread-local injection (tests don't see `#[cfg(test)]`), use:

```rust
// In src/module/mod.rs
#[doc(hidden)]
pub static INJECTION_SLOT: std::sync::RwLock<Option<T>> = std::sync::RwLock::new(None);
```

In the test file, define an RAII guard:

```rust
pub struct InjectionGuard;
impl InjectionGuard {
    pub fn install(value: T) { /* set slot */ }
}
impl Drop for InjectionGuard {
    fn drop(&mut self) { /* clear slot */ }
}
```

Used for: `MIGRATIONS_OVERRIDE`, `HARNESS_MODULES_OVERRIDE`, `SUMMARISER_OVERRIDE`.

**Why not `#[cfg(test)]`?** Integration tests don't see `#[cfg(test)]` code; only public items are visible. `#[doc(hidden)]` signals that the slot is internal.

### HarnessModulesGuard Pattern (Phase 4 US3)

For tests using synthetic harness modules:

```rust
pub struct HarnessModulesGuard;
impl HarnessModulesGuard {
    pub fn install(modules: Vec<Box<dyn HarnessModule>>) {
        *tome::harness::HARNESS_MODULES_OVERRIDE
            .write()
            .expect("poisoned") = Some(modules);
    }
}
impl Drop for HarnessModulesGuard {
    fn drop(&mut self) {
        *tome::harness::HARNESS_MODULES_OVERRIDE
            .write()
            .expect("poisoned") = None;
    }
}
```

Per-test-file scope ensures cleanup on panic. Store in `tests/common/mod.rs`.

### SummariserOverrideGuard Pattern (Phase 4 US4.b)

In test files verifying summariser triggering:

```rust
use tome::summarise::{Summariser, StubSummariser, SummariserOverrideGuard};

#[test]
fn summariser_fires_after_enable() {
    let stub = StubSummariser::new();
    let _guard = SummariserOverrideGuard::install(Arc::new(stub.clone()));
    
    lifecycle::enable(&id, &deps).expect("enable");
    assert_eq!(stub.call_count(), 1);
    // Guard drops, clearing SUMMARISER_OVERRIDE
}
```

### Per-Test Mutex for Concurrent Test Isolation (Phase 4 US3)

For tests using injection points, declare a process-wide mutex in the test file:

```rust
static OVERRIDE_MUTEX: Mutex<()> = Mutex::new(());

fn install_synthetic() -> (HarnessModulesGuard, MutexGuard<'static, ()>) {
    let lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let guard = HarnessModulesGuard::install(...);
    (guard, lock)  // Return both; hold lock for entire test
}
```

Return both guard and lock; hold them for the entire test body. **Order matters**: `(guard, lock)` so guard drops before lock (standard RAII unwinding).

## Summariser & Inference Patterns (Phase 4 US4)

### Single Source of Truth for Length Constants

Hard upper bounds for summariser output are defined **only in `src/summarise/mod.rs`**:

```rust
pub const SHORT_MAX_CHARS: usize = 800;
pub const LONG_MAX_CHARS: usize = 2500;
```

All consumers import and use directly:
- `src/summarise/prompts.rs` — re-exports and asserts bounds
- `src/summarise/llama.rs` — uses for inference loop breaks
- `src/workspace/regen_summary.rs` — uses for warn predicates

Before consolidation (US4.d-1), duplicated constants with divergent values caused warn predicates to fire at different boundaries.

### Model Cache at Constructor Time

The `LlamaSummariser` caches the loaded GGUF model and context environment:

```rust
pub struct LlamaSummariser {
    model: LlamaModel,
    // ... context fields ...
}

impl LlamaSummariser {
    pub fn new() -> Result<Self, TomeError> {
        // Expensive work happens ONCE:
        // 1. Verify SHA-256 against registry
        // 2. Load GGUF into ONNX Runtime
        // 3. Deserialize into LlamaModel
        let model = load_and_verify_model()?;
        Ok(Self { model, ... })
    }

    pub fn summarise(&self, input: &PluginSummariesInput) -> Result<SummariserOutput, TomeError> {
        // Fast path: create fresh context, reuse cached model
        let context = self.model.create_context()?;
        // ... inference loop ...
        Ok(SummariserOutput { short, long })
    }
}
```

**Pattern**: Immutable resource setup (model files, allocations) in constructor. Per-invocation work (context, forward passes) is cheap.

### Silent Model Missing in Trigger Paths (Phase 4 / US4)

When summariser model is absent, trigger paths (`plugin enable`, `disable`, `reindex`, `catalog update`) silently return `Ok(())`:

```rust
pub fn regenerate_for_trigger(workspace_name: &WorkspaceName, paths: &Paths) -> Result<(), TomeError> {
    let summariser = match LlamaSummariser::new() {
        Ok(s) => s,
        Err(TomeError::SummariserFailure { 
            kind: SummariserFailureKind::ModelMissing, .. 
        }) => {
            // Silent no-op: model not yet downloaded
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    // ... regenerate ...
}
```

Contrast with **explicit `tome workspace regen-summary`**, which hard-fails (exit 24) if model is missing.

**Rationale**: Triggers are implicit side-effects; shouldn't break unrelated actions. Explicit invocations must enforce feature availability.

### Defence-in-Depth: Registry Placeholder Prevention

Summariser registry entry guards against the all-zero SHA placeholder at both runtime and test time:

```rust
pub fn new() -> Result<Self, TomeError> {
    let entry = MODEL_REGISTRY.iter()
        .find(|e| e.name == "qwen2.5-0.5b-instruct")
        .expect("registry missing qwen entry");
    
    if entry.sha256 == "0000000000000000000000000000000000000000000000000000000000000000" {
        return Err(TomeError::SummariserFailure { /* ... */ });
    }
    // ... proceed ...
}
```

Regression test:
```rust
#[test]
fn registry_qwen_sha256_is_not_placeholder() {
    let entry = MODEL_REGISTRY.iter()
        .find(|e| e.name == "qwen2.5-0.5b-instruct")
        .expect("qwen entry");
    
    assert_ne!(entry.sha256, "0000...", "must be real hash before ship");
}
```

**Pattern**: When a hard-coded value must never be X, use defensive runtime check PLUS automated regression test.

## Git Conventions

### Commit Messages

Format: `type(scope): subject`

| Type | Usage | Example |
|------|-------|---------|
| feat | New feature | `feat(doctor): --fix handlers for Phase 4` |
| fix | Bug fix | `fix(doctor): US5 reviewer-flagged fixups` |
| docs | Documentation | `docs(codebase): refresh after US5` |
| test | Test additions | `test(doctor_p4): per-subsystem coverage` |
| refactor | Code restructure | (minimize; only when necessary) |
| chore | Maintenance | Dependency updates, tooling |

Enforced by `cocogitto` in `.githooks/commit-msg`. Use `git commit --no-verify` only with explicit justification in message body.

### Branching

- **Trunk-based**: Short-lived feature branches off `main`, deleted after merge.
- **PR size**: ~400 lines or 2 modules as soft cap. Keeps review tractable.

## Dependency Boundaries

### Crate Feature Flags

- `serde_json/preserve_order` — globally enabled for order-preserving JSON
- `toml_edit` — scoped to `src/harness/mcp_config.rs` only
- Phase 3: `tokio`, `rmcp` scoped to `src/mcp/` only; enforced by `tests/sync_boundary.rs`
- Phase 4: `llama-cpp-2` scoped to `src/summarise/llama.rs`; exact-pinned at `=0.1.146` (upstream breaks C ABI on every minor)

### Not Used

- `tokio` outside `src/mcp/` (sync codebase discipline)
- `libgit2` / `git2` (shell out to system `git`)
- `directories` crate (replaced by `std::env` in Phase 4 F2)
- `atty`, `colored`, `lazy_static`, `once_cell` (not needed)

---

## What Does NOT Belong Here

- Test strategies → `TESTING.md`
- Security practices → `SECURITY.md`
- Architecture decisions → `ARCHITECTURE.md`
- Technology choices → `STACK.md`

---

*This document defines HOW to write code. Update when conventions change or new patterns stabilize.*
*Last refreshed 2026-05-26 against Phase 4 / US5-complete source (916 tests passing, 125 suites).*
