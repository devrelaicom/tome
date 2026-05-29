# Coding Conventions

> **Purpose**: Document code style, naming conventions, error handling, and patterns for Tome (Rust CLI).
> **Generated**: 2026-05-27
> **Last Updated**: 2026-05-29 (Phase 6 Foundational)

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
| Constants | SCREAMING_SNAKE_CASE | `MAX_RETRIES`, `LONG_MAX_CHARS`, `MCP_SLASH_PREFIX` |
| Functions | snake_case, verb-prefix | `enable_plugin`, `resolve_plugin_dir`, `assemble_report` |
| Structs | PascalCase | `TomeError`, `Config`, `Fixture` |
| Enums | PascalCase, variants as items | `ScopeKind`, `CompositionErrorKind`, `EntryKind` |
| Traits | PascalCase | `Embedder`, `Reranker`, `HarnessModule` |
| Type aliases | PascalCase or semantic | `ResolvedScope`, `SubstitutionContext` |

**Phase 5 constant promotion pattern**: String literals that appear across multiple modules become constants at their first cross-module consumer. Example: `MCP_SLASH_PREFIX = "/mcp__"` defined in `src/mcp/mod.rs` once, consumed by `commands/doctor.rs` (establishes rule-of-3).

### Phase 5: Stringly-Typed Enum Dispatch Pattern

When deserializing enum variants from database or JSON strings, prefer `kind.parse::<EntryKind>()` + match-on-variants over stringly-typed `match kind.as_str()`:

```rust
// Good: type-safe dispatch
let kind = stored_kind.parse::<EntryKind>()?;
match kind {
    EntryKind::Skill => { ... },
    EntryKind::Command => { ... },
}

// Bad: stringly-typed dispatch
match kind.as_str() {
    "skill" => { ... },
    "command" => { ... },
    _ => { ... },  // Unknown branches are error-prone
}
```

**Rationale** (Phase 5 Polish M-3): Schema-column enums should round-trip through type-safe parsing. The Unknown case surfaces as `IndexIntegrityCheckFailure` (exit 51) — a clear signal to the user that their database schema is ahead of the running binary. Used in `doctor/checks.rs`, `commands/plugin/show.rs`, and future entry-kind dispatch sites.

### Phase 6: Canonical-Enum Dispatch (Exhaustive Matching Without Catch-All)

When matching on enum types that may grow (e.g., `EntryKind` now admits `Skill`, `Command`, `Agent`), use exhaustive pattern matching without a catch-all (`_ => `) arm:

```rust
// Good: compiler enforces when new variants are added
match kind {
    EntryKind::Skill => count.skills = n,
    EntryKind::Command => count.commands = n,
    EntryKind::Agent => count.agents = n,
}

// Bad: allows silent miscount if variant is added later
match kind {
    EntryKind::Skill => count.skills = n,
    EntryKind::Command => count.commands = n,
    _ => count.unknown += n,  // Silent on new variants
}
```

**Rationale** (Phase 6 Foundational F2): When the compiler adds a missing-arm error, it's a hard signal to review all consumers of that type. The exhaustive discipline surfaces schema drift as `IndexIntegrityCheckFailure` (exit 51) rather than silently miscounting or skipping entity types. Every exhaustive `match EntryKind` site was widened by hand (no automated macro expansion) to include the new `Agent` variant.

**Applied to**: `src/doctor/checks.rs` (entry count accumulators), `src/commands/plugin/mod.rs` (JSON serialization), `src/commands/plugin/show.rs` (display grouping), `src/plugin/frontmatter.rs` (user-invocable defaults), `src/plugin/lifecycle.rs` (entry creation factories).

## Error Handling

### Error Pattern: Closed Enum with Exit Codes

Tome uses a **closed `TomeError` enum** — every error variant has an enumerated exit code. The compiler enforces the chain: adding a variant forces edits to `tests/exit_codes.rs`, the spec, and contracts.

**File**: `src/error.rs`

**Exit codes by phase**:
- Phase 1: codes 2–8, plus Internal=1
- Phase 2: codes 20–52 (plugins, index, embedding)
- Phase 3: codes 60–75 (MCP, doctor, schema migration)
- Phase 4: codes 13–20 (workspace, harness, composition)
- Phase 5: codes 9, 21–26 (substitution, commands, prompts, data dirs)
- Phase 6: codes 43–46 (hooks, agents, guardrails)

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
| `warn!` | Recoverable issues, state drift, read_dir failures | `warn!("length window exceeded"); warn!("read_dir error on {}: {}", path, err)` |
| `info!` | Important transitions, user-facing events | `info!("plugin enabled: {}")` |
| `debug!` | Internal state, algorithm details | `debug!("cost function: {}")` |

**Phase 5 read_dir pattern**: Silent-bail on `NotFound` (legitimate under FR-124 read-only doctor invariant), emit `warn!` with `path` + `error` fields on other errors. Used in `doctor::assemble_report` multi-statement snapshot reads.

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

**Pattern used in**: `commands/status/mod.rs`, `commands/plugin/{enable,list,show}.rs`, `commands/reindex.rs`, `commands/doctor.rs`, `commands/workspace/mod.rs`.

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

**Phase 5 examples**: 
- `Paths::plugin_data_root()` — single-source-of-truth accessor at first cross-module consumer per US5.a
- `body_has_bare_arguments()` — promoted to `pub` in `src/substitution/mod.rs` for use in `prompts/get` + `get_skill` MCP tools (US3.d R-M1)
- `build_context_for_entry()` — extracted to `src/substitution/context.rs` at Polish M-2 to serve both `prompts.rs::build_get_context` and `get_skill.rs::build_substitution_context`
- `validate_db_stored_path()` — extracted to `src/index/skills.rs` at Polish M-4 to serve both `resolve_entry_body_path` and `commands/plugin/show.rs::list_entries`

### Single-Source-of-Truth for Length Windows

When multiple modules need the same length limit (e.g., short vs long summaries, description truncation), define it once:

```rust
// src/substitution/mod.rs (Phase 5)
pub const MAX_DESCRIPTION_MAX_CHARS: u32 = 100_000;

// Reference it everywhere
const DEFAULT_DESCRIPTION_MAX: u32 = 150;
```

**Rationale**: Phase 4 US4 caught a `LONG_MAX_CHARS` split (2400 vs 2500) across `src/mcp/prompts.rs` and `src/commands/workspace/regen_summary.rs`. Single source discovered at first cross-module consumer.

### Snapshot Consistency for Diagnostic Reads

When a doctor command or other read-only tool makes multiple SQL reads, use `conn.unchecked_transaction()` on read-only connections to establish a consistent snapshot:

```rust
// src/doctor/mod.rs (Phase 5 US5.a)
let conn = index::db::open_read_only(&paths.index_db)?;
let _tx = conn.unchecked_transaction()?;  // Snapshot boundary

// All reads within this scope see consistent schema_version + rows
let schema_version = meta::get_schema_version(&conn)?;
let workspaces = list_workspaces(&conn)?;
```

**Rationale**: `unchecked_transaction()` is valid on read-only connections; works because the transaction is purely a snapshot boundary, rollback is implicit on Drop (Phase 5 US5.a pattern).

### Phase 5: Caller-Controlled String Truncation

When truncating strings for length limits (e.g., MCP tool descriptions), use `char_indices` for boundary-safe truncation that avoids O(n) full-string traversal:

```rust
// Phase 5 US4.d C-M2 + Polish M-1 pattern: O(max) bounded-walk truncation
const MAX_DESCRIPTION: usize = 8000;

if description.len() > MAX_DESCRIPTION {
    // char_indices walks at most max+1 chars; never full string
    if let Some((idx, _)) = description
        .char_indices()
        .take_while(|&(i, _)| i < MAX_DESCRIPTION)
        .last()
    {
        description.truncate(idx);
        description.push('…');  // U+2026 ellipsis
    }
}
```

**Alternative (fixed-window O(1) lookup)**:
```rust
// For fixed-size windows: capture indices, drop all after limit
let safe_len = description
    .char_indices()
    .nth(MAX_CHARS)
    .map(|(i, _)| i)
    .unwrap_or(description.len());
description.truncate(safe_len);
```

**Security note** (US4.d HIGH fix): Pre-fix implementations did TWO `chars()` passes per value (early `chars().count()` check + `chars().take().collect()` for truncation). With caller-controlled `max` × `top_k` results × multi-KB inputs, this was a DoS amplifier. Bounded-walk discipline prevents regression: **never call `chars().count()` as an early gate**; **only walk up to the limit**.

**Used in**: `src/mcp/tools/search_skills.rs` (US4), `src/mcp/prompts.rs` (Polish M-1), any bounded-text CLI output.

### Phase 5: JSON Output with Alphabetical Key Order

For deterministic JSON wire shapes, use `BTreeMap` or `IndexMap` (via `serde_json` feature `preserve_order`) for objects:

```rust
// src/mcp/tools/*.rs — Serialize with key ordering
use std::collections::BTreeMap;

#[derive(Serialize)]
struct SearchResult {
    #[serde(flatten)]
    fields: BTreeMap<String, serde_json::Value>,
}
```

**Rationale**: MCP tools and CLI output that's parsed by external consumers benefit from deterministic field order. `serde_json` with `preserve_order` feature re-exports `IndexMap`; explicit `BTreeMap` is clearer where sorted order is the intent. Phase 5 US4.d solidified this: all new Serialize types get byte-stable JSON pins via `*_json_shape.rs` test files.

### Phase 5: Sanity Caps for Deserialized Values

When deserializing bounds that should be "reasonable but not enforced at the syntax level", apply sanity caps at code level:

```rust
// src/mcp/tools/search_skills.rs — Phase 5 US4.c
const MAX_DESCRIPTION_MAX_CHARS: u32 = 100_000;  // Documented in contracts

fn validate_description_max(limit: Option<u32>) -> Result<u32, TomeError> {
    match limit {
        None => Ok(DEFAULT_DESCRIPTION_MAX),
        Some(l) if l < 1 => Err(TomeError::InvalidDescriptionMaxChars),
        Some(l) if l > MAX_DESCRIPTION_MAX_CHARS => Err(TomeError::InvalidDescriptionMaxChars),
        Some(l) => Ok(l),
    }
}
```

**Pattern**: Negative/absurd values caught at deserialization boundary; sensible but very large values documented in contracts per FR-092.

### Phase 5: Stub MetaSeed Pattern in Tests

When tests inject model identity via `embedder_entry` / `reranker_entry` that must match `StubEmbedder`'s compiled-in identity, use an explicit stub seed:

```rust
// tests/common/mod.rs (future: promote when 3rd caller emerges)
pub fn stub_embedder_seed() -> MetaSeed {
    MetaSeed {
        name: "stub-embedder".to_string(),
        version: "0.0.0".to_string(),
        entry_point: "stub".to_string(),
    }
}

// In test setup:
let deps = LifecycleDeps {
    embedder_seed: stub_embedder_seed(),
    // ... other fields ...
};
```

**Rationale** (Phase 5 US4.a C-B1): Tests that bootstrap the index `meta` table with synthetic seeds must match what the stub implementations report. Single source of truth prevents name-drift between test harness and stub.

### Phase 5: Unicode Truncation Boundary Testing

When tests verify string truncation, include edge cases for multi-byte UTF-8 sequences:

```rust
#[test]
fn truncate_respects_char_boundaries_with_emoji() {
    // 4-byte UTF-8 emoji × N to verify char-not-byte slicing
    let input = "hello 👋 world 🌍 test";  // Emoji are 4 bytes each
    let max_len = 15;
    
    let result = truncate_description(&input, max_len);
    assert!(result.is_char_boundary(result.len()), "truncation must land on char boundary");
    assert!(!result.ends_with("👋"), "emoji should not be sliced in half");
}
```

**Pattern**: Use emoji or other multi-byte sequences to catch byte-slicing bugs that would panic on `String::truncate()`. Phase 5 US4.d T-M1 added this pattern.

### Phase 5: Exact-Count Assertions Over Heuristics

When testing deterministic fixtures with known entity counts, use exact assertions instead of inequality checks:

```rust
// Phase 5 US5.a T-W1 pattern
#[test]
fn doctor_p5_surface_creates_no_dirs() {
    let fix = Fixture::build_sample();  // 4 skills, 2 plugins
    let env = ToolEnv::new();
    
    // Count files before
    let before_count = count_all_files(&env.home);
    
    // Run doctor
    let out = env.cmd().args(["doctor"]).output().unwrap();
    assert!(out.status.success());
    
    // Count files after — exact match proves read-only
    let after_count = count_all_files(&env.home);
    assert_eq!(before_count, after_count, "doctor must not create any directories");
}
```

**Rationale**: Masks off-by-one regressions better than `>=` checks. Phase 5 US5.a extended this: exact entry counts (`assert_eq!(counts.skills, 4)`) prevent silent mutations.

### Phase 6: Test-Configurable Test Double Pattern

The `StubHarness` test double in `src/harness/stub.rs` evolved from a unit struct to a `#[derive(Default)]` struct with optional config fields and builder setters (`with_*`). This allows tests to drive different harness capabilities without spelling out the full struct:

```rust
// Old pattern (unit struct, not configurable)
let harness = StubHarness;

// New pattern (configurable via Default + builder)
let harness = StubHarness::default()
    .with_hook_settings()
    .with_native_agents(AgentFormat::MarkdownYaml);
```

**Discipline**: `Default` produces a safe baseline (safe defaults on all trait methods); builder methods enable specific capabilities for the test case. All field defaults remain backward-compatible with the original behaviour. Used in `tests/harness_trait_p6.rs` to exercise Phase 6 hook + agent dispatch without binding to production harness modules.

### Phase 6: Marker-Only Migration Pattern

A registered `Migration` whose `apply` function is a documented no-op, used solely to advance `SCHEMA_VERSION` when a column's domain widens (e.g., `kind` text enum from `{'skill', 'command'}` to `{'skill', 'command', 'agent'}`). No DDL, no backfill — just the version bump for audit and `doctor` schema consistency checks.

```rust
// src/index/migrations.rs
Migration {
    from_version: 3,
    to_version: 4,
    description: "Phase 6: widen 'kind' domain to admit 'agent'",
    apply: |_conn| {
        // No-op apply; domain widening needs no DDL on TEXT columns
        Ok(())
    },
}
```

**Used in**: `tests/schema_migration_p6.rs` pins the production marker migration. Discovered at Phase 6 Foundational when `EntryKind` was widened to include `Agent` variant but the index schema remained at version 3.

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
- **4-reviewer parallel pass** (Phase 5+ pattern): contract audit / Rust-lens / test audit / security audit in parallel at phase-wide closeout (distinct from per-US passes). Findings + disposition committed BEFORE fixes land. Pattern established in Phase 5 Polish: 4 agents in ONE message, captures cross-US drift.

## Import Ordering

Standard order:

1. External crates (`clap`, `serde`, `rusqlite`)
2. Internal crates (rare; Tome is single-binary)
3. Crate-root re-exports (`crate::commands`, `crate::error`)
4. Relative imports (`self::helper`, `super::parent`)

```rust
use std::path::PathBuf;
use std::collections::{HashSet, HashMap};  // Phase 5: consolidated multiline imports

use clap::Parser;
use rusqlite::Connection;

use crate::commands;
use crate::error::TomeError;

use self::helper;
use super::config;
```

**Phase 5 US5.c pattern**: `use std::collections::{HashSet, HashMap};` at top-of-file over `std::collections::HashSet` / `std::collections::HashMap` fully-qualified in function bodies.

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
- **Output leniency**: third-party JSON and YAML parsed leniently. Output `Serialize` types do NOT carry `#[serde(deny_unknown_fields)]` (Phase 5: consistency pattern established).
- **Credentials scrubbed**: all error chains + model URLs sanitized before logging.
- **Symlink refusal**: symlink paths refused at every read/write entry point.
- **Path validation**: database-stored paths validated to be relative + `..`-free via `validate_db_stored_path()` before use in `fs::read` (Phase 5 Polish M-4).

---

*This document defines HOW to write code. Update when conventions change or a new pattern reaches 3+ uses.*
