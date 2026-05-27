# Coding Conventions

> **Purpose**: Document code style, naming conventions, error handling, and patterns for Tome (Rust CLI).
> **Generated**: 2026-05-27
> **Last Updated**: 2026-05-27

## Code Style

### Formatting Tools

| Tool | Configuration | Command |
|------|---------------|---------|
| rustfmt | `rustfmt.toml` | `cargo fmt` |
| clippy | `clippy.toml` | `cargo clippy --all-targets --all-features -- -D warnings` |
| typos | `_typos.toml` | `typos` |

All three enforce at the pre-commit hook (`.githooks/pre-commit`) before commits land. No linting violations are permitted (`-D warnings`).

### Style Rules

| Rule | Convention |
|------|------------|
| Edition | Rust 2024 (in `Cargo.toml`) |
| MSRV | 1.93 (enforced in CI) |
| Indentation | 4 spaces (rustfmt default) |
| Semicolons | Required (rustfmt enforced) |
| Max line length | 100 chars (soft; clippy tunable) |
| Comments | Explain *why*, not *what* (readers know Rust) |

**Strictness boundary** (FR-013a): `#[serde(deny_unknown_fields)]` applies to Tome-owned inputs (`config.toml`, model manifests, index `meta` rows). Third-party inputs (SKILL.md YAML frontmatter, `plugin.json` metadata) parse leniently — enforced by `tests/manifest_strictness.rs` grep guard.

## Naming Conventions

### Files & Directories

| Type | Convention | Example |
|------|------------|---------|
| Modules | snake_case, descriptive | `src/workspace/binding.rs`, `src/plugin/lifecycle.rs` |
| Submodules | snake_case, noun-verb paired | `src/commands/plugin/{mod,enable,disable,list,show}.rs` |
| Test files | snake_case + `_*.rs` suffix | `tests/catalog_add.rs`, `tests/plugin_enable.rs` |
| Test fixtures | PascalCase directories | `tests/fixtures/sample-catalog/` |

### Code Elements

| Type | Convention | Example |
|------|------------|---------|
| Variables | snake_case | `catalog_name`, `plugin_id`, `is_enabled` |
| Constants | SCREAMING_SNAKE_CASE | `MAX_RETRIES`, `LONG_MAX_CHARS` |
| Functions | snake_case, verb-prefix | `enable_plugin`, `resolve_plugin_dir`, `assemble_report` |
| Structs | PascalCase | `TomeError`, `Config`, `Fixture` |
| Enums | PascalCase, variants as items | `ScopeKind`, `CompositionErrorKind` |
| Traits | PascalCase | `Embedder`, `Reranker`, `HarnessModule` |
| Type aliases | PascalCase or semantic | `ResolvedScope` |

## Error Handling

### Error Pattern: Closed Enum with Exit Codes

Tome uses a **closed `TomeError` enum** — every error variant has an enumerated exit code. The compiler enforces the chain: adding a variant forces edits to `tests/exit_codes.rs`, the spec, and contracts.

**File**: `src/error.rs`

**Exit codes by phase**:
- Phase 1: codes 2–8, plus Internal=1
- Phase 2: codes 20–52 (plugins, index, embedding)
- Phase 3: codes 60–75 (MCP, doctor, schema migration)
- Phase 4: codes 13–20 (workspace, harness, composition)
- Phase 5: codes 21–26 (commands, prompts, substitution)

**Contract reference**: `specs/00X-phase-Y-*/contracts/exit-codes-p*.md` per phase.

### Error Propagation

- Library functions return `Result<T, TomeError>` (errors are specific and closed).
- CLI commands call `TomeError::exit_code()` to map errors to integers.
- Application-level context chains use `anyhow::Context` where needed.
- Credential scrubbing at boundaries via `src/catalog/git.rs::scrub_credentials` before logging.

### Logging Conventions

| Level | Usage | Example |
|-------|-------|---------|
| `error!` | Unrecoverable failures, MCP preflight failures | `error!("hard shutdown: {}")` |
| `warn!` | Recoverable issues, state drift | `warn!("length window exceeded")` |
| `info!` | Important transitions, user-facing events | `info!("plugin enabled: {}")` |
| `debug!` | Internal state, algorithm details | `debug!("cost function: {}")` |

**Tool**: `tracing` + `tracing-subscriber` with `env-filter` feature. Configured via `RUST_LOG` env var.

## Common Patterns

### Atomic Writes (Multi-File Invariant)

When a directory of files must appear complete or not at all, use `util::atomic_dir::land_directory_with_replace`:

```rust
// 1. Create sibling staging dir on same filesystem
let staging = TempDir::new_in(parent)?;

// 2. Populate staging
write_file_1(&staging)?;
write_file_2(&staging)?;

// 3. Move existing aside (optional)
if target.exists() {
    fs::rename(&target, &target.with_extension("old"))?;
}

// 4. POSIX-atomic rename
land_directory_with_replace(staging, target)?;
```

**Used in**: `src/workspace/init.rs`, `src/util/atomic_dir.rs`.

### Silent Compute + Emit Wrapper

When a command's logic is reused by non-CLI surfaces (MCP, library API, tests), split into:

```rust
// Library entry: pure computation
pub fn assemble_report(args, deps) -> Result<Outcome, Error> {
    // compute
}

// CLI wrapper: calls library, then emits
pub fn run(args, deps, mode: Mode) -> Result<(), Error> {
    let outcome = assemble_report(args, deps)?;
    output::write_json_or_human(&outcome, mode);
    Ok(())
}
```

**Pattern used in**: `commands/status/mod.rs`, `commands/plugin/{enable,list,show}.rs`, `commands/reindex.rs`, `commands/doctor.rs`.

### Test Injection via `#[doc(hidden)] pub static`

For integration tests (under `tests/`, which have no `cfg(test)` visibility), inject state via process-local slots:

```rust
// src/plugin/lifecycle.rs
#[doc(hidden)]
pub static EMBEDDER_OVERRIDE: OnceLock<Option<Arc<dyn Embedder>>> = OnceLock::new();

// tests/plugin_enable.rs
let _guard = embedding::StubEmbedderGuard::with_force_fail_after(2);
```

**Patterns**: `HARNESS_MODULES_OVERRIDE`, `MIGRATIONS_OVERRIDE`, `SUBSTITUTION_CLOCK_OVERRIDE`, `PLUGIN_DATA_DIR_OVERRIDE`, `WORKSPACE_DATA_DIR_OVERRIDE`. All guarded by `tests/common/mod.rs` RAII helpers.

### RAII Guards for Process-Global Mutations

When tests mutate process-global state (`$HOME`, override slots, etc.), use RAII guards:

```rust
pub struct HomeGuard {
    _previous: PrevHome,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        // Restore in declared-field order: _previous before _lock
    }
}

// Usage
let _guard = HomeGuard::install(test_home_path);
// guard drops, HOME is restored, mutex is released
```

**Discipline**: declare fields in reverse drop order. Documented at `tests/common/mod.rs` §HOME serialisation.

### Reference-Counted Shared Resources

For on-disk resources shared across workspaces (e.g., catalog cache dirs), reference-count instead of unconditionally deleting:

```rust
// src/catalog/store.rs
pub fn reference_count(url: &str, paths: &Paths) -> Vec<Scope> {
    // Walk global config + every workspace in workspaces.txt
    // Return list of scopes that still reference this URL
}
```

**TOCTOU profile**: concurrent removes race benignly; concurrent add + remove may leave a dangling reference recoverable by re-fetching.

### Structural Fix Over Targeted Fix

When a refactor touches multiple call sites, do a mechanical sweep across all sites rather than targeted fixes at a few.

**Example**: Phase 4 US1 mechanical sweep threaded `ResolvedScope` + `Paths::*_for(&scope)` through every command surface (35 files touched).

### Helper Visibility Promotion (Rule of 3)

When a helper function is reused at 3+ call sites, promote it to `pub` or move it to a shared module.

**Example**: `paths_for(&ToolEnv) -> Paths` was duplicated across 3 test files, then promoted to `tests/common/mod.rs`.

## Comments & Documentation

| Type | Format | Usage |
|------|--------|-------|
| Module-level doc | `//! ...` | Every `mod.rs` explains its purpose |
| Function-level doc | `/// ...` | Public functions get docs with examples |
| Inline comments | `// ...` | Explain *why* — assume reader knows Rust |
| TODO/FIXME | `// TODO: ...` | Mark incomplete work |
| Compiler directives | `#[doc(hidden)]` | Mark test-injection seams |

## Git Conventions

### Commit Messages

Format: `type(scope): description`

| Type | Usage |
|------|-------|
| feat | New feature (phase deliverable) |
| fix | Bug fix |
| refactor | Code restructure (no behavior change) |
| test | Test additions or fixes |
| docs | Documentation |
| chore | Maintenance, dependencies |

**Enforcement**: `cocogitto` hook at `.githooks/commit-msg` validates format. Constitution principle IX mandates this.

### Branch Naming

Trunk-based development. Short-lived feature/fix branches off `main`.

### PR Strategy

- **Small batches**: ~400 lines or 2 modules max as soft cap.
- **Chunked slices**: multi-PR features follow numbered slices (PR #74 Slice 1a, PR #75 Slice 1b).
- **Focused commits**: one logical change per commit.

## Import Ordering

Standard order:

1. External crates (`clap`, `serde`, `rusqlite`)
2. Internal crates (rare; Tome is single-binary)
3. Crate-root re-exports (`crate::commands`, `crate::error`)
4. Relative imports (`self::helper`, `super::parent`)

```rust
use std::path::PathBuf;

use clap::Parser;
use rusqlite::Connection;

use crate::commands;
use crate::error::TomeError;

use self::helper;
use super::config;
```

## Module Organization

Capability-oriented: each module owns one cohesive feature.

| Module | Responsibility |
|--------|-----------------|
| `catalog` | Catalog manifest, git cloning, registry persistence |
| `config` | Top-level `config.toml` / per-catalog entries (legacy) |
| `paths` | XDG path resolution, Tome root layout |
| `error` | Closed `TomeError` enum + exit codes |
| `commands` | CLI command dispatch + emission adapters |
| `plugin` | Plugin manifest, frontmatter, lifecycle |
| `index` | SQLite database, schema, migrations, skill CRUD, queries |
| `embedding` | Embedder/Reranker traits, FastembedEmbedder impl |
| `presentation` | CLI tables, spinners, colour, prompts |
| `workspace` | Workspace lifecycle, binding, resolution, scope |
| `settings` | Layered settings composition, TOML edit |
| `harness` | Per-harness module integration, rules-file + MCP-config |
| `doctor` | System diagnostics, suggested fixes |
| `mcp` | Async MCP server (only async island) |
| `substitution` | Hand-rolled variable substitution (Phase 5) |
| `summarise` | LLM-based text summarization (Phase 4) |

## Testing Discipline

See `TESTING.md` for detailed test patterns, but core conventions:

- **One assertion per test** (or tightly-related assertions).
- **Test file names match functionality**: `catalog_add.rs` tests `tome catalog add`.
- **Common helpers in `tests/common/mod.rs`**: `ToolEnv`, `Fixture`, guard patterns.
- **Isolation via TempDir**: every test gets fresh `$HOME` and XDG layout.
- **No real I/O by default**: models, embedders, git clones fabricated or stubbed.

## Strictness at Boundaries

- **Input strictness**: `#[serde(deny_unknown_fields)]` on Tome-owned inputs.
- **Output leniency**: third-party JSON and YAML parsed leniently.
- **Credentials scrubbed**: all error chains + model URLs sanitized before logging.
- **Symlink refusal**: symlink paths refused at every read/write entry point.

---

*This document defines HOW to write code. Update when conventions change or a new pattern reaches 3+ uses.*
