# Testing Strategy

> **Purpose**: Document test frameworks, patterns, organization, and coverage requirements.
> **Generated**: 2026-05-25
> **Last Updated**: 2026-05-25

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

**Phase 4 Status**: 609 passing tests, 29 ignored, across 82 test suites:
- ~100 unit tests in `src/lib.rs` + modules
- ~509 integration tests in `tests/*.rs`

## Test Organization

### Directory Structure

```
tests/
├── common/mod.rs                                  # Shared test harness (Fixture, ToolEnv, helpers)
│   └── Tests call `paths_for()`, `lifecycle_paths()`, `stub_embedder_seed()`, etc.
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
├── harness_module_claude_code.rs                  # Claude Code production harness (Phase 4)
├── harness_skeleton.rs                            # Harness module composition
├── harness_sync_stub.rs                           # Sync algorithm with StubHarness
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
├── plugin_summariser_forward_progress.rs          # Summariser byte-progress callback (Phase 4)
├── plugin_workspace_skills.rs                     # Workspace-scoped skills isolation (Phase 4)
├── query.rs                                       # KNN + reranker + drift
├── reindex.rs                                     # Reindex command
├── schema_migration_e2e.rs                        # Forward schema migrations (synthetic)
├── schema_migrations.rs                           # Schema version guards
├── scrubbing.rs                                   # Credential scrubbing in errors
├── security_hardening.rs                          # Hardening measures (chmod, symlink skip, mode preservation)
├── settings_skeleton.rs                           # Settings composition (Phase 4)
├── status.rs                                      # Status report assembly
├── summariser_stub.rs                             # StubSummariser determinism (Phase 4)
├── sync_algorithm.rs                              # Sync orchestrator (Phase 4)
├── sync_boundary.rs                               # Enforce tokio confinement to src/mcp/
├── sync_idempotence.rs                            # Idempotence-by-mtime for sync primitives (Phase 4)
├── version_output.rs                              # `tome --version` formats
├── workspace_commands.rs                          # Scope isolation across commands
├── workspace_info.rs                              # Workspace info report
├── workspace_init.rs                              # Workspace init atomicity
├── workspace_name.rs                              # WorkspaceName validation
├── workspace_resolution.rs                        # Workspace scope resolution algorithm
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
- `workspace_use_atomicity.rs` — workspace use atomicity properties
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
    // Derive XDG paths from ToolEnv.home (used at the 4th caller → promoted here)
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
    // MetaSeed matching StubSummariser identity (Phase 4)
}
```

**Workspace Setup**:
```rust
pub fn seed_workspace(paths: &Paths, name: &str) {
    // Create the workspace in the central DB (Phase 4)
}
```

**RAII Injection Guards**:
```rust
pub struct HarnessModulesGuard;
impl HarnessModulesGuard {
    pub fn install(modules: Vec<Box<dyn HarnessModule>>) {
        // Installs into HARNESS_MODULES_OVERRIDE thread-local
    }
}
impl Drop for HarnessModulesGuard {
    fn drop(&mut self) {
        // Clears the slot on drop (survives panics)
    }
}
```

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

**Rationale**: No opaque binary `.db` files in git; no large test data in the repository. Fixtures are code + templates; synthesis is deterministic.

## Test Categories

### Happy Path Tests

Tests exercising the successful flow:

- `plugin_enable.rs::enable_inserts_skill_rows_with_content_hash_and_enabled_flag`
- `workspace_init.rs::init_creates_dot_tome_with_empty_config`
- `workspace_use_binding.rs::bind_inserts_row_and_returns_outcome`
- `query.rs::knn_returns_sorted_results_above_minimum_score`

### Error Path Tests

Tests exercising specific failure modes:

- `workspace_resolution.rs::global_fallback_with_workspace_missing_index` (bootstrap-not-yet)
- `doctor.rs::missing_reranker_classified_as_degraded` (partial failure)
- `exit_codes_e2e.rs::plugin_show_with_malformed_plugin_json_exits_22` (specific exit code)

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

**Important**: `MIGRATIONS_OVERRIDE` and `HARNESS_MODULES_OVERRIDE` are `thread_local!` and do NOT propagate across `thread::spawn`. Writer threads must install their own guard.

See CONVENTIONS.md for details on the `#[doc(hidden)] pub static` + RAII guard pattern.

### Idempotence Tests (Phase 4)

Tests verifying repeated operations with identical inputs produce no changes:

- `sync_idempotence.rs::rules_file_write_preserves_mtime_on_idempotent_rewrite`
- `sync_idempotence.rs::mcp_config_write_preserves_mtime_on_idempotent_rewrite`

Pattern: capture mtime before write, sleep 1.5s, rewrite identical content, verify mtime unchanged.

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

### StubSummariser (Phase 4)

Stub for the `Summariser` trait that avoids loading Llama.cpp models:

```rust
pub struct StubSummariser;

impl Summariser for StubSummariser {
    fn summarise(&self, text: &str, _style: SummariseStyle) -> Result<String, TomeError> {
        // Deterministic stub response
        Ok(format!("Summary of {} chars", text.len()))
    }
}
```

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
- **Integration tests**: ~40–60 seconds (temp dirs, fixture setup, git ops, DB creation)
- **Total**: ~70 seconds on a modern machine

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

## Known Gaps & Deferrals

**T088**: Manual SC-001 / SC-002 verification against real BGE models. The MCP tools exercise the KNN+rerank pipeline, but the real embedder isn't loaded in CI. Deferred to developer pass post-v0.3.0.

**T093/T094/T095**: MCP protocol-purity, latency, and SIGINT graceful-shutdown tests. Require either real models or a stub-injection point on `McpState`. Deferred to Phase 4+ / post-v0.3.0.

**T088 (US4.b)**: Summariser manual verification — Qwen2.5-0.5B-Instruct model inference tested via stub only in CI. Real model testing deferred.

---

*This document describes HOW to test. Update when testing strategy changes. Last refreshed 2026-05-25 against Phase 4 / US1 source (609 tests, 82 suites).*
