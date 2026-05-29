# Coding Conventions

> **Purpose**: Document code style, naming conventions, error handling, and patterns for Tome (Rust CLI).
> **Generated**: 2026-05-27
> **Last Updated**: 2026-05-29 (Phase 6 US4 — agent personas via MCP prompts)

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

**Strictness boundary** (FR-013a): `#[serde(deny_unknown_fields)]` applies to Tome-owned inputs (`config.toml`, model manifests, index `meta` rows, settings structs). Third-party inputs (SKILL.md YAML frontmatter, `plugin.json` metadata, agent frontmatter) parse leniently — enforced by `tests/manifest_strictness.rs` grep guard.

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
| Constants | SCREAMING_SNAKE_CASE | `MAX_RETRIES`, `LONG_MAX_CHARS`, `MCP_SLASH_PREFIX`, `DROP_PERSONA_NAME` |
| Functions | snake_case, verb-prefix | `enable_plugin`, `resolve_plugin_dir`, `assemble_report` |
| Structs | PascalCase | `TomeError`, `Config`, `Fixture` |
| Enums | PascalCase, variants as items | `ScopeKind`, `CompositionErrorKind`, `EntryKind` |
| Traits | PascalCase | `Embedder`, `Reranker`, `HarnessModule` |
| Type aliases | PascalCase or semantic | `ResolvedScope`, `SubstitutionContext` |

**Phase 5 constant promotion pattern**: String literals that appear across multiple modules become constants at their first cross-module consumer. Example: `MCP_SLASH_PREFIX = "/mcp__"` defined in `src/mcp/mod.rs` once, consumed by `commands/doctor.rs` (establishes rule-of-3).

**Phase 6 US4 constant promotion**: `DROP_PERSONA_NAME = "drop-persona"` defined once in `src/mcp/prompts.rs`, consumed by `mcp::state` and prompt-collision detection (`src/mcp/prompt_collision.rs`).

### Phase 5: Stringly-Typed Enum Dispatch Pattern

When deserializing enum variants from database or JSON strings, prefer `kind.parse::<EntryKind>()` + match-on-variants over stringly-typed `match kind.as_str()`:

```rust
// Good: type-safe dispatch
let kind = stored_kind.parse::<EntryKind>()?;
match kind {
    EntryKind::Skill => { ... },
    EntryKind::Command => { ... },
    EntryKind::Agent => { ... },
}

// Bad: stringly-typed dispatch
match kind.as_str() {
    "skill" => { ... },
    "command" => { ... },
    "agent" => { ... },
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

**Phase 6 US4 examples**:
- `settings::scopes` module — canonical scope-loaders (`load_project_marker`, `load_workspace_settings`, `load_global_settings`) single-sourced from prior triplicate copies in `commands::harness::list`, `harness::sync`, and MCP server startup (R-4-2). Each resolver consistent error classification + reason strings.
- `resolve_scalar` + `resolve_scalar_with` — first-declarer-wins scalar resolver for Phase 6 boolean settings (FR-053, R-12). The generic closure form (`resolve_scalar_with`) enables reuse for both `expose_agents_as_personas` and `strip_plugin_agent_privileges` (US5) without duplication.

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

### Phase 6: Validate Third-Party Names as a Single Safe Path Segment (S-1)

When a plugin-supplied name (e.g., agent `name` frontmatter field) will be composed into a filesystem path, validate it at the boundary before storing or emitting:

```rust
// src/harness/agents.rs::is_safe_agent_name
pub(crate) fn is_safe_agent_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    // NUL can never appear in a valid path component
    if name.contains('\u{0}') {
        return false;
    }
    // Reject separators on either platform
    if name.contains('/') || name.contains('\\') {
        return false;
    }
    // Explicit traversal / dotfile rejection
    if name == "." || name == ".." || name.starts_with('.') {
        return false;
    }
    // Robust backstop: exactly one `Component::Normal` equal to the name
    let mut comps = Path::new(name).components();
    match (comps.next(), comps.next()) {
        (Some(std::path::Component::Normal(seg)), None) => seg == std::ffi::OsStr::new(name),
        _ => false,
    }
}
```

The emitted filename `<plugin>__<name>.<ext>` is joined onto each harness's agent dir; an attacker-controlled `name` like `../../../../tmp/evil` would otherwise escape. **Index-time gate**: a `name` that fails `is_safe_agent_name` maps to `TomeError::AgentTranslationFailed` (exit 45) with no row stored. Paired with a defensive `target.parent() == Some(dir)` check at the write site (`sync` reconciliation). Applied at `src/plugin/lifecycle.rs::enable_plugin_atomic` before the agent row is inserted.

**Used in**: `tests/agent_path_traversal.rs` (S-1 defence), `tests/agent_naming_clash.rs` (name as the `<name>` half of `<plugin>__<name>`).

### Phase 6: Same-Vendor-Only Model Alias Table

Never cross-vendor when translating a plugin agent's `model` field to a harness-native identifier. Use per-harness policy slots even when identifiers aren't enumerated:

```rust
// src/harness/agents.rs::map_model
pub(crate) fn map_model(harness: &str, source: &str) -> Option<String> {
    if source == "inherit" {
        return None;  // No "inherit" across harnesses
    }
    match harness {
        "claude-code" => Some(source.to_owned()),  // Canonical vendor: pass through
        "codex" => None,  // OpenAI-vendored: no Anthropic alias maps
        "cursor" => None,  // Drop (no enumerated Anthropic ids yet)
        "opencode" => match source {
            "opus" => Some("anthropic/claude-opus-4.7".to_owned()),
            "sonnet" => Some("anthropic/claude-sonnet-4.7".to_owned()),
            "haiku" => Some("anthropic/claude-haiku-4.7".to_owned()),
            _ => None,
        },
        _ => None,  // Unknown harness: drop conservatively
    }
}
```

**Contract**: pinned in `contracts/agent-translation.md` (SC-002). The policy is fixed; harness-native identifiers are confirmed at implementation time. Drop-on-no-target is intentional — an Anthropic-sourced `model: opus` → Codex yields `None` (Codex never gets an Anthropic id). Verified in `tests/agent_translate_*.rs`.

### Phase 6: `plugin_of_owned_file` as Single Source of Truth

The inverse of the `<plugin>__<name>.<ext>` filename builder, `plugin_of_owned_file` is the sole provenance rule consumed by both agent emission and cleanup:

```rust
// src/harness/agents.rs::plugin_of_owned_file
pub(crate) fn plugin_of_owned_file(filename: &str) -> Option<&str> {
    let (plugin, rest) = filename.split_once("__")?;
    if plugin.is_empty() {
        return None;
    }
    // Require a non-empty `<name>` before the extension dot
    let stem = rest.rsplit_once('.').map(|(s, _)| s).unwrap_or(rest);
    if stem.is_empty() {
        return None;
    }
    Some(plugin)
}
```

The double-underscore separator (`__`) distinguishes Tome-owned agent files from user or harness files containing a single underscore. Every harness's removal glob and sync reconciliation's per-plugin and orphan-cleanup passes route through this one accessor. **Rationale** (instance of helper visibility rule-of-3): establishes a single query point so the ownership rule is never re-rolled across call sites (FR-043).

**Used in**: `src/harness/sync.rs` (both removal passes), `tests/agent_removal.rs` (ownership verification).

### Phase 6: Embedding-Skip via `Option<&[f32]>` + `embed_unless_agent` Predicate

When indexing entries that include agents alongside skills (both live in the same database), skip embedding for agent rows and preserve the embedding column as a predicate decision point:

```rust
// src/index/skills.rs::embed_unless_agent
fn embed_unless_agent<F>(
    pending: &PendingSkill,
    embed: &mut F,
) -> Result<Option<Vec<f32>>, TomeError>
where
    F: FnMut(&str) -> Result<Vec<f32>, TomeError>,
{
    if pending.kind == EntryKind::Agent {
        return Ok(None);  // Agents are never embedded
    }
    let vector = embed(&embedding_text(...))?;
    Ok(Some(vector))
}
```

The embedding column is `BLOB` or `NULL`; agent rows carry `NULL` (never a vector). The predicate is single-sourced in `embed_unless_agent` so both the enable path and later reindex operations use the same rule. **Rationale**: agents are not searchable (FR-063); queries filter on `embedding IS NOT NULL` to exclude them. Documented in `contracts/agent-translation.md` (FR-063).

**Used in**: `src/index/skills.rs` (enable + reindex paths), verified in `tests/entry_kind_agent_indexing.rs` (agents absent from search results).

### Phase 6: Content-Driven Codex TOML Triple-Quoting via `toml_edit`

When rendering Codex agent files in TOML format with multi-line bodies, use `toml_edit` to render the body in a triple-quoted `developer_instructions` string. The library automatically promotes multi-line strings to the multiline basic form (`"""…"""`):

```rust
// src/harness/agents.rs::render_codex_toml
pub(crate) fn render_codex_toml(scalars: &[(String, String)], body: &str) -> String {
    use toml_edit::{DocumentMut, value};

    let mut doc = DocumentMut::new();
    for (k, v) in scalars {
        doc[k.as_str()] = value(v.as_str());
    }
    doc["developer_instructions"] = value(body);  // Automatically triple-quoted if multiline
    doc.to_string()
}
```

**Discipline**: Never hand-roll TOML quoting or escaping. `toml_edit`'s `value()` function's default string representation promotes any value containing a newline to `"""…"""` — exactly the triple-quoted form the contract mandates (FR-033 / R-14). Agent bodies are Markdown (multi-line), so the promotion is reliable and deterministic. Verified in `tests/agent_translate_codex.rs::body_lands_in_triple_quoted_developer_instructions`.

### Phase 6: `reconcile_<sink>` Template for Sync Orchestration (US2/US3)

When writing a harness sync reconciliation function (hooks, agents, guardrails), follow the `reconcile_hooks` / `reconcile_agents` / `reconcile_guardrails` template in `src/harness/sync.rs`:

```rust
fn reconcile_guardrails(
    deps: &SyncDeps<'_>,
    effective_names: &HashSet<String>,
    snapshots: &[HarnessSnapshot],
    suppressed: &HashMap<String, HashSet<String>>,  // <harness> → <catalog>:<plugin>
    outcome: &mut SyncOutcome,
) -> Result<GuardrailsReconciliation, TomeError> {
    // 1. Open the central DB read-only (propagate the error for an EXISTING DB;
    //    never .ok()-swallow a non-absent database).
    let conn = if deps.paths.index_db.exists() {
        Some(crate::index::open_read_only(&deps.paths.index_db)?)
    } else {
        None
    };

    // 2. Compute shared inputs once (enabled plugins, guardrails bodies, etc.).
    let workspace = deps.workspace_name.as_str();
    let enabled = match &conn {
        Some(c) => crate::index::skills::enabled_plugins_for_workspace(c, workspace)?,
        None => Vec::new(),
    };

    // 3. Per-harness loop with an actions map + first_error forward-progress.
    let mut actions: HashMap<String, Action> = HashMap::new();
    let mut first_error: Option<TomeError> = None;

    for snap in snapshots {
        // A write failure on harness A doesn't stop harness B from being processed.
        match process_harness_guardrails(snap, &effective_names, &enabled, suppressed) {
            Ok(action) => { actions.insert(snap.name.clone(), action); },
            Err(e) if first_error.is_none() => {
                first_error = Some(e);
                actions.insert(snap.name.clone(), Action::LeftAlone);
            }
            Err(_) => {
                actions.insert(snap.name.clone(), Action::LeftAlone);
            }
        }
    }

    // 4. Append the new SyncSubsystem variant LAST so the byte-stable JSON pin
    //    only gains a trailing field (Guardrails appended after Agents/Hooks).
    let recon = GuardrailsReconciliation { actions, first_error };
    Ok(recon)
}
```

**Discipline**: The template ensures proper error propagation (abort on existing DB, forward-progress on per-harness failures), single computation of shared inputs, and byte-stable JSON ordering (new `SyncSubsystem::Guardrails` field appended to `HarnessDecision` LAST). Used in `src/harness/sync.rs` at lines 913+ (reconcile_agents) and 1100+ (reconcile_guardrails); verified in `tests/harness_sync_stub.rs` (agents + guardrails forward-progress tests).

### Phase 6: Targeted Two-Token Rewrite via Fixed-Needle `str::replace` (US2)

When rewriting path variables in JSON hook specifications, use fixed-needle `str::replace` (not the Phase 5 substitution pipeline, which violates NFR-007) over JSON string VALUES only (keys untouched):

```rust
// src/harness/hooks.rs::rewrite_string_leaves
fn rewrite_string_leaves(value: &mut serde_json::Value, plugin_root: &str, plugin_data: &str) {
    match value {
        serde_json::Value::String(s) => {
            if s.contains("${CLAUDE_PLUGIN_ROOT}") {
                *s = s.replace("${CLAUDE_PLUGIN_ROOT}", plugin_root);
            }
            if s.contains("${CLAUDE_PLUGIN_DATA}") {
                *s = s.replace("${CLAUDE_PLUGIN_DATA}", plugin_data);
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                rewrite_string_leaves(item, plugin_root, plugin_data);
            }
        }
        serde_json::Value::Object(map) => {
            // Only the VALUES are rewritten; keys stay verbatim.
            for (_k, v) in map.iter_mut() {
                rewrite_string_leaves(v, plugin_root, plugin_data);
            }
        }
        _ => {}  // Numbers / booleans / null carry no rewritable text.
    }
}
```

**Discipline**: Exactly two tokens (`${CLAUDE_PLUGIN_ROOT}` / `${CLAUDE_PLUGIN_DATA}`); all other `${CLAUDE_*}` left verbatim (Claude Code resolves them). Fixed-needle `replace` is ReDoS-free and **cannot** accidentally match a longer variable name (e.g., `${CLAUDE_PLUGIN_ROOTX}` survives as `<root>X`, harmless because no such variable exists). Applied only to JSON string leaves (contract FR-003, R-4). Verified in `tests/hooks_rewrite.rs`.

### Phase 6: Structural-Deep-Equal Ownership for Config Merges (US2)

When merging hooks or other entries into a configuration file, establish ownership solely by re-derivation + deep-equal comparison, with no sidecar provenance marker (NFR-003):

```rust
// src/harness/hooks.rs::merge_into_settings
fn append_if_absent(
    hooks_obj: &mut serde_json::Map<String, serde_json::Value>,
    event: &str,
    entry: &serde_json::Value,
) -> bool {
    let arr = hooks_obj
        .entry(event.to_string())
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    if !arr.is_array() {
        *arr = serde_json::Value::Array(Vec::new());
    }
    let Some(items) = arr.as_array_mut() else {
        return false;
    };
    // Idempotent: add only if no deep-equal entry exists.
    if items.iter().any(|existing| existing == entry) {
        return false;
    }
    items.push(entry.clone());
    true
}

// src/harness/hooks.rs::remove_from_settings
fn remove_if_present(
    hooks_obj: &mut serde_json::Map<String, serde_json::Value>,
    event: &str,
    entry: &serde_json::Value,
) -> bool {
    let Some(items) = hooks_obj.get_mut(event).and_then(serde_json::Value::as_array_mut) else {
        return false;
    };
    let before = items.len();
    // Removal deletes only the deep-equal entry (never user-edited).
    items.retain(|existing| existing != entry);
    before != items.len()
}
```

**Discipline**: A user-edited hook no longer matches its deep-equal source after Tome wrote it — it is never deleted. Symmetrically, on add, a user-authored entry identical to Tome's re-derivation counts as present and is not duplicated. The contract mandates no sidecar (FR-004, FR-005, NFR-003). Verified in `tests/hooks_merge.rs` (user-edited preservation, idempotence, dedup).

### Phase 6: Fail Closed on Non-UTF-8 Load-Bearing Paths (US2)

When a path becomes an executed command or load-bearing data, fail closed (exit 44) on non-UTF-8 paths rather than silently corrupting the path via `to_string_lossy`:

```rust
// src/harness/hooks.rs::non_utf8_guard
fn non_utf8_guard<'a>(path: &'a Path, error_path: &Path) -> Result<&'a str, TomeError> {
    path.to_str().ok_or_else(|| {
        // The rewritten value becomes a hook COMMAND; U+FFFD corruption
        // silently breaks the hook instead of refusing (R2-2).
        settings_write_failed(
            error_path,
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("non-UTF-8 hook rewrite target path: {}", path.display()),
            ),
        )
    })
}
```

**Rationale** (R2-2): Hook paths become command-line arguments in executed hooks. Non-UTF-8 paths with `to_string_lossy` substitution create silently-broken commands rather than failing early. Applied at `src/harness/hooks.rs::read_rewritten_entries` before rewrite, surfacing `TomeError::HookSettingsWriteFailed` (exit 44). Verified in `tests/hooks_merge.rs` (wrong-type settings → exit 44).

### Phase 6: Validate Verbatim Third-Party Content for Managed-Marker Collisions (US3)

When plugin content is copied verbatim into a marker-delimited region of a file Tome re-parses (e.g., `GUARDRAILS.md` body into a `<!-- START GUARDRAILS: … -->` region), scan it for ANY managed-marker regex that would let content escape its region, wedge the file, or corrupt sibling regions. The scan uses the exact same compiled regexes the reconciler parses with:

```rust
// src/harness/guardrails.rs::body_contains_marker_line (B-1 fail-closed pattern)
fn body_contains_marker_line(body: &str) -> bool {
    body.split('\n').any(|line| {
        // Check for guardrails START/END markers
        start_regex().is_match(line) ||
            end_regex().is_match(line) ||
            // Check for Phase 4 tome:begin/end block markers
            block_marker_regex().is_match(line)
    })
}
```

A body that fails the validation surfaces `TomeError::GuardrailsWriteFailed` (exit 46) naming the source file. Escaping the body is wrong (it is contractually verbatim), so refusal is the defence against region-escape/file-wedge. The reconcile loop records this on its forward-progress error slot and keeps reconciling sibling plugins (FR-084).

**Used in**: `src/harness/guardrails.rs::read_guardrails_source` (boundary validation), verified in `tests/guardrails_marker_injection.rs` (three crafted marker-injection bodies each rejected; sibling plugin region still renders; re-sync convergent).

### Phase 6: Parameterised Keyed Marker Family for Per-Plugin Regions (US3)

The `MarkerSpec` type in `src/harness/rules_file.rs` generalises the single `tome:begin/end` block into a parameterised keyed-marker family. Multiple keyed regions coexist with the block on the same file; deterministic placement ensures idempotence:

```rust
// src/harness/rules_file.rs::MarkerSpec / find_marker_regions / compose_in_file
pub struct MarkerSpec {
    start_regex: Regex,
    end_regex: Regex,
    render_begin: fn(&str) -> String,
    render_end: fn(&str) -> String,
}

fn compose_in_file(target: &Path, desired: &BTreeMap<String, String>) -> Result<(), TomeError> {
    // 1. Read existing file content
    let contents = std::fs::read_to_string(target)?;
    let mut lines: Vec<String> = contents.split('\n').map(|s| s.to_owned()).collect();

    // 2. Find all existing regions; overwrite in-place or mark for append
    let regions = find_marker_regions(&lines)?;
    let mut appends = BTreeMap::new();
    for (key, body) in desired {
        if let Some((start, end)) = regions.get(key) {
            // Overwrite between markers in place
            update_region_in_place(&mut lines, *start, *end, body)?;
        } else {
            // Queue for append in lex order
            appends.insert(key.clone(), body.clone());
        }
    }

    // 3. Append new regions in lexicographic order
    for (key, body) in &appends {
        lines.push(render_begin(key));
        lines.push(body.clone());
        lines.push(render_end(key));
    }

    // 4. Remove orphaned regions (desired keys not in file)
    prune_orphans(&mut lines, desired.keys())?;

    // 5. Write atomically
    write_atomic_idempotent(target, &lines)?;
    Ok(())
}
```

**Discipline**: Within a file: the `tome:begin/end` block is rendered first, then guardrails regions in lexicographic `<catalog>:<plugin>` order (FR-014). Existing regions are overwritten between their markers in place (never duplicated, never reordered); new regions are appended in lex order; orphaned regions are removed. A re-sync with no change rewrites nothing (idempotence via short-circuit compare, FR-525). Used in `src/harness/guardrails.rs::reconcile_in_file_region`, verified in `tests/guardrails_render.rs` (region placement, lexicographic order, overwrite-in-place, new-append, orphan-removal, idempotence).

### Phase 6 US4: First-Declarer-Wins Scalar Settings Resolver

When resolving a Phase 6 boolean setting (e.g., `expose_agents_as_personas`, `strip_plugin_agent_privileges`) across project → workspace → global scopes, apply a first-declarer-wins priority walk where the nearest scope that declares the field wins:

```rust
// src/settings/mod.rs (FR-053, R-12)
pub fn resolve_scalar(
    project: Option<bool>,
    workspace: Option<bool>,
    global: Option<bool>,
) -> bool {
    project.or(workspace).or(global).unwrap_or(false)
}

// Closure-based form for generic field accessors
pub fn resolve_scalar_with<FP, FW, FG>(
    project: Option<&ProjectMarkerConfig>,
    workspace: Option<&WorkspaceSettings>,
    global: &GlobalSettings,
    project_field: FP,
    workspace_field: FW,
    global_field: FG,
) -> bool
where
    FP: Fn(&ProjectMarkerConfig) -> Option<bool>,
    FW: Fn(&WorkspaceSettings) -> Option<bool>,
    FG: Fn(&GlobalSettings) -> Option<bool>,
{
    resolve_scalar(
        project.and_then(project_field),
        workspace.and_then(workspace_field),
        global_field(global),
    )
}
```

**Discipline**: This is deliberately NOT the `harnesses` composition grammar (`resolve_effective_list`): there is no list to union/subtract and no `[workspace]` / `[global]` / `!name` references. A project `false` simply overrides a global `true`. The closure form (`resolve_scalar_with`) enables reuse for multiple scalar settings (presently `expose_agents_as_personas`; US5 adds `strip_plugin_agent_privileges`) without code duplication — a new scalar adds a one-line call site extracting its field. Applied in `src/settings/mod.rs` (FR-053); verified in `tests/settings_p6.rs` (default/layering/first-declarer-wins behaviour).

### Phase 6 US4: SSOT Scope-Loaders for Settings Composition

When resolving the project marker + workspace settings + global settings triple, use canonical scope-loaders from `src/settings/scopes.rs` rather than triplicating the resolution logic:

```rust
// src/settings/scopes.rs — three functions with consistent error classification
pub(crate) fn load_project_marker(project_root: Option<&Path>) -> Result<Option<ProjectMarkerConfig>, TomeError> { ... }
pub(crate) fn load_workspace_settings(paths: &Paths, workspace_name: &WorkspaceName) -> Result<Option<WorkspaceSettings>, TomeError> { ... }
pub(crate) fn load_global_settings(paths: &Paths) -> Result<GlobalSettings, TomeError> { ... }
```

**Rationale** (R-4-2 / rule-of-3): The three loaders were previously duplicated verbatim across `commands::harness::list`, `harness::sync`, and the MCP server's persona-option startup resolver. Each copy carried the same error classification (`NotFound` → `Ok(None)`, parse error → `WorkspaceMalformed`) and the same reason strings. They live here once, `pub(crate)`, so every consumer resolves the (project, workspace, global) settings triple through a single source of truth. Documented in module-level docs with error-mapping contract. Applied to: `commands/harness/list.rs`, `harness/sync.rs`, `mcp/state.rs::resolve_expose_personas`; verified in `tests/personas_startup_scope.rs` (startup scope resolution from on-disk settings).

### Phase 6 US4: Parallel Prompt Path on the Phase 5 Registry

When exposing agent personas as MCP prompts, use a parallel prompt-builder path alongside Phase 5's command and skill prompts, consuming the same `build_context_for_entry` + substitution/argument pipeline but with persona-specific template wrapping:

```rust
// src/mcp/prompts.rs — persona role variant
pub enum PersonaRole {
    None,            // Phase 5 command/skill
    DropPersona,     // Global drop-persona prompt
    AssumePersona,   // Agent <name>-persona prompt
}

// Persona template wrapping (distinct from command/skill body path)
// Merges agent body with agent name + role-assumption template
// Applies Phase 5 substitution + argument pipeline
// Returns wrapped body for MCP prompt response
```

**Discipline**: Personas join the single collision namespace alongside commands/skills (shared `drop-persona` name reserved, clash-prefix logic extended to personas). The registry's prompt-builder indexes agent rows at startup (same query as the sync path but with different column projection). The persona template-wrapping path forks from the command/skill path at the body-rendering stage, consuming the same `build_context_for_entry` + substitution (no parallel substitution pipeline — NFR-007 preserved). Applied in `src/mcp/prompts.rs` (FR-062 / FR-064); verified in `tests/personas.rs` (prompts/list + prompts/get persona rendering with substitution/arguments) and `tests/mcp_prompts_json_shape.rs::persona_response_is_byte_stable` (T111 wire-shape pin).

### Phase 6 US4: Full Column Projection on Row Queries for Multiple Consumers

When extending a row-projection struct for a new consumer (e.g., personas added to the prompt registry), mirror the sibling query's full column set even if only a subset is locally used:

```rust
// src/index/skills.rs — persona query includes full column set
// SELECT id, name, description, indexed_at, plugin_id, ...
// This mirrors the command/skill query, ensuring personas
// can access plugin_version + indexed_at for collision tie-break
```

**Rationale** (US4 M-1): The persona path needed `plugin_version` + `indexed_at` for collision tie-breaking (matching command/skill behaviour), but early iterations omitted these columns from the persona query, silently degrading substitution + collision logic. Including the full column set ensures future paths consume the same data shape. Applied in `src/index/skills.rs::enabled_agents_for_workspace` (persona query includes plugin_version + indexed_at); verified in `tests/personas_collision.rs` (clash detection + name rendering consistent with commands/skills).

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
| `settings` | Layered settings composition, TOML edit, scalar resolution |
| `harness` | Per-harness module integration, rules-file + MCP-config + agent translation + hooks merge + guardrails |
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

- **Input strictness**: `#[serde(deny_unknown_fields)]` on Tome-owned inputs (including settings structs).
- **Output leniency**: third-party JSON and YAML parsed leniently. Output `Serialize` types do NOT carry `#[serde(deny_unknown_fields)]` (Phase 5: consistency pattern established).
- **Credentials scrubbed**: all error chains + model URLs sanitized before logging.
- **Symlink refusal**: symlink paths refused at every read/write entry point.
- **Path validation**: database-stored paths validated to be relative + `..`-free via `validate_db_stored_path()` before use in `fs::read` (Phase 5 Polish M-4).

---

*This document defines HOW to write code. Update when conventions change or a new pattern reaches 3+ uses.*
