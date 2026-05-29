# Testing Strategy

> **Purpose**: Document test frameworks, patterns, organization, and coverage requirements.
> **Generated**: 2026-05-27
> **Last Updated**: 2026-05-29 (Phase 6 US1 — native agents)

## Test Framework

| Type | Framework | Configuration | Commands |
|------|-----------|---------------|----------|
| Unit | Rust `#[test]` | None (built-in) | `cargo test --lib` |
| Integration | Rust `#[test]` in `tests/` | None (built-in) | `cargo test --test '*'` |
| All | Combined | `.cargo/config.toml` | `cargo test` |

### Running Tests

| Command | Purpose |
|---------|---------|
| `cargo test` | Run all unit + integration tests (uses stub embedder — fast) |
| `cargo test --test catalog_add` | Run one integration test file |
| `cargo test catalog_add::` | Run one test by path |
| `cargo test --test query` | Phase 2 query tests |
| `cargo test --test concurrency` | Two-process index contention |
| `cargo test --test atomicity` | Interrupt-injection tests |

**MSRV tested**: CI runs `cargo +1.93 build` to enforce `rust-version = "1.93"`.

**Pre-push hook** (Phase 5 Polish change): local `cargo fmt`, `cargo clippy`, `typos` checks only. Full `cargo test --workspace` runs in CI as the source of truth (deferred from local pre-push per Phase 5 Polish PR #126 to keep pre-push under ~1 minute on warm cache). Test discipline NOT relaxed — CI matrix is the enforcement surface.

## Test Organization

### Directory Structure

```
tests/
├── *.rs                         # Integration test files (161+ total as of Phase 6 US1)
├── common/
│   ├── mod.rs                   # Shared harness: ToolEnv, Fixture, guards
│   └── ...                      # (exported helpers)
└── fixtures/
    ├── sample-catalog/          # Catalog skeleton (git repo template)
    └── sample-plugin-catalog/   # Plugin skeleton (for lifecycle tests)
```

### Test File Location Strategy

**Separate directory** (`tests/` parallel to `src/`): all integration tests. No co-located unit tests (Rust convention discouraged here because the test binary needs to invoke the CLI and construct real environments).

**Unit tests** within `src/` modules: for pure functions that don't need I/O isolation. Example: `src/config.rs::tests` tests TOML round-tripping.

### Test Categories by File

| Category | Files | Example |
|----------|-------|---------|
| **Catalog commands** | `catalog_*.rs` | `catalog_add.rs`, `catalog_remove.rs` (12 files) |
| **Plugin commands** | `plugin_*.rs` | `plugin_enable.rs`, `plugin_disable.rs` (10 files) |
| **Query & search** | `query.rs`, `entry_*.rs` | Embedding + reranking tests (5 files) |
| **Models & embedding** | `models_*.rs`, `embedding_*.rs` | Download, list, remove (6 files) |
| **Workspace lifecycle** | `workspace_*.rs` | Init, rename, remove, sync (12 files) |
| **Harness integration** | `harness_*.rs` | Use, list, remove, sync (12 files) |
| **Index & schema** | `index_*.rs`, `schema_migration_*.rs` | Database, migrations (7 files) |
| **Doctor & diagnostics** | `doctor_*.rs` | Report, fixes, orphan cleanup (7 files) |
| **MCP server** | `mcp_*.rs` | Server lifecycle, tools, log format (10 files) |
| **Concurrency & atomicity** | `concurrency.rs`, `atomicity.rs` | Lock contention, interrupts (4 files) |
| **Frontmatter & manifests** | `frontmatter*.rs`, `manifest_*.rs` | YAML parsing, strictness (4 files) |
| **Security & hardening** | `security_hardening.rs` | File perms, symlink refusal (1 file) |
| **Error & exit codes** | `exit_codes*.rs`, `error_messages.rs` | Exit code coverage, Display impl (2 files) |
| **Substitution** (Phase 5) | `substitution_*.rs`, `entry_*.rs` | Variable expansion, argument coercion (8 files) |
| **Agent translation** (Phase 6) | `agent_translate_*.rs`, `agent_*.rs` | Per-harness native agents (8 files) |
| **Misc** | `path_validation.rs`, `atomic_dir.rs`, etc. | Phase 1 foundational (10 files) |

**Total**: 161+ test files across 161+ suites; 1200+ tests pass (Phase 6 US1: Phase 6 Foundational 1194 → 1200+, +8 agent test files).

**Phase 6 US1 additions**:
- `tests/agent_translate_claude_code.rs` — Claude Code native-agent translation (MarkdownYaml format, model alias map, dropped field tracking)
- `tests/agent_translate_codex.rs` — Codex native-agent translation (TOML format, triple-quoted developer_instructions, model drop)
- `tests/agent_translate_cursor.rs` — Cursor native-agent translation (MarkdownYaml format, empty model alias drop)
- `tests/agent_translate_opencode.rs` — OpenCode native-agent translation (MarkdownYaml format, fully-qualified model ids, plugin-prefixed names)
- `tests/agent_naming_clash.rs` — Agent name-clash display naming (same filename regardless; displayed name only on clash)
- `tests/agent_removal.rs` — Orphan agent cleanup and per-plugin removal (plugin_of_owned_file ownership split)
- `tests/agent_path_traversal.rs` — Path-traversal defence via is_safe_agent_name (S-1)
- Extensions to `tests/entry_kind_agent_indexing.rs` — Verify `EntryKind::Agent` widening integrates with storage + queries without schema drift
- Extensions to `tests/harness_sync_stub.rs` — Native-agent emit/orphan-removal/idempotence via mtime capture, symlink refusal (exit 7), forward-progress (exit 45), multi-harness fan-out

## Test Patterns

### Test Structure: Arrange-Act-Assert

```rust
#[test]
fn happy_path_human_mode() {
    // Arrange: set up fixture and environment
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();

    // Act: invoke the command
    let out = env
        .cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .expect("spawn");

    // Assert: verify exit code, stdout, state
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Added catalog"));
}
```

### Fixture Pattern: Git-backed Catalog

```rust
pub struct Fixture {
    pub tempdir: TempDir,
    pub repo_path: PathBuf,
    pub url: String,  // file:// URL for cloning
}

impl Fixture {
    pub fn build_sample() -> Self {
        // Copy tests/fixtures/sample-catalog/ into temp dir
        // Run git init && git commit
        // Return handle to the temp repo
    }
}
```

**Used by**: all catalog tests, plugin lifecycle tests, reindex tests.

### Test Environment: ToolEnv

```rust
pub struct ToolEnv {
    pub home: TempDir,
}

impl ToolEnv {
    pub fn new() -> Self {
        // Create isolated $HOME with fresh XDG layout
    }

    pub fn cmd(&self) -> Command {
        // Return a Command for the `tome` binary
        // Pre-populate HOME + XDG_* env vars
        // Suppress logging output
    }
}
```

**Key discipline**: Every test gets its own `ToolEnv`. The host's real `~/.tome/` is never touched because `HOME` is redirected to a `TempDir`.

### Library API Pattern: No CLI Binary

When a test needs to verify library logic without loading real ONNX models, use the library API directly:

```rust
#[test]
fn enable_sets_enabled_flag() {
    let root = TempDir::new().unwrap();
    let paths = lifecycle_paths(root.path());
    let catalog = copy_sample_plugin_catalog(&root, "sample");
    fabricate_models(&paths);

    let embedder = StubEmbedder::new();
    let _guard = EmbedderGuard::install(Arc::new(embedder));

    let id = PluginId::from_str("sample/hello").unwrap();
    let deps = LifecycleDeps { ... };
    let outcome = lifecycle::enable(&id, &deps, false).unwrap();

    assert_eq!(outcome.status, PluginStatus::Enabled);
}
```

**Used by**: plugin lifecycle, reindex, workspace tests (avoid CLI spawn when library API suffices).

### CLI Binary Pattern: Full Integration

When testing the CLI's complete stack (command parsing, output formatting, exit codes), spawn the binary:

```rust
#[test]
fn catalog_add_emits_json_on_flag() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();

    let out = env
        .cmd()
        .args(["catalog", "add", &fix.url, "--json"])
        .output()
        .expect("spawn");

    assert!(out.status.success());
    let v: Value = serde_json::from_slice(&out.stdout).expect("json parse");
    assert_eq!(v["added"]["name"], "sample-experts");
}
```

**Used by**: output formatting tests, exit code coverage, TTY-dependent features like prompts.

### Isolation: `HomeGuard` for `$HOME` Mutations

```rust
#[test]
fn test_reads_home_var() {
    let new_home = TempDir::new().unwrap();
    let _guard = HomeGuard::install(new_home.path());

    // Inside this scope, $HOME is redirected
    assert_eq!(std::env::var("HOME").unwrap(), new_home.path().to_str().unwrap());

    // Test code runs here
}
// _guard drops, HOME is restored, mutex is released
```

**Discipline**: `HomeGuard` holds `HOME_MUTEX` for its lifetime, serializing all tests that mutate `$HOME`. This prevents parallel-test races.

### Phase 5: Test Injection for Time-Dependent Features

When tests verify time-based behavior (e.g., substitution with `$now` variable), use a clock injection guard:

```rust
#[test]
fn substitution_now_returns_fixed_time() {
    let now = time::OffsetDateTime::from_unix_timestamp(1609459200).unwrap();
    let _guard = ClockOverrideGuard::install(now);

    let result = substitution::substitute("built at $now", &ctx);
    assert_eq!(result, "built at 2021-01-01T00:00:00Z");
}
```

**Pattern**: `ClockOverrideGuard` (in `tests/common/mod.rs`) injects via `SUBSTITUTION_CLOCK_OVERRIDE` slot. Drop guard restores real clock. Used in `tests/substitution_*.rs`.

### Phase 5: Test Injection for Data Directory Features

When tests verify plugin or workspace data directory isolation, use data-dir injection guards:

```rust
#[test]
fn plugin_data_dir_isolates_per_plugin() {
    let plugin_root = TempDir::new().unwrap();
    let _guard = PluginDataDirGuard::install(plugin_root.path());

    // Tests now write plugin data to isolated dir, not user's home
    let config = load_plugin_config("my-plugin");
    assert_eq!(config.data_root, plugin_root.path());
}
```

**Patterns**: `PluginDataDirGuard`, `WorkspaceDataDirGuard` (Phase 5 US2 data-model changes).

### Phase 6: Test-Configurable Test Double (StubHarness Builder)

When a test double needs to drive different capability combinations, use the builder pattern with `Default`:

```rust
#[test]
fn harness_with_native_agents_registers_directory() {
    let harness = StubHarness::default()
        .with_native_agents(AgentFormat::MarkdownYaml);
    
    // harness::supports_native_agents() returns true
    // harness::agent_dir() returns Some(<project>/.stub/agents)
    assert!(harness.supports_native_agents());
}

#[test]
fn harness_with_hook_settings_returns_path() {
    let harness = StubHarness::default().with_hook_settings();
    
    // harness::hook_settings_path() returns Some(<project>/.stub/settings.local.json)
    assert!(harness.hook_settings_path(Path::new("/project")).is_some());
}
```

**Pattern** (Phase 6 Foundational F3): `StubHarness` evolved from a unit struct to a `#[derive(Default)]` struct. The `Default` impl produces safe defaults (trait safe defaults for all methods). Builder setters (`with_*`) flip capabilities without spelling out the whole struct. Used in `tests/harness_trait_p6.rs` to exercise hook + agent dispatch paths.

### Phase 6: Direct Per-Harness `translate_agent` Unit Tests

When testing a harness's agent translation without the full CLI/sync stack, call the harness's `translate_agent` method directly:

```rust
// tests/agent_translate_codex.rs
#[test]
fn body_lands_in_triple_quoted_developer_instructions() {
    let agent = read_only_agent();
    let t = CODEX.translate_agent(&agent, false).expect("translate");

    // Parse the rendered TOML and read the value back
    let doc: toml_edit::DocumentMut = t.rendered.parse().expect("parse");
    assert_eq!(
        doc["developer_instructions"].as_str(),
        Some(agent.body.as_str()),
        "developer_instructions holds the body verbatim",
    );
}
```

**Pattern** (Phase 6 US1): Harness modules implement `HarnessModule::translate_agent`, which takes a `CanonicalAgent` and a clash-set boolean. Direct calls avoid spinning up CLI, project markers, sync orchestration — tests remain fast and narrowly focused. Supports quick iteration on format/field-mapping details. Used for per-harness contract coverage.

### Phase 6: Full-Stack Agent Sync Tests via `sync_project` + Override

When testing the complete agent pipeline (enable → index → sync), use the library API with harness override:

```rust
// tests/agent_naming_clash.rs
#[test]
fn clash_applies_plugin_prefix_to_display_name() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(ClaudeCode)]);
    let fx = Fixture::build("test-workspace");

    // Insert two agents with the same `name` from different plugins
    insert_enabled_agent_row(&paths, "test-workspace", "cat", "pluginA", "reviewer", ...);
    insert_enabled_agent_row(&paths, "test-workspace", "cat", "pluginB", "reviewer", ...);

    let outcome = sync::sync_project(&fx.project, &fx.deps()).expect("sync");
    
    // Both files exist on disk with plugin-prefixed display names
    let a_rules = std::fs::read_to_string(&fx.project.join("CLAUDE_CODE_RULES.md"))?;
    assert!(a_rules.contains("name: pluginA-reviewer"));
    assert!(a_rules.contains("name: pluginB-reviewer"));
}
```

**Pattern** (Phase 6 US1): Tests that verify end-to-end agent sync behavior must install a real harness module (not `StubHarness`, which lacks translation semantics), seed agent rows in the index, and call `sync_project`. The `OVERRIDE_MUTEX` serializes concurrent override access. Used for integration-layer tests like clash handling, orphan cleanup, removal.

### Phase 6: Byte-Stable JSON Pins for Agent Dropped-Fields

When testing agent translation, verify the `dropped_fields` vector is recorded correctly and byte-stable:

```rust
// tests/agent_translate_codex.rs
#[test]
fn model_is_dropped_and_recorded() {
    let t = CODEX
        .translate_agent(&read_only_agent(), false)
        .expect("translate");

    // Recorded for the doctor surface
    assert!(
        t.dropped_fields.contains(&"model".to_owned()),
        "dropped model must be recorded; got {:?}",
        t.dropped_fields,
    );
}
```

**Pattern** (Phase 6 US1 / T053 placeholder): Every translation result carries a `dropped_fields: Vec<String>` describing which frontmatter keys were dropped during the field-map. Byte-stable JSON pin tests verify field order and presence for `TranslatedAgent` serialization (when agents are stored in doctor diagnostics).

## Test Data

### Fixtures

**Catalog fixture** (`tests/fixtures/sample-catalog/`):
- Git repo skeleton with `tome-catalog.toml` manifest
- Two sample plugins (`hello`, `goodbye`) with plugin.json manifests
- Copied into temp dir by `Fixture::build_sample()` for each test

**Plugin catalog fixture** (`tests/fixtures/sample-plugin-catalog/`):
- Same structure; used by workspace/lifecycle tests via `copy_sample_plugin_catalog()`

### Fabrication Helpers

| Helper | Purpose | Output |
|--------|---------|--------|
| `fabricate_models(paths)` | Create manifest.json for every model | `~/.tome/models/{name}/manifest.json` |
| `fabricate_installed_models(paths, entries)` | Fabricate model artefact files (sparse) | Manifest + sparse artefact files |
| `fabricate_all_registry_models(paths)` | Fabricate every entry in `MODEL_REGISTRY` | All 3 models (embedder, reranker, summariser) |
| `write_index_db_with_schema_version(path, v)` | Synthetic DB with minimal schema | `/path/index.db` at version `v` |
| `write_config_for_cli(paths, config)` | Seed catalog config + enrol in DB | `config.toml` + `workspace_catalogs` rows |
| `seed_workspace(paths, name)` | Inject workspace row into DB | `workspaces` table entry |

**Sparse file pattern**: `File::set_len(size)` creates zero-filled files that take ~no disk space. Embedder fixture is 66 MB but occupies 0 bytes on disk. SHA-256 mismatch is intentional for `--verify` tests.

## Mocking Strategy

### Stub Embedder (`src/embedding/stub.rs`)

Deterministic embedder that produces fixed vectors based on input. Used in all tests that don't need real inference.

```rust
pub struct StubEmbedder {
    // Produces consistent vectors for the same input
}

#[test]
fn plugin_enable_uses_embedder() {
    let _guard = EmbedderGuard::install(Arc::new(StubEmbedder::new()));
    // Test proceeds with stub instead of loading ONNX models
}
```

**Override mechanism**: `EMBEDDER_OVERRIDE` slot at `src/embedding/mod.rs`, installed via `EmbedderGuard::install()` in `tests/common/mod.rs`.

### Stub Reranker

Similar pattern to embedder; deterministic ranking by vector sum.

### Stub Summariser (Phase 4)

Deterministic text summarization (returns fixed text) instead of loading Qwen2.5 model. Override via `SUMMARISER_OVERRIDE` slot.

### Stub Harness (Phase 6)

Test-only deterministic harness implementation in `src/harness/stub.rs`, configurable via builder pattern. Override via `HARNESS_MODULES_OVERRIDE` slot.

### Test-Only Injection Points

| Slot | Override Guard | Used For |
|------|----------------|----------|
| `EMBEDDER_OVERRIDE` | `EmbedderGuard` | Stub embedder in tests |
| `RERANKER_OVERRIDE` | `RerankerGuard` | Stub reranker in tests |
| `SUMMARISER_OVERRIDE` | `SummariserOverrideGuard` | Stub summariser (Phase 4) |
| `HARNESS_MODULES_OVERRIDE` | `HarnessModulesGuard` | Synthetic harness registry (Phase 6) |
| `MIGRATIONS_OVERRIDE` | `MigrationsGuard` | Synthetic schema migrations |
| `SUBSTITUTION_CLOCK_OVERRIDE` | `ClockOverrideGuard` | Fixed system clock (Phase 5) |
| `PLUGIN_DATA_DIR_OVERRIDE` | `PluginDataDirGuard` | Plugin data directory (Phase 5) |
| `WORKSPACE_DATA_DIR_OVERRIDE` | `WorkspaceDataDirGuard` | Workspace data directory (Phase 5) |

All defined in `tests/common/mod.rs` with RAII drop guards.

## Coverage Requirements

| Metric | Target | Current | Notes |
|--------|--------|---------|-------|
| Exit codes | All enumerated variants | ✓ | `tests/exit_codes.rs` grep guard |
| CLI binary paths | Representative sampling | ✓ | Exit codes + output format tested |
| Library API | 100% on public surface | ✓ | Unit tests in modules |
| Error Display | All variants | ✓ | `tests/error_messages.rs` |
| JSON wire shapes | Byte-stable pins | ✓ | `tests/*_json_shape.rs` (Phase 4+) |
| Agent translation | Per-harness contract | ✓ | `tests/agent_translate_*.rs` (Phase 6 US1) |

**Exclusions**: ONNX inference (real model load excluded; library `fastembed` tests own path), real model downloads (fabricated fixtures instead), MCP protocol purity (deferred T093–T095).

**Phase 6 US1**: Exit codes 43–46 (Phase 6 hooks + agents) are covered in `tests/exit_codes.rs`. New JSON wire shape `TranslatedAgent` with `dropped_fields` pinned in per-harness translation tests. Agent indexing integrated with `EntryKind::Agent` variant verified in `tests/entry_kind_agent_indexing.rs`. Agent removal (orphan + per-plugin) logic tested in `tests/agent_removal.rs`. Path-traversal defence (S-1) verified in `tests/agent_path_traversal.rs`.

## Test Categories by Purpose

### Smoke Tests

Critical path tests that must pass before deploy:

| Test | Purpose |
|------|---------|
| `catalog_add.rs::happy_path_human_mode` | Core catalog registration flow |
| `plugin_enable.rs::happy_path_json_mode` | Core plugin enable flow |
| `query.rs::happy_path` | Core search + ranking flow |
| `workspace_use.rs::happy_path` | Core project binding flow |
| `doctor.rs::assemble_report_happy_path` | Core diagnostic flow (Phase 5 US5) |

### Regression Tests

Tests for previously fixed bugs, linked to phase retros:

| Category | Retro | Example |
|----------|-------|---------|
| Phase 4 US1 | `retro/P3.md` | `sync_idempotence.rs` (Sync twice → no changes) |
| Phase 4 US3 | `retro/P5.md` | `workspace_commands.rs` (Scope isolation) |
| Phase 5 US3 | `retro/P5.md` | `entry_kind_indexing.rs` (Entry kind + collision handling) |
| Phase 5 US5 | (current) | `doctor_phase5_surface_creates_no_dirs` (FR-124 read-only invariant) |
| Phase 6 Foundational F2 | (current) | `entry_kind_agent_indexing.rs` (Agent row integration; schema drift prevention) |
| Phase 6 US1 S-1 | (current) | `agent_path_traversal.rs` (Index-time gate blocks ../../../../tmp/evil) |

### Invariant Tests

Tests that verify core properties hold:

| Property | Test File | Checks |
|----------|-----------|--------|
| Manifest strictness | `manifest_strictness.rs` | All Tome-owned types have `#[serde(deny_unknown_fields)]` |
| Exit code completeness | `exit_codes.rs` | All `TomeError` variants are covered |
| Syncability | `sync_idempotence.rs` | Harness sync is idempotent |
| Atomicity | `atomicity.rs` | Partial failures leave committed state |
| JSON wire shape | `*_json_shape.rs` | Serialization is deterministic + byte-stable |
| Read-only invariant | `doctor_p5.rs` | `doctor assemble_report` creates no directories (Phase 5 US5.a) |
| Exact-count pins | `plugin_show_p5.rs`, `doctor_p5.rs`, `doctor_json.rs` | Deterministic fixture counts stay exact (Phase 5 Polish + Phase 6) |
| Canonical enum dispatch | `entry_kind_agent_indexing.rs` | Exhaustive match on `EntryKind` surfaces schema drift (Phase 6 F2) |
| Marker migration | `schema_migration_p6.rs` | Version bump advances without DDL (Phase 6 Foundational) |
| Filename provenance | `agent_removal.rs` | `<plugin>__<name>` is the sole provenance rule (Phase 6 US1) |
| Agent embedding skip | `entry_kind_agent_indexing.rs` | Agent rows are never embedded; queries filter on `embedding IS NOT NULL` (Phase 6 US1) |
| Path-traversal defence | `agent_path_traversal.rs` | Attacker-controlled `name: ../../../../tmp/evil` rejected at index time (S-1) |
| Display name clash | `agent_naming_clash.rs` | Two agents with same `<name>` show plugin-prefixed display names (FR-041) |

### Phase 5: Truncation Boundary Tests

Tests for string truncation edge cases (US4.d + Polish M-1 pattern):

| Test | Checks |
|------|--------|
| `mcp_tool_description.rs::truncate_respects_char_boundaries_with_emoji()` | Multi-byte UTF-8 char slicing |
| `mcp_search_skills_truncation.rs::truncation_at_multibyte_char_boundary_does_not_split_codepoint()` | Emoji boundaries (Polish M-1) |
| `entry_kind_*.rs::search_skills_description_truncation_*()` | Description max-length enforcement |
| `substitution_*.rs::argument_value_truncation_boundary()` | Argument coercion with limits |

### Phase 5: Exact-Count + Empty-Section Invariant Tests

Tests that verify deterministic entity counts and collection states (US5.b + Polish patterns):

| Test | Checks | Pattern |
|------|--------|---------|
| `plugin_show_p5.rs::dormant_entry_annotated()` | Dormant bit set correctly | Positive assertion |
| `plugin_show_p5.rs::dormant_not_annotated_when_searchable_true()` | Boolean-logic negative case (T-G1) | Explicit "NOT" test |
| `doctor_p5.rs::empty_section_arrays_present_not_omitted()` | Empty arrays serialize; not omitted (T-G2) | Presence invariant |
| `doctor_json.rs::entry_counts_by_kind_exact_match()` | Exact skill/command/agent counts match fixture (T-W1) | Exact-count discipline |
| `doctor_p5.rs::pending_re_embedding_zero_when_no_files_touched()` | Zero re-embeds when nothing changed (GAP-2, Polish) | Zero-state assertion |

**Rationale** (Polish phase learnings): The zero-state and empty-section invariant tests catch "off-by-one forgot to reset" bugs. Phase 5 Polish T-W1 introduced the pattern; now applied to pending counts and empty arrays. Together with positive tests, this three-case coverage (positive/negative/zero/empty) becomes the canonical pattern for deterministic fixtures.

### Phase 6: Agent Translation Contract Tests

Tests that verify each harness's agent format and field-mapping contract (US1):

| Test | Harness | Checks |
|------|---------|--------|
| `agent_translate_claude_code.rs::body_lands_in_frontmatter()` | Claude Code | MarkdownYaml format; frontmatter keys; model pass-through |
| `agent_translate_codex.rs::body_lands_in_triple_quoted_developer_instructions()` | Codex | TOML format; developer_instructions triple-quote; model DROP |
| `agent_translate_cursor.rs::format_and_filename_match_contract()` | Cursor | MarkdownYaml format; model DROP (no alias) |
| `agent_translate_opencode.rs::model_maps_to_qualified_anthropic_id()` | OpenCode | MarkdownYaml format; `opus` → `anthropic/claude-opus-4.7`; display name override |

**Pattern** (Phase 6 US1): Each harness has a `translate_agent` test file verifying the contract (`contracts/agent-translation.md` SC-001 row). Direct calls to `HarnessModule::translate_agent` with hand-crafted `CanonicalAgent` fixtures. Tests verify:
- Format (MarkdownYaml or Toml)
- Filename (always `<plugin>__<name>.<ext>`)
- Body placement (frontmatter vs triple-quoted)
- Model mapping (same-vendor-only)
- Dropped-fields vector
- Read-only inference (tools → sandbox_mode or not)

## CI Integration

### Test Pipeline (`.github/workflows/*`)

- Unit tests (parallel)
- Integration tests (parallel, with stub embedder)
- Binary size check (`target/release/tome` <= 50 MB)
- Clippy strict linting
- rustfmt check
- typos check

### Required Checks

| Check | Blocking | Runs On |
|-------|----------|---------|
| `cargo test` | Yes (main) | Every PR |
| `cargo clippy` | Yes | Every PR |
| `cargo fmt --check` | Yes | Every commit hook |
| `typos` | Yes | Every commit hook |
| Binary size | Yes (main) | Linux x86_64 |
| MSRV | Yes | CI only |

### Pre-Commit Hook

`.githooks/pre-commit` runs `cargo fmt --check`, `typos`, and `cargo clippy` sequentially. All three must pass before commit succeeds (no `--no-verify` bypasses without documented reason).

### Pre-Push Hook (Phase 5 Polish Change)

**Phase 5 Polish PR #126**: Pre-push hook now runs **local fmt/clippy/typos checks ONLY** (no full `cargo test --workspace`). Rationale: pre-push completes under ~1 minute on warm cache, staying responsive for local iteration. Full test suite runs in CI (GitHub matrix across Linux/macOS) as the source of truth. Test discipline is NOT relaxed — CI is the enforcement surface.

## Test Discipline

### One Assertion Per Test

Each test verifies one behavior. Related assertions on the same outcome are grouped, but independent checks get separate tests.

```rust
// Good: one concept per test
#[test]
fn catalog_add_success_updates_config() { ... }

#[test]
fn catalog_add_duplicate_exits_4() { ... }

// Bad: mixing multiple concerns
#[test]
fn catalog_add_works() {
    // Assert success
    // Assert config updated
    // Assert cache cloned
    // Assert manifest parsed
}
```

### Test Names

Descriptive, underscore-separated. Format: `{subject}_{action}_{expectation}`.

```rust
#[test]
fn catalog_add_duplicate_registration_exits_4() { ... }

#[test]
fn plugin_enable_missing_models_prompts_download() { ... }

#[test]
fn harness_use_composition_error_exits_17() { ... }

#[test]
fn doctor_p5_surface_creates_no_dirs() { ... }

#[test]
fn entry_kind_agent_injected_rows_counted_correctly() { ... }

#[test]
fn agent_path_traversal_rejected_at_index_time() { ... }
```

### Minimal External I/O

- **Git**: real repo fabrication via `Fixture` (necessary for catalog tests).
- **HTTP**: none (no real downloads; fixtures or error paths).
- **Filesystem**: all under TempDir (no host state pollution).
- **ONNX models**: stub inference only (no real model load in test suite).
- **Time**: fixed via `ClockOverrideGuard` when needed (Phase 5).

### Deterministic Execution

- No flaky sleeps or timeouts.
- Stub embedder produces fixed vectors for deterministic test assertions.
- Concurrent tests serialized via `HOME_MUTEX` + RAII guards.
- No real time dependencies (fixed clock via `ClockOverrideGuard`).

### Phase 5: 4-Reviewer Parallel Pass Pattern

**Phase 5 Polish introduces this pattern at PHASE-WIDE scope** (distinct from per-US passes). Every Phase 5+ closeout runs a parallel 4-reviewer pass **once at the end** rather than per-user-story:

| Reviewer | Focus | Deliverable |
|----------|-------|-------------|
| Contract audit | Spec alignment, cross-US drift, contract amendments | `review/findings.md` + `review/disposition.md` |
| Rust-lens | Code review, idioms, safety, cross-US patterns | Inline code comments, M-1/M-2/M-3/M-4 fixes |
| Test audit | Coverage gaps, edge cases, invariant tests | Test additions (GAP-2, Polish M-1 truncation) |
| Security audit | Hardening, boundary validation, no new vectors | Security findings, deferred items |

Findings + disposition committed **BEFORE** fixes land (Phase 5 Polish PR pattern). Exemplified in Polish Polish: "4-reviewer pass surfaced 0 BLOCKERS + 4 majors + 1 test gap + 5 minors; applied 2 majors + 1 test."

**Impact**: Phase-wide passes catch cross-US drift earlier than per-US passes can. Emerged at Phase 5 Polish as the structural-safety net for multi-user-story phases.

## What Does NOT Belong Here

- Code style rules → `CONVENTIONS.md`
- Security testing → `SECURITY.md`
- Architecture patterns → `ARCHITECTURE.md`

---

*This document describes HOW to test. Update when testing strategy changes or a new pattern emerges.*
