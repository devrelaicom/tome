# Testing Strategy

> **Purpose**: Document test frameworks, patterns, organization, and coverage requirements.
> **Generated**: 2026-05-26
> **Last Updated**: 2026-05-26

## Test Framework

| Type | Framework | Configuration |
|------|-----------|---------------|
| Unit | Rust built-in (`#[test]`) | Implicit in `src/lib.rs` + `src/**/*.rs` |
| Integration | Rust built-in (`#[test]`) | Files in `tests/` directory |
| E2E | Not currently in use | N/A |

Cargo automatically discovers and runs all tests via `cargo test`. No external test runner needed.

### Running Tests

| Command | Purpose |
|---------|---------|
| `cargo test` | Run all tests (unit + integration) |
| `cargo test --lib` | Unit tests only |
| `cargo test --test <name>` | Single integration test file |
| `cargo test <pattern>::` | Tests matching the pattern |
| `cargo test -- --nocapture` | Show stdout/stderr (suppress output capture) |
| `cargo test -- --test-threads=1` | Run sequentially (for thread-local state or shared resource tests) |

**Phase 4 US4 Status**: 862 passing tests, 16 ignored, across 117 test suites:
- ~135 unit tests in `src/lib.rs` + modules
- ~727 integration tests in `tests/*.rs`

## Test Organization

### Directory Structure

```
tests/
├── common/mod.rs                                  # Shared test harness (Fixture, ToolEnv, helpers, guards)
│   ├── Fixture, ToolEnv, paths_for, global_scope()
│   ├── HomeGuard, HarnessModulesGuard (RAII guards for env + injection)
│   ├── NamedStubHarness for synthetic harness composition
│   └── seed_workspace(), seed_project(), write_index_db_with_schema_version()
├── atomic_dir.rs                                  # Atomic directory landing tests
├── atomicity.rs                                   # Atomic writes under SIGINT injection
├── atomicity_enable.rs                            # Plugin enable atomicity (thread-local state)
├── catalog_add.rs                                 # Catalog add with fixtures
├── catalog_list.rs                                # CLI binary list smoke tests
├── catalog_remove.rs                              # Remove without cascade
├── catalog_remove_cascade.rs                      # Remove with --force cascade-disable
├── catalog_show.rs                                # Show single catalog
├── catalog_update.rs                              # Catalog update (git sync)
├── catalog_update_cross_workspace_reindex.rs      # Cross-workspace reindex
├── catalog_update_reindex.rs                      # Catalog update triggering reindex
├── catalog_workspace_refcount.rs                  # Shared clone reference counting across workspaces
├── concurrency.rs                                 # Cross-process index contention
├── doctor.rs                                      # Doctor assemble + --fix tests
├── doctor_json.rs                                 # Doctor JSON envelope shape
├── embedding_stub.rs                              # StubEmbedder determinism
├── error_messages.rs                              # TomeError Display format
├── exit_codes.rs                                  # Exit code mappings (library API)
├── exit_codes_e2e.rs                              # CLI binary exit codes
├── frontmatter.rs                                 # SKILL.md frontmatter parsing
├── harness_bare.rs                                # Bare `tome harness` (Phase 4 US3)
├── harness_info.rs                                # `tome harness info` output (Phase 4 US3)
├── harness_json_shape.rs                          # JSON envelope shape (Phase 4 US3)
├── harness_list_as_written.rs                     # Declared harnesses (Phase 4 US3)
├── harness_list_effective.rs                      # Effective harness list (Phase 4 US3)
├── harness_module_claude_code.rs                  # Claude Code production harness (Phase 4 US1)
├── harness_modules.rs                             # Harness discovery + composition (Phase 4)
├── harness_remove_scope.rs                        # Remove harness from scope (Phase 4 US3)
├── harness_skeleton.rs                            # Harness module trait + stubs (Phase 4)
├── harness_sync.rs                                # Sync command smoke test (Phase 4 US3)
├── harness_sync_stub.rs                           # Sync algorithm with StubHarness (Phase 4)
├── harness_use_scope.rs                           # Add harness to scope (Phase 4 US3)
├── index_lock.rs                                  # Advisory lock contention
├── index_schema_bootstrap.rs                      # DB schema bootstrap
├── manifest_strictness.rs                         # Strictness boundary (#[serde(deny_unknown_fields)])
├── mcp_config_clash.rs                            # MCP config ownership clash detection
├── mcp_config_create.rs                           # MCP config creation (TOML + JSON)
├── mcp_config_preserve_order.rs                   # Order/comment preservation on rewrite
├── mcp_config_remove.rs                           # MCP config entry removal
├── mcp_config_update.rs                           # MCP config update (read-modify-write)
├── mcp_lifecycle.rs                               # MCP server startup paths
├── mcp_log_format.rs                              # MCP log JSON field names
├── mcp_server.rs                                  # MCP tool routing + schemas
├── mcp_tool_description.rs                        # MCP tool descriptions with summariser (Phase 4 US4)
├── model_download.rs                              # Model download mid-stream abort
├── models_download.rs                             # CLI models download
├── models_list.rs                                 # CLI models list
├── models_remove.rs                               # CLI models remove
├── no_directories_imports.rs                      # Verify directories crate dropped (Phase 4)
├── no_phase3_paths.rs                             # Verify phase3-era paths removed (Phase 4)
├── path_validation.rs                             # XDG path resolution
├── paths_phase2.rs                                # Phase 2 per-scope paths
├── paths_phase3.rs                                # Phase 3 workspace paths
├── plugin_cheap_reenable_across_workspaces.rs     # Cheap re-enable idempotency (Phase 4)
├── plugin_disable.rs                              # Plugin disable CLI
├── plugin_enable.rs                               # Plugin enable library API
├── plugin_interactive.rs                          # Bare `tome plugin` via pty (rexpect)
├── plugin_list.rs                                 # Plugin list CLI
├── plugin_repeated.rs                             # Enable/disable of already-enabled
├── plugin_show.rs                                 # Plugin show CLI
├── plugin_summariser_forward_progress.rs          # Summariser byte-progress callback (Phase 4 US4)
├── plugin_workspace_skills.rs                     # Workspace-scoped skills isolation (Phase 4)
├── query.rs                                       # KNN + reranker + drift
├── reindex.rs                                     # Reindex command
├── rules_file_block_in_existing.rs                # Rules file injection into existing file (Phase 4 US1)
├── rules_file_standalone.rs                       # Standalone rules file (Phase 4 US1)
├── schema_migration_e2e.rs                        # Forward schema migrations (synthetic)
├── schema_migrations.rs                           # Schema version guards
├── scrubbing.rs                                   # Credential scrubbing in errors
├── security_hardening.rs                          # Hardening measures (mode preservation, symlink skip)
├── settings_array_types.rs                        # Array-type settings validation (Phase 4 US3)
├── settings_bad_exclusion.rs                      # Unsupported harness validation (Phase 4 US3)
├── settings_composition.rs                        # Settings layering validation (Phase 4 US3)
├── settings_composition_resolves_to_as_written.rs # Composition semantics (Phase 4 US3)
├── settings_cycle_detection.rs                    # Circular workspace refs (Phase 4 US3)
├── settings_harness_not_supported.rs              # Per-entry validation (Phase 4 US3)
├── settings_priority.rs                           # Layer priority (Phase 4 US3)
├── settings_skeleton.rs                           # Settings composition skeleton (Phase 4)
├── settings_unknown_workspace_resolver.rs         # Workspace resolution errors (Phase 4 US3)
├── settings_workspace_ref_outside_project.rs      # Ref constraints (Phase 4 US3)
├── status.rs                                      # Status report assembly
├── summariser_cache.rs                            # Cache hit/miss semantics (Phase 4 US4)
├── summariser_forward_progress.rs                 # Byte-progress callback (Phase 4 US4)
├── summariser_real.rs                             # Real Qwen model tests (env-gated, Phase 4 US4)
├── summariser_registry_no_placeholder.rs          # Registry placeholder regression (Phase 4 US4)
├── summariser_stub.rs                             # StubSummariser determinism (Phase 4 US4)
├── summariser_triggers.rs                         # Trigger-wiring coverage (Phase 4 US4)
├── summariser_triggers_end_to_end.rs              # End-to-end trigger tests (Phase 4 US4)
├── sync_algorithm.rs                              # Sync orchestrator (Phase 4 US1)
├── sync_boundary.rs                               # Enforce tokio confinement to src/mcp/
├── sync_idempotence.rs                            # Idempotence-by-mtime for sync primitives (Phase 4)
├── version_output.rs                              # `tome --version` formats
├── workspace_commands.rs                          # Scope isolation across commands
├── workspace_info.rs                              # Workspace info report
├── workspace_init.rs                              # Workspace init atomicity
├── workspace_list.rs                              # Workspace list command (Phase 4 US2)
├── workspace_name.rs                              # WorkspaceName validation
├── workspace_remove.rs                            # Workspace remove with cascade (Phase 4 US2)
├── workspace_remove_cascade.rs                    # Workspace remove catalog refcount cleanup (Phase 4 US2)
├── workspace_rename.rs                            # Workspace rename command (Phase 4 US2)
├── workspace_regen_summary.rs                     # Regen summary command (Phase 4 US2)
├── workspace_resolution.rs                        # Workspace scope resolution algorithm
├── workspace_sync.rs                              # Workspace sync command (Phase 4 US2)
├── workspace_use_atomicity.rs                     # Project binding atomicity (Phase 4 US1)
├── workspace_use_binding.rs                       # Core project-binding flow (Phase 4 US1)
├── workspace_use_claude_code_e2e.rs               # Claude Code harness integration (Phase 4 US1)
├── workspace_use_concurrent.rs                    # Concurrent workspace use (Phase 4 US1)
├── workspace_use_cross_product.rs                 # Cross-product coverage (Phase 4 US1)
├── workspace_use_forward_progress.rs              # Forward progress validation (Phase 4 US1)
└── workspace_use_json_shape.rs                    # JSON envelope shape pinning (Phase 4 US1)

fixtures/
├── sample-catalog/                                # Git fixture with plugin-alpha + plugin-beta
└── sample-plugin/                                 # (future expansion)
```

### Test File Naming

Integration test files follow the pattern `<command-or-feature>_<suffix>.rs`:
- `plugin_enable.rs` — plugin enable feature
- `catalog_add.rs` — catalog add subcommand
- `workspace_init.rs` — workspace init subcommand
- `workspace_use_binding.rs` — workspace use project binding
- `harness_bare.rs` — bare `tome harness` interactive browse (Phase 4 US3)
- `harness_list_effective.rs` — effective harness list resolution (Phase 4 US3)
- `settings_composition.rs` — settings layer composition (Phase 4 US3)
- `summariser_triggers.rs` — summariser trigger wiring (Phase 4 US4)
- `summariser_cache.rs` — cached summary hit/miss (Phase 4 US4)
- `schema_migration_e2e.rs` — end-to-end synthetic migrations
- `manifest_strictness.rs` — cross-cutting strictness boundary
- `sync_idempotence.rs` — sync idempotence verification

## Test Patterns

### Unit Tests (in src/lib.rs + modules)

Unit tests verify individual functions and small APIs without external dependencies:

```rust
#[test]
fn parse_plugin_identity_accepts_catalog_slash_plugin() {
    let id: PluginId = "my-catalog/my-plugin".parse().unwrap();
    assert_eq!(id.catalog, "my-catalog");
    assert_eq!(id.plugin, "my-plugin");
}

#[test]
fn parse_workspace_name_validates_at_boundary() {
    let name = WorkspaceName::parse("my-workspace").unwrap();
    assert_eq!(name.as_str(), "my-workspace");
    
    let err = WorkspaceName::parse("my-workspace-").unwrap_err();
    assert!(matches!(err, TomeError::WorkspaceNameInvalid { .. }));
}
```

**Characteristics**:
- No I/O, no temp directories, no `Command` spawning
- Fast (~2–5 ms per test)
- Run with `cargo test --lib`

### Integration Tests (in tests/*.rs)

Integration tests use the library API with isolated state (temp directories, fake catalogs, stub embedders):

```rust
#[test]
fn enable_inserts_skill_rows_with_content_hash_and_enabled_flag() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);

    let embedder = StubEmbedder::new();
    let deps = LifecycleDeps {
        paths: &paths,
        scope: &tome::workspace::Scope(tome::workspace::WorkspaceName::global()),
        config: &config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    };

    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();
    let outcome = lifecycle::enable(&id, &deps).expect("enable should succeed");
    assert_eq!(outcome.summary.total_skills, 4);
}
```

**Characteristics**:
- Use `TempDir` for isolated state
- Call library functions directly (not via CLI binary)
- Pass `StubEmbedder` / `StubReranker` / `StubSummariser` to avoid loading real models
- Verify outcomes via assertions on returned structs and on-disk state
- Run with `cargo test` or `cargo test --test plugin_enable`

### CLI Binary Tests

Some tests spawn the `tome` binary as a child process to verify exit codes, argument parsing, and non-TTY refusals:

```rust
#[test]
fn plugin_disable_non_tty_without_force_refuses() {
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    // ... setup ...

    let output = Command::new(bin_path())
        .args(&["plugin", "disable", "catalog/plugin"])
        .env("HOME", env.home_path())
        .output()
        .expect("spawn");

    assert_eq!(output.status.code(), Some(54));  // NotATerminal
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("requires a terminal"));
}
```

**Characteristics**:
- Use `ToolEnv` to isolate `$HOME` / `$XDG_*` env vars
- Spawn `Command::new(bin_path())` (from `env!("CARGO_BIN_EXE_tome")`)
- Verify exit codes and stdout/stderr content
- Avoid real model loading (use `--force` to skip prompts, or test error paths)
- Run alongside other integration tests; separate by feature

### Silent-Compute + Emit-Wrapper Tests (Phase 4 US3)

Tests for CLI commands using the new two-function pattern call the silent compute function (`assemble_*`) and assert on the return value, avoiding stdout emission:

```rust
#[test]
fn harness_info_returns_correct_outcome_shape() {
    // Setup: workspace + harness scope
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    seed_workspace(&paths, "my-workspace");
    
    // Call the silent compute function (tested in isolation)
    let outcome = commands::harness::info::assemble(
        args,
        &scope,
        &paths
    ).expect("assemble must succeed");
    
    // Assert on the outcome struct
    assert_eq!(outcome.name.as_str(), "my-workspace");
    assert!(outcome.is_bound);
}
```

The CLI dispatcher (`run`) is tested separately in CLI binary tests (for exit codes / TTY behavior). Splitting compute from emit improves testability and allows MCP to reuse the compute path.

### Interactive CLI Tests (via rexpect)

The `plugin_interactive.rs` test file uses `rexpect` to drive a pty harness:

```rust
#[test]
fn bare_plugin_navigates_catalog_plugin_view_loop() {
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    // ... setup ...

    let mut p = PtySession::new(
        Command::new(bin_path())
            .arg("plugin")
            .env("HOME", env.home_path())
    ).expect("spawn pty");

    p.read_until("Catalogs").expect("prompt");
    p.write_line("sample-plugin-catalog").unwrap();
    p.read_until("Plugins").expect("prompt");
    p.write_line("plugin-alpha").unwrap();
    p.read_until("skill-a").expect("skill list");
    p.write_line("q").unwrap();
}
```

**Characteristics**:
- Uses `rexpect = "0.7"` (Unix-only)
- Drives the CLI through interactive prompts
- Only for tests where prompts are central to the feature
- `rexpect` is a dev-dependency; not in the release binary

### Summariser Tests (Phase 4 US4)

Tests for the workspace summariser verify trigger wiring, caching, and stub determinism:

```rust
#[test]
fn summariser_fires_after_enable() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    let ws = WorkspaceName::parse("mine").unwrap();
    workspace::init::init(ws.clone(), false, &paths).unwrap();

    let stub = StubSummariser::new();
    let stub_arc: Arc<dyn Summariser> = Arc::new(stub.clone());
    let _guard = SummariserOverrideGuard::install(stub_arc);

    let deps = LifecycleDeps { /* ... */ };
    lifecycle::enable(&id, &deps).expect("enable");

    // Summariser was invoked once after enable
    assert_eq!(stub.call_count(), 1);
    
    // _guard drops here, clearing SUMMARISER_OVERRIDE
}
```

**Characteristics**:
- Use `SummariserOverrideGuard::install()` to inject a test summariser (Phase 4 US4.b pattern)
- Verify call count via `stub.call_count()` (atomic via `Cell`)
- Assert on cached summary fields in `settings.toml`
- Both guard and stub handle must be held for the lifetime of the test

Pattern mirrors `MigrationsGuard` (schema tests) and `HarnessModulesGuard` (harness composition tests).

### Atomic-Directory Landing Tests (Phase 4)

Tests for `src/util/atomic_dir.rs` verify crash safety:

```rust
#[test]
fn land_directory_is_atomic_on_sigint_before_keep() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("marker");
    
    // Simulate SIGINT mid-populate by returning Err
    let result = atomic_dir::land_directory(&target, 0o700, |staged| {
        // Partial write, then error
        std::fs::File::create(staged.join("partial.txt"))?;
        Err(TomeError::Interrupted)
    });
    
    // Target must not exist; staging dir cleaned by TempDir::drop
    assert!(result.is_err());
    assert!(!target.exists());
}
```

### Idempotence-by-Mtime Tests (Phase 4)

Tests for rules files and MCP configs verify that repeated writes with identical content don't change mtime:

```rust
#[test]
fn mcp_config_write_preserves_mtime_on_idempotent_rewrite() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.json");
    
    // First write
    let entry = TomeEntry::new("tome".to_string(), vec!["mcp".to_string()]);
    mcp_config::write_json(&config_path, &entry).unwrap();
    let mtime1 = std::fs::metadata(&config_path).unwrap().modified().unwrap();
    
    // Sleep to ensure distinct granularity
    std::thread::sleep(Duration::from_millis(1500));
    
    // Identical rewrite
    mcp_config::write_json(&config_path, &entry).unwrap();
    let mtime2 = std::fs::metadata(&config_path).unwrap().modified().unwrap();
    
    assert_eq!(mtime1, mtime2, "idempotent write should not change mtime");
}
```

### Settings Composition Tests (Phase 4 US3)

Tests for `settings::resolver::resolve_effective_list` verify the three-layer priority stack:

```rust
#[test]
fn resolve_effective_list_prioritizes_project_over_workspace_over_global() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    
    // Global declares `["a", "b"]`
    write_global_settings(&paths, "harnesses = [\"a\", \"b\"]").unwrap();
    
    // Workspace declares `["x", "y"]` (overrides global)
    write_workspace_settings(&paths, "harnesses = [\"x\", \"y\"]").unwrap();
    
    // Project declares `["m", "n"]` (overrides workspace)
    write_project_settings(&project_root, "harnesses = [\"m\", \"n\"]").unwrap();
    
    let list = resolve_effective_list(&scope, &paths)?;
    
    // Project wins
    assert_eq!(list, vec!["m", "n"]);
}
```

Pattern: The test uses synthetic harness names (`a`, `b`, `x`, `y`, `m`, `n`) installed via `HarnessModulesGuard` so validation passes. See CONVENTIONS.md for the guard discipline.

### Workspace Lifecycle Tests (Phase 4 US2+)

Tests for workspace operations (`rename`, `remove`, `sync`) exercise the two-phase sync pattern:

```rust
#[test]
fn rename_updates_marker_and_database() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    
    let outcome = workspace::init::init(parse("old-name"), false, &paths).expect("init");
    
    // Phase A: database update (atomic with marker rename)
    // Phase B: sync harnesses (unlocked)
    let result = workspace::rename::rename(parse("old-name"), parse("new-name"), &paths)
        .expect("rename");
    
    assert_eq!(result.new_name.as_str(), "new-name");
    
    // Verify marker file reflects the new name
    let marker = paths.workspace_dir(&result.new_name).join(".tome");
    assert!(marker.exists());
}
```

## Shared Test Harness

All integration tests import from `tests/common/mod.rs`:

### Fixture

A self-contained Git repository fixture built from `tests/fixtures/sample-catalog/`:

```rust
pub struct Fixture {
    pub tempdir: TempDir,
    pub repo_path: PathBuf,
    pub url: String,  // file:// URL for cloning
}

impl Fixture {
    pub fn build_sample() -> Self {
        Self::build_from(fixture_path("sample-catalog"))
    }
}
```

The `sample-catalog/` skeleton contains:
- `plugin-alpha/` with skills (skill-a, skill-b, skill-c, skill-d, skill-malformed-yaml-body)
- `plugin-beta/` (minimal)
- `.gitignore` (so empty directories are tracked)

Each test calls `Fixture::build_sample()` which:
1. Copies the skeleton to a temp dir
2. Runs `git init && git add -A && git commit`
3. Returns a `file://` URL the CLI can clone from

### ToolEnv

Isolated environment for spawning the `tome` binary:

```rust
pub struct ToolEnv {
    pub home: TempDir,
}

impl ToolEnv {
    pub fn new() -> Self {
        Self {
            home: TempDir::new().expect("tmpdir"),
        }
    }

    pub fn home_path(&self) -> PathBuf {
        self.home.path().to_path_buf()
    }
}
```

Pass the `home_path()` via `Command::env("HOME", ...)` so the spawned binary never sees real config.

### RAII Guards (Phase 4 US3+)

Multiple RAII guards in `tests/common/mod.rs` manage test isolation:

**`HomeGuard`**: Restores `$HOME` after mutation
```rust
pub struct HomeGuard {
    _previous: PrevHome,  // Drops FIRST, restores HOME
    _lock: MutexGuard<'static, ()>,  // Drops SECOND, releases mutex
}

impl HomeGuard {
    pub fn install(new_home: &Path) -> Self {
        let lock = HOME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os("HOME");
        unsafe { std::env::set_var("HOME", new_home) };
        Self {
            _previous: PrevHome(previous),
            _lock: lock,
        }
    }
}
```

**`HarnessModulesGuard`**: Manages harness module injection
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

**`SummariserOverrideGuard`**: Manages summariser injection (Phase 4 US4.b)
```rust
pub use tome::summarise::SummariserOverrideGuard;

// In test file:
let stub = StubSummariser::new();
let stub_arc: Arc<dyn Summariser> = Arc::new(stub.clone());
let _guard = SummariserOverrideGuard::install(stub_arc);
// Guard drops at end of test scope, clearing SUMMARISER_OVERRIDE
```

All guards survive panics and use field-order semantics to ensure cleanup happens in the correct sequence. See CONVENTIONS.md for detailed discipline.

### Helper Functions

**Fabricate Models**:
```rust
pub fn fabricate_installed_model(paths: &Paths, entry: &ModelManifest) {
    // Create sparse files (zero-filled via set_len) for each model artefact
    // Fast: 45 MB embedder + 280 MB reranker take no actual disk space
}

pub fn fabricate_all_registry_models(paths: &Paths) {
    for entry in MODEL_REGISTRY {
        fabricate_installed_model(paths, entry);
    }
}
```

**Config & Paths**:
```rust
pub fn config_with_catalog(catalog_name: &str, catalog_root: &Path) -> Config {
    // Construct a minimal Config with one catalog
}

pub fn paths_for(env: &ToolEnv) -> Paths {
    // Derive XDG paths from ToolEnv.home (promoted to common after 4th caller)
}

pub fn lifecycle_paths(root: &Path) -> Paths {
    // Paths rooted in a tempdir for unit-like tests
}
```

**Database Fixtures**:
```rust
pub fn write_index_db_with_schema_version(path: &Path, version: u32) {
    // Create a minimal index.db with only the meta table at the requested version
    // No binary .db files committed; generated at test setup time
}
```

**Stub Seeds**:
```rust
pub fn stub_embedder_seed() -> MetaSeed {
    // MetaSeed matching StubEmbedder identity
}

pub fn stub_reranker_seed() -> MetaSeed {
    // MetaSeed matching StubReranker identity
}

pub fn stub_summariser_seed() -> MetaSeed {
    // MetaSeed matching StubSummariser identity (Phase 4 US4)
}
```

**Workspace Setup (Phase 4 US2+)**:
```rust
pub fn seed_workspace(paths: &Paths, name: &str) {
    // Create the workspace in the central DB
}

pub fn seed_project(paths: &Paths, workspace_name: &str, project_root: &Path) {
    // Bind a project to a workspace in the central DB
}
```

**Synthetic Harness Modules (Phase 4 US3)**:
```rust
pub struct NamedStubHarness;
impl NamedStubHarness {
    pub fn new(name: &str) -> Box<dyn HarnessModule> {
        // Creates a stub module with the given name (leaked via Box::leak)
    }
    
    pub fn boxed_set<I, S>(names: I) -> Vec<Box<dyn HarnessModule>>
    where I: IntoIterator<Item = S>, S: AsRef<str> {
        // Helper to build a vec of synthetic harnesses from an iterator
    }
}
```

Used in composition tests to exercise the harness resolver with arbitrary names.

## Test Data

### Fixtures

Pre-built test data committed to `tests/fixtures/`:

- `sample-catalog/` — Git repository skeleton with plugin fixtures
  - `plugin-alpha/` — Comprehensive plugin with good + malformed skills
  - `plugin-beta/` — Minimal plugin for multi-catalog tests

### Fabricators

Helper functions that synthesize test data on the fly:

- `fabricate_installed_model()` — Sparse files for model artefacts
- `copy_sample_plugin_catalog()` — Copy a fixture to a temp dir
- `write_index_db_with_schema_version()` — Generate synthetic DB at a version
- `seed_workspace()` — Insert a workspace into the central DB
- `seed_project()` — Bind a project to a workspace

**Rationale**: No opaque binary `.db` files in git; no large test data in the repository. Fixtures are code + templates; synthesis is deterministic.

## Test Categories

### Happy Path Tests

Tests exercising the successful flow:

- `plugin_enable.rs::enable_inserts_skill_rows_with_content_hash_and_enabled_flag`
- `workspace_init.rs::init_creates_dot_tome_with_empty_config`
- `workspace_use_binding.rs::bind_inserts_row_and_returns_outcome`
- `workspace_rename.rs::rename_updates_marker_and_database`
- `harness_list_effective.rs::resolve_effective_list_prioritizes_layers`
- `query.rs::knn_returns_sorted_results_above_minimum_score`
- `summariser_triggers.rs::summariser_fires_after_enable`

### Error Path Tests

Tests exercising specific failure modes:

- `workspace_resolution.rs::global_fallback_with_workspace_missing_index` (bootstrap-not-yet)
- `doctor.rs::missing_reranker_classified_as_degraded` (partial failure)
- `exit_codes_e2e.rs::plugin_show_with_malformed_plugin_json_exits_22` (specific exit code)
- `workspace_remove_cascade.rs::remove_refuses_without_force_when_plugins_exist`
- `settings_harness_not_supported.rs::validate_rejects_unsupported_harness_names`
- `summariser_triggers.rs::trigger_returns_ok_when_model_missing_silent_noop` (Phase 4 US4)

### Atomicity Tests

Tests verifying multi-step operations are all-or-nothing:

- `atomicity.rs::enable_interrupted_mid_transaction_rolls_back` (inject error mid-transaction)
- `workspace_init.rs::init_atomic_rename_prevents_partial_dotted_tome` (crash mid-populate)
- `workspace_use_atomicity.rs::bind_with_concurrent_writers_serializes` (concurrent binding)
- `schema_migration_e2e.rs::forward_migration_fails_mid_step_rolls_back` (SIGINT mid-migration)

### Concurrency Tests

Tests verifying multi-process and multi-thread safety:

- `concurrency.rs::two_processes_contending_on_index_lock` (advisory lock)
- `workspace_use_concurrent.rs::concurrent_threads_both_succeed` (thread barrier + binding)
- `atomicity_enable.rs::enable_of_enabled_is_noop` (idempotency)

### Strictness Boundary Tests

Tests verifying the strictness/lenience split:

- `manifest_strictness.rs::tome_owned_config_rejects_unknown_fields`
- `manifest_strictness.rs::third_party_plugin_json_ignores_unknown_fields`

### Schema & Migration Tests

Tests verifying schema evolution:

- `schema_migrations.rs::read_path_schema_too_new_exits_52`
- `schema_migration_e2e.rs::forward_migration_v0_to_v1_succeeds`
- `schema_migration_e2e.rs::forward_migration_fails_mid_step_rolls_back`

### Thread-Local Injection Tests (Phase 3+)

Tests verifying per-thread injection patterns:

- `schema_migration_e2e.rs` uses `MigrationsGuard::install(MIGRATIONS)` with per-thread scope
- `sync_algorithm.rs` uses `HarnessModulesGuard::install(stubs)` for harness dispatch testing
- `settings_composition.rs` uses `HarnessModulesGuard::install(synth)` for synthetic harnesses
- `summariser_triggers.rs` uses `SummariserOverrideGuard::install(stub)` (Phase 4 US4)

**Important**: `MIGRATIONS_OVERRIDE`, `HARNESS_MODULES_OVERRIDE`, and `SUMMARISER_OVERRIDE` are `thread_local!` and do NOT propagate across `thread::spawn`. Writer threads must install their own guard.

See CONVENTIONS.md for details on the `#[doc(hidden)] pub static` + RAII guard pattern.

### Idempotence Tests (Phase 4)

Tests verifying repeated operations with identical inputs produce no changes:

- `sync_idempotence.rs::rules_file_write_preserves_mtime_on_idempotent_rewrite`
- `sync_idempotence.rs::mcp_config_write_preserves_mtime_on_idempotent_rewrite`

Pattern: capture mtime before write, sleep 1.5s, rewrite identical content, verify mtime unchanged.

### Settings Composition Tests (Phase 4 US3)

Tests verifying layer priority and validation:

- `settings_composition.rs::resolve_effective_list_prioritizes_project_over_workspace_over_global`
- `settings_priority.rs::global_layer_is_foundation`
- `settings_cycle_detection.rs::circular_workspace_refs_rejected`
- `settings_workspace_ref_outside_project.rs::workspace_ref_must_be_in_project`
- `settings_bad_exclusion.rs::exclusion_without_inclusion_rejected`
- `settings_harness_not_supported.rs::validate_per_entry_unsupported_harness`

### Summariser Tests (Phase 4 US4)

Tests verifying summariser wiring and behavior:

- `summariser_triggers.rs::summariser_fires_after_enable` (trigger wiring)
- `summariser_triggers.rs::trigger_skips_summariser_when_unchanged` (cheap skip)
- `summariser_cache.rs::trigger_overwrites_cached_summaries` (cache invalidation)
- `summariser_cache.rs::read_only_paths_do_not_invoke_summariser` (cache reuse)
- `summariser_stub.rs::stub_deterministic_output` (stub determinism)
- `summariser_registry_no_placeholder.rs::registry_qwen_sha256_is_not_placeholder` (regression)

## Mocking Strategy

### StubEmbedder

A deterministic stub for the `Embedder` trait that avoids loading ONNX models:

```rust
pub struct StubEmbedder {
    call_count: Cell<u32>,
    force_fail_after: Option<u32>,
}

impl StubEmbedder {
    pub fn new() -> Self { /* ... */ }
    
    pub fn with_force_fail_after(calls: u32) -> Self {
        Self {
            call_count: Cell::new(0),
            force_fail_after: Some(calls),
        }
    }
    
    pub fn call_count(&self) -> u32 {
        self.call_count.get()
    }
}

impl Embedder for StubEmbedder {
    fn embed(&self, input: &str) -> Result<Vec<f32>, TomeError> {
        let count = self.call_count.get() + 1;
        self.call_count.set(count);
        
        if let Some(fail_after) = self.force_fail_after {
            if count > fail_after {
                return Err(TomeError::EmbedderFailure { /* ... */ });
            }
        }
        
        // Deterministic hash-based vector
        let seed = hash(input);
        Ok(vec![seed as f32 / u32::MAX as f32; 384])
    }
}
```

**Usage**:
- Library API tests pass `&embedder` instead of loading `FastembedEmbedder`
- Call `stub_embedder_seed()` to get a `MetaSeed` matching the stub's identity
- Use `with_force_fail_after` to inject mid-stream failures

### StubReranker

Similar stub for the `Reranker` trait:

```rust
pub struct StubReranker;

impl Reranker for StubReranker {
    fn rerank(&self, query: &str, candidates: &[&str]) -> Result<Vec<f32>, TomeError> {
        // Deterministic scores
        Ok(candidates.iter().map(|c| similarity(query, c)).collect())
    }
}
```

### StubSummariser (Phase 4 US4)

Stub for the `Summariser` trait that avoids loading Llama.cpp models:

```rust
#[derive(Clone)]
pub struct StubSummariser {
    call_count: Arc<Cell<u32>>,
}

impl StubSummariser {
    pub fn new() -> Self {
        Self {
            call_count: Arc::new(Cell::new(0)),
        }
    }
    
    pub fn call_count(&self) -> u32 {
        self.call_count.get()
    }
}

impl Summariser for StubSummariser {
    fn summarise(&self, input: &PluginSummariesInput) -> Result<SummariserOutput, TomeError> {
        let count = self.call_count.get() + 1;
        self.call_count.set(count);
        
        // Deterministic stub: join enabled skill names as the short summary
        let short = input.plugins.iter()
            .flat_map(|p| p.skills.iter().map(|s| s.name.clone()))
            .collect::<Vec<_>>()
            .join(", ");
        
        Ok(SummariserOutput {
            short,
            long: format!("Summary of {} skills", input.plugins.len()),
        })
    }
}
```

**Key properties** (Phase 4 US4.b):
- `call_count` is backed by `Arc<Cell<u32>>` so multiple holders of the stub see the same counter
- Cloneable via `#[derive(Clone)]` so the stub can be wrapped in `Arc<dyn Summariser>` and held in `SUMMARISER_OVERRIDE`
- Deterministic output: enabled skill names form the short summary

### StubHarness (Phase 4)

Stub for the `HarnessModule` trait used in harness sync tests:

```rust
pub struct StubHarness;

impl HarnessModule for StubHarness {
    fn name(&self) -> &str { "stub" }
    fn sync_rules_file(&self, project_root: &Path, content: &str) -> Result<(), TomeError> {
        // No-op stub
        Ok(())
    }
    // ... other trait methods ...
}
```

### Git Fixtures

No mocking of git; use real `file://` repositories:

```rust
let fixture = Fixture::build_sample();
// fixture.url is a real git repo; `tome catalog add file://...` clones it
```

**Rationale**: Git is critical; mocking it would hide real bugs. Fixtures are cheap (shallow clones).

## Coverage Requirements

**Target**: No hard coverage percentage (avoid the coverage trap). Instead, require:
- Every code path reachable from the public API is tested
- Every error variant is covered
- Every exit code is exercised
- Atomicity properties verified
- Concurrency safety verified

**Measurement**: `cargo tarpaulin` or similar tool tracks coverage informally; a drop in coverage flags potential gaps for review.

**Exclusions**:
- `src/embedding/stub.rs` — stub implementation
- `src/harness/stub.rs` — stub harness module
- `src/summarise/stub.rs` — stub summariser
- Config file parsing (covered by fixture loading)
- Dead code (rare due to strong module boundaries)

## CI Integration

### Test Pipeline

```yaml
- cargo test --lib          # Unit tests
- cargo test                # All tests (unit + integration)
- cargo fmt --check         # Style
- cargo clippy ... -D warnings  # Linting
- typos                     # Spell check
```

All three quality gates are enforced locally via `.githooks/pre-commit`. PR CI mirrors the same checks.

### Test Execution

- **Unit tests**: ~1 second (no I/O)
- **Integration tests**: ~60–80 seconds (temp dirs, fixture setup, git ops, DB creation, workspace lifecycle)
- **Total**: ~80 seconds on a modern machine

Tests are deterministic; no flakiness tolerance. A flaky test is a bug.

## Common Patterns

### Arrange-Act-Assert (AAA)

Every test follows the three-phase structure:

```rust
#[test]
fn test_example() {
    // Arrange: set up fixtures, config, state
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    let config = config_with_catalog("test", &catalog_root);
    
    // Act: call the function under test
    let result = some_function(&config, &paths);
    
    // Assert: verify the result and side-effects
    assert_eq!(result.status, "success");
    assert!(paths.index_db.is_file());
}
```

### Panic as Test Failure

Tests use `expect()` and `assert!()` liberally. A panic = test failure; a return of `Result` = test passes only if `Ok`.

```rust
let outcome = lifecycle::enable(&id, &deps).expect("enable must succeed");
// ↑ If enable returns Err, the test panics with the error message
```

For expected errors, use `assert!(result.is_err())`:

```rust
let result = "invalid".parse::<WorkspaceName>();
assert!(result.is_err());
```

### Fixture Isolation

Each test owns its fixtures; no shared state across tests:

```rust
#[test]
fn test_one() {
    let tmp = TempDir::new().unwrap();  // ← New temp dir per test
    // ...
}

#[test]
fn test_two() {
    let tmp = TempDir::new().unwrap();  // ← Fresh temp dir
    // ...
}
```

`TempDir` is cleaned up automatically on drop, so no manual cleanup needed.

### Static Lifetime Scope for Global State (Phase 4+)

When multiple tests need a `&'static Scope` for constructing dependencies:

```rust
pub fn test_scope() -> &'static tome::workspace::Scope {
    static SCOPE: std::sync::OnceLock<tome::workspace::Scope> = std::sync::OnceLock::new();
    SCOPE.get_or_init(|| {
        tome::workspace::Scope(tome::workspace::WorkspaceName::global())
    })
}
```

Returns `&'static Scope` so callers avoid repeated allocations. Preferred over per-call `fn test_scope() -> Scope` when many tests share the same global scope.

### Per-Test-File OVERRIDE_MUTEX Pattern (Phase 4 US3+)

When a test file uses `HARNESS_MODULES_OVERRIDE` or `SUMMARISER_OVERRIDE`, serialize all tests via a process-wide `Mutex`:

```rust
static OVERRIDE_MUTEX: Mutex<()> = Mutex::new(());

fn install_synthetic() -> (HarnessModulesGuard, MutexGuard<'static, ()>) {
    let lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let guard = HarnessModulesGuard::install(NamedStubHarness::boxed_set(names));
    (guard, lock)
}

#[test]
fn my_test() {
    let (_guard, _lock) = install_synthetic();  // Hold both for lifetime of test
    // ... test code ...
}  // Guard drops first (restores override to None), lock drops second (releases mutex)
```

Field order in the tuple (guard, lock) ensures guard drops before lock (RAII discipline). Without this pattern, concurrent tests race on the process-global injection slot.

## Known Gaps & Deferrals

**T088**: Manual SC-001 / SC-002 verification against real BGE models. The MCP tools exercise the KNN+rerank pipeline, but the real embedder isn't loaded in CI. Deferred to developer pass post-v0.3.0.

**T093/T094/T095**: MCP protocol-purity, latency, and SIGINT graceful-shutdown tests. Require either real models or a stub-injection point on `McpState`. Deferred to Phase 4+ / post-v0.3.0.

**T331**: Summariser drift detection in `tome doctor`. Deferred to Phase 4 / post-US4.

---

*This document describes HOW to test. Update when testing strategy changes. Last refreshed 2026-05-26 against Phase 4 / US4-complete source (862 passing tests, 16 ignored, 117 suites).*
