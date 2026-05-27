# Testing Strategy

> **Purpose**: Document test frameworks, patterns, organization, and coverage requirements.
> **Generated**: 2026-05-27
> **Last Updated**: 2026-05-27

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

## Test Organization

### Directory Structure

```
tests/
├── *.rs                         # Integration test files (144 total as of Phase 5)
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
| **Index & schema** | `index_*.rs`, `schema_migration_*.rs` | Database, migrations (6 files) |
| **Doctor & diagnostics** | `doctor_*.rs` | Report, fixes, orphan cleanup (7 files) |
| **MCP server** | `mcp_*.rs` | Server lifecycle, tools, log format (10 files) |
| **Concurrency & atomicity** | `concurrency.rs`, `atomicity.rs` | Lock contention, interrupts (4 files) |
| **Frontmatter & manifests** | `frontmatter*.rs`, `manifest_*.rs` | YAML parsing, strictness (4 files) |
| **Security & hardening** | `security_hardening.rs` | File perms, symlink refusal (1 file) |
| **Error & exit codes** | `exit_codes*.rs`, `error_messages.rs` | Exit code coverage, Display impl (2 files) |
| **Substitution** (Phase 5) | `substitution_*.rs` | Variable expansion, argument coercion (6 files) |
| **Misc** | `path_validation.rs`, `atomic_dir.rs`, etc. | Phase 1 foundational (10 files) |

**Total**: 144 test files across 127 suites (some files define multiple test functions); 954+ tests pass.

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

### Test-Only Injection Points

| Slot | Override Guard | Used For |
|------|----------------|----------|
| `EMBEDDER_OVERRIDE` | `EmbedderGuard` | Stub embedder in tests |
| `RERANKER_OVERRIDE` | `RerankerGuard` | Stub reranker in tests |
| `SUMMARISER_OVERRIDE` | `SummariserOverrideGuard` | Stub summariser (Phase 4) |
| `HARNESS_MODULES_OVERRIDE` | `HarnessModulesGuard` | Synthetic harness registry |
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

**Exclusions**: ONNX inference (real model load excluded; library `fastembed` tests own path), real model downloads (fabricated fixtures instead), MCP protocol purity (deferred T093–T095).

## Test Categories by Purpose

### Smoke Tests

Critical path tests that must pass before deploy:

| Test | Purpose |
|------|---------|
| `catalog_add.rs::happy_path_human_mode` | Core catalog registration flow |
| `plugin_enable.rs::happy_path_json_mode` | Core plugin enable flow |
| `query.rs::happy_path` | Core search + ranking flow |
| `workspace_use.rs::happy_path` | Core project binding flow |

### Regression Tests

Tests for previously fixed bugs, linked to phase retros:

| Category | Retro | Example |
|----------|-------|---------|
| Phase 4 US1 | `retro/P3.md` | `sync_idempotence.rs` (Sync twice → no changes) |
| Phase 4 US3 | `retro/P5.md` | `workspace_commands.rs` (Scope isolation) |
| Phase 5 US3 | (current) | `entry_kind_indexing.rs` (Entry kind + collision handling) |

### Invariant Tests

Tests that verify core properties hold:

| Property | Test File | Checks |
|----------|-----------|--------|
| Manifest strictness | `manifest_strictness.rs` | All Tome-owned types have `#[serde(deny_unknown_fields)]` |
| Exit code completeness | `exit_codes.rs` | All `TomeError` variants are covered |
| Syncability | `sync_idempotence.rs` | Harness sync is idempotent |
| Atomicity | `atomicity.rs` | Partial failures leave committed state |
| JSON wire shape | `*_json_shape.rs` | Serialization is deterministic + byte-stable |

### Phase 5: Truncation Boundary Tests

Tests for string truncation edge cases (US4.d pattern):

| Test | Checks |
|------|--------|
| `mcp_tool_description.rs::truncate_respects_char_boundaries_with_emoji()` | Multi-byte UTF-8 char slicing |
| `entry_kind_*.rs::search_skills_description_truncation_*()` | Description max-length enforcement |
| `substitution_*.rs::argument_value_truncation_boundary()` | Argument coercion with limits |

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

## What Does NOT Belong Here

- Code style rules → `CONVENTIONS.md`
- Security testing → `SECURITY.md`
- Architecture patterns → `ARCHITECTURE.md`

---

*This document describes HOW to test. Update when testing strategy changes or a new pattern emerges.*
