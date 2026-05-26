# Testing Strategy

> **Purpose**: Document test frameworks, organization, patterns, and coverage expectations.
> **Generated**: 2026-05-26
> **Last Updated**: 2026-05-26 (Phase 5 US1 shipped via `/sdd:map incremental`)

## Test Framework

| Type | Framework | Configuration |
|------|-----------|---------------|
| Unit | Rust built-in (`#[test]`) | Inline in `src/**/*.rs` modules |
| Integration | Rust built-in (`#[test]`) | Separate binaries in `tests/` directory |
| E2E (CLI) | Integration tests + `Command` subprocess invocation | `tests/fixtures/` + real binary |

Cargo automatically discovers and runs tests via `cargo test`. No external test runner or config needed.

### Running Tests

| Command | Purpose | Speed |
|---------|---------|-------|
| `cargo test` | All tests + lib (954 passing, 16 ignored, 127 suites) | ~1 min |
| `cargo test --lib` | Unit tests only (153 passing) | ~1 sec |
| `cargo test --test <name>` | Single integration file | ~1-10 sec |
| `cargo test <pattern>::` | Tests matching pattern | Varies |
| `cargo test -- --nocapture` | Show stdout/stderr | Full output |
| `cargo test -- --test-threads=1` | Sequential (debug thread-local state) | Slow but deterministic |

**Quality gates** enforced by git hooks:
- `.githooks/pre-commit`: `cargo fmt --check`, `cargo clippy`, `typos`
- `.githooks/pre-push`: `cargo test --workspace` (full suite)

### Stub Embedder & Reranker (Avoid Real Models)

Tests use a **deterministic stub** instead of real `FastembedEmbedder` to keep CI fast and bounded:

```rust
// src/embedding/stub.rs — not gated by #[cfg(test)]
pub struct StubEmbedder {
    call_count: Arc<AtomicUsize>,
}

impl Embedder for StubEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, TomeError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        // Deterministic: same input always produces same vector
        Ok(vec![0.1, 0.2, 0.3, ...])
    }
}

impl StubEmbedder {
    pub fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }
}
```

Tests can assert on `call_count()` to verify cheap re-enable (FR-006) and other optimization paths don't invoke the embedder unnecessarily.

## Test Organization

### Directory Structure

```
tests/
├── common/mod.rs               # Shared helpers: Fixture, HomeGuard, ClockOverrideGuard, PluginDataDirGuard, WorkspaceDataDirGuard
├── catalog_*.rs                # Phase 1 catalog add/remove/list/show (extended for P2+)
├── plugin_*.rs                 # Phase 2 plugin enable/disable/list/show + interactive
├── query.rs                    # Phase 2 KNN + reranker search
├── models_*.rs                 # Phase 2 model download/list/remove
├── workspace_*.rs              # Phase 3 info/init + Phase 4 binding/use/list/rename/sync/remove
├── harness_*.rs                # Phase 4 harness lifecycle, integration, rules, MCP
├── doctor_*.rs                 # Phase 4 doctor report, fixes, subsystems, JSON shapes
├── mcp_*.rs                    # Phase 3 MCP server lifecycle, tools, logging
├── mcp_prompts*.rs             # Phase 5 MCP prompts + list/get endpoints (new, US1)
├── settings_*.rs               # Phase 4 composition resolution, edit, validation
├── summariser_*.rs             # Phase 4 summariser registry, prompts, real-model E2E
├── substitution*.rs            # Phase 5 skeleton + entry_kind_indexing (new, US1)
├── entry_kind_indexing.rs      # Phase 5 US1.a — kind-discriminated entry indexing with schema v3
├── frontmatter_p5_fields.rs    # Phase 5 US1.a — frontmatter parser extensions
├── prompt_naming.rs            # Phase 5 US1.a — prompt naming algorithm + collision handling
├── atomicity.rs                # Interrupt-injection for atomic writes
├── concurrency.rs              # Two-process lock contention
├── exit_codes.rs               # Exhaustive CLI exit code verification
├── manifest_strictness.rs      # Verify deny_unknown_fields on all Tome-owned types
├── sync_boundary.rs            # Enforce tokio/rmcp scoped to src/mcp/ only
├── doctor_subsystem_serialize.rs # Phase 4 Subsystem enum round-trip stability
├── fixtures/
│   └── sample-catalog/         # Git repo skeleton for catalog tests
└── [135+ .rs integration test files total]
```

### Test File Header Pattern

Every integration test file includes a module comment explaining its scope:

```rust
//! Phase 5 / US1.a (T374) — kind-discriminated entry indexing and schema v3
//! promotion. Exercises `lifecycle::enable` against a hand-rolled fixture
//! plugin that ships BOTH `skills/*/SKILL.md` and `commands/*.md`. Verifies
//! the schema-v3 column shape, identity-tuple widening, and the
//! `when_to_use`-aware embedding/content-hash composition.

mod common;

use <imports>;

#[test]
fn descriptive_name() { ... }
```

## Test Patterns

### Unit Tests (Library API)

Inline in `src/` modules via `#[cfg(test)]` blocks:

```rust
// src/module/feature.rs
pub fn process(input: &str) -> Result<String, Error> {
    // implementation
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_valid_input() {
        let result = process("test").unwrap();
        assert_eq!(result, "expected");
    }
}
```

**Coverage**: Pure compute paths, error branches, edge cases. Does NOT exercise filesystem, concurrency, or full lifecycle.

**Count**: 153 tests across 1 test suite (`src/lib.rs`).

### Integration Tests (Isolated CLI + Filesystem)

Located in `tests/*.rs`; each file is compiled as a separate binary:

```rust
// tests/catalog_add.rs
#[test]
fn add_catalog_persists_to_config_toml() {
    // 1. Build a real Git repo fixture
    let fixture = Fixture::build_sample();
    
    // 2. Create isolated Paths in a TempDir
    let tmpdir = TempDir::new().unwrap();
    let tool_env = ToolEnv::new(&tmpdir);
    
    // 3. Invoke the CLI binary as a subprocess
    let output = Command::new(cargo_bin("tome"))
        .args(&["catalog", "add", "test", &fixture.url])
        .env_clear()
        .env("HOME", tool_env.home())
        .output()
        .expect("command");
    
    // 4. Assert on exit code and filesystem state
    assert_eq!(output.status.code(), Some(0));
    let config = std::fs::read_to_string(tool_env.config_file()).unwrap();
    assert!(config.contains("test"));
}
```

**Isolation guarantees**:
- Fresh `Fixture` per test (isolated Git repo in `TempDir`)
- Fresh `ToolEnv` per test (isolated `HOME` and `XDG_*` paths, never touches host)
- CLI runs as subprocess (cross-process, no shared Rust state)

**Why subprocess?** The binary is compiled once (`cargo build`), and each test forks a separate `Command` invocation. This ensures no global state leaks (ctrlc handlers, thread-locals, open file descriptors) between tests, mirroring how users invoke the tool.

**Count**: 800+ integration tests across 135+ files.

### Library-API Tests (Non-CLI Reuse)

When a command has a library entry point (e.g., `assemble`, `pipeline`), test it directly without the CLI wrapper:

```rust
// tests/workspace_info.rs
#[test]
fn assemble_workspace_info_without_cli_emission() {
    let tool_env = ToolEnv::new(&tmpdir);
    let paths = tool_env.paths();
    let scope = ResolvedScope::global();
    
    // Direct library call; no CLI wrapper
    let outcome = commands::workspace::info::assemble(
        Args::default(),
        &scope,
        &paths
    ).expect("assemble");
    
    // Assert on the returned Outcome struct, not stdout
    assert_eq!(outcome.kind, ScopeKind::Global);
}
```

Tested in: `tests/workspace_info.rs`, `tests/harness_info.rs`, `tests/plugin_list.rs`, `tests/doctor_p4.rs`, `tests/mcp_prompts.rs`.

**Benefit**: Tests the compute logic without waiting for the CLI binary to compile and run, and decouples from output formatting changes.

### E2E Exit Code Tests

Tests that invoke the CLI and verify exit codes for various failure scenarios:

```rust
// tests/exit_codes.rs
#[test]
fn exit_code_30_on_missing_embedder_model() {
    let tool_env = ToolEnv::new(&tmpdir);
    // Intentionally don't create model files
    
    let output = Command::new(cargo_bin("tome"))
        .args(&["plugin", "enable", "catalog/plugin"])
        .env_clear()
        .env("HOME", tool_env.home())
        .output()
        .expect("command");
    
    assert_eq!(output.status.code(), Some(30));  // ModelMissing
}
```

Exhaustive coverage: Every `TomeError` variant has a corresponding `#[test]` in `exit_codes.rs`. The closed-enum design forces all variants to be tested. Polish phase (PR-A through PR-G) added exit-code tests for codes 14, 16, 17, 18, 70, 7 via CLI binary e2e scenarios.

**Coverage**: 19+ distinct exit codes verified end-to-end.

### Concurrent & Contention Tests

Tests that spawn multiple threads/processes to verify locking and race-free behavior:

```rust
// tests/concurrency.rs
#[test]
fn index_lock_serializes_concurrent_writers() {
    let tmpdir = TempDir::new().unwrap();
    
    let handle1 = std::thread::spawn(|| {
        let tool_env = ToolEnv::new(&tmpdir);
        let _lock = index::lock::LockFile::acquire(&tool_env.paths().index_lock)?;
        std::thread::sleep(Duration::from_secs(1));  // Hold lock
        Ok(())
    });
    
    // Second writer waits
    let handle2 = std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(100));
        let tool_env = ToolEnv::new(&tmpdir);
        let start = Instant::now();
        let _lock = index::lock::LockFile::acquire(&tool_env.paths().index_lock)?;
        let elapsed = start.elapsed();
        assert!(elapsed > Duration::from_millis(900));  // Waited for first
        Ok(())
    });
    
    handle1.join().unwrap().unwrap();
    handle2.join().unwrap().unwrap();
}
```

Used in: `tests/concurrency.rs`, `tests/atomicity.rs`, `tests/workspace_use_concurrent.rs`, `tests/workspace_use_atomicity.rs`.

### Interrupt-Injection Tests (Atomicity Verification)

Tests that model SIGINT mid-transaction by returning a deliberate `Err` from a migration closure:

```rust
// tests/atomicity.rs
#[test]
fn migration_abort_mid_transaction_leaves_schema_unchanged() {
    let db = TempDir::new().unwrap();
    let path = db.path().join("index.db");
    
    // Bootstrap schema v0 DB
    write_index_db_with_schema_version(&path, 0).unwrap();
    
    // Install a migration that fails mid-way
    let migrations = vec![
        Migration {
            from: 0,
            to: 1,
            name: "test",
            apply: |tx| {
                tx.execute("CREATE TABLE test (id INTEGER)", [])?;
                Err(TomeError::Interrupted)  // Model SIGINT here
            },
        },
    ];
    
    let _guard = MigrationsGuard::install(&migrations);
    
    // Attempt migrate; should rollback both table + version
    let conn = rusqlite::Connection::open(&path).unwrap();
    let result = apply_pending(&conn, 0, 1);
    
    assert!(result.is_err());
    
    // Verify rollback: table never created, version unchanged
    let tables: Vec<String> = conn.prepare("SELECT name FROM sqlite_master WHERE type='table'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(!tables.contains(&"test".to_string()));
    
    let schema_version: u32 = conn.query_row(
        "SELECT version FROM meta",
        [],
        |r| r.get(0)
    ).unwrap();
    assert_eq!(schema_version, 0);
}
```

**Why not use `catalog::git::CANCELLED` signal?** Tests run in the same process; flipping a global static races with other tests. Model SIGINT as a deliberate closure-level `Err` — the rollback path is identical regardless of signal origin.

**Count**: 10+ interrupt-injection tests in `atomicity.rs` and `atomicity_enable.rs`.

### Fixture & Factory Patterns

#### Real File State: `Fixture`

```rust
// tests/common/mod.rs
pub struct Fixture {
    pub tempdir: TempDir,
    pub repo_path: PathBuf,
    pub url: String,  // file://...
}

impl Fixture {
    pub fn build_sample() -> Self {
        let tempdir = TempDir::new().expect("tempdir");
        let repo_path = tempdir.path().join("catalog");
        copy_dir(&fixture_path("sample-catalog"), &repo_path).expect("copy");
        git_init_and_commit(&repo_path);  // Real Git init
        let url = format!("file://{}", repo_path.display());
        Self { tempdir, repo_path, url }
    }
}
```

Used whenever tests need a real on-disk Git repository to clone or add.

#### Synthetic Models: `fabricate_*`

```rust
// tests/common/mod.rs
pub fn fabricate_installed_model(dir: &Path, name: &str, size_mb: u64) -> PathBuf {
    let path = dir.join(format!("{}.gguf", name));
    std::fs::File::create(&path)
        .unwrap()
        .set_len(size_mb * 1024 * 1024)  // Sparse file, zero-filled
        .unwrap();
    path
}
```

Uses `std::fs::File::set_len()` to create sparse files. A 280 MB reranker takes ~no actual disk space, and the all-zero contents intentionally disagree with the registry-pinned SHA-256, which `--verify` tests rely on.

#### Synthetic Database: `write_index_db_with_schema_version`

```rust
// tests/common/mod.rs
pub fn write_index_db_with_schema_version(path: &Path, version: u32) -> Result<()> {
    let conn = rusqlite::Connection::open(path)?;
    conn.execute(
        "CREATE TABLE meta (
            schema_version INTEGER PRIMARY KEY,
            embedder_name TEXT,
            reranker_name TEXT
        )",
        [],
    )?;
    conn.execute(
        "INSERT INTO meta (schema_version, embedder_name, reranker_name) VALUES (?, ?, ?)",
        (version, "bge-small-en-v1.5", "bge-reranker-base"),
    )?;
    Ok(())
}
```

Generates synthetic `.db` files at runtime rather than committing binary fixtures. Avoids PR noise and binary churn in git history.

#### Test Helper Patterns: `lifecycle_paths`, `stub_embedder_seed`

Phase 5 extends test helpers for multi-context testing:

```rust
// tests/common/mod.rs
pub fn lifecycle_paths(home: &Path) -> tome::paths::Paths {
    tome::paths::Paths::from_env_or_home(home).expect("paths")
}

pub fn stub_embedder_seed() -> tome::embedding::registry::ModelManifest {
    // Pre-configured seed for StubEmbedder initialization
}

pub fn stub_reranker_seed() -> tome::embedding::registry::ModelManifest {
    // Pre-configured seed for StubReranker initialization
}

pub fn stub_summariser_seed() -> tome::summarise::StubSummariserConfig {
    // Pre-configured seed for StubSummariser initialization
}
```

Used in: Entry-kind indexing tests, prompt naming tests, MCP prompts tests.

## Test Coverage & Categorization

### Overall: 954 Passing Tests, 127 Suites, 16 Ignored

**Breakdown by Phase**:
- **Phase 1**: ~40 tests (catalog add/remove/list/show, path validation, strictness)
- **Phase 2**: ~85 tests (plugin enable/disable/list/show, query, models, exit codes)
- **Phase 3**: ~120 tests (workspace info/init, MCP server, doctor, schema migrations)
- **Phase 4**: ~710 tests (workspace binding/use/list, harness lifecycle, settings composition, doctor US5, summariser, Polish e2e)
- **Phase 5 (US1)**: ~9 new tests (substitution skeleton, entry-kind indexing, frontmatter p5 fields, prompt naming, mcp_prompts)

**Ignored**: 16 tests
- `#[ignore]` used for tests requiring external resources (real model downloads, network access)
- E.g., `summariser_real.rs` requires Qwen2.5 model; enable via `TOME_REAL_SUMMARISER_TESTS=1 cargo test --test summariser_real -- --include-ignored`

### Coverage by Domain

| Domain | Count | Example |
|--------|-------|---------|
| Library API (unit) | 153 | In-process tests in `src/` modules |
| CLI + Filesystem | 800+ | `tests/catalog_add.rs`, `tests/workspace_use_atomicity.rs` |
| Concurrency/Atomicity | 30 | Two-thread/process contention, interrupt injection |
| Exit Codes | 50+ | `tests/exit_codes.rs`, `tests/exit_codes_e2e.rs` (Polish: new CLI binary tests) |
| Schema/Migration | 25+ | Forward migration, MVCC reader, too-new schema |
| Strictness & Format | 20+ | `deny_unknown_fields`, Subsystem wire shape, EffectiveEntry, AsWrittenOutcome, SyncOutcome, WorkspaceCatalogEntry, DoctorReport |
| JSON Wire Shapes | 20+ | EffectiveEntry, AsWrittenOutcome, SyncOutcome, WorkspaceCatalogEntry, DoctorReport, PromptDescriptor (Phase 5 US1) |
| Phase 5 US1 Framework | 9 | substitution_skeleton, entry_kind_indexing, frontmatter_p5_fields, prompt_naming, mcp_prompts |

### Tests NOT Yet Included (Deferred)

- **Real BGE model inference** (SC-001 / SC-002): T088 in `retro/P3.md` — requires real embedder + reranker downloads. Deferred to manual developer verification.
- **Real Qwen2.5 summariser** (Phase 4 US4): `tests/summariser_real.rs` exists but `#[ignore]`. Enable with `TOME_REAL_SUMMARISER_TESTS=1`.
- **MCP protocol state machine** (T093–T095): Full SIGINT + deadline latency tests deferred.
- **Phase 5 US2–US5 integration tests**: TBD in subsequent slices.

## Test Isolation & Safety

### Environment Isolation

Every test uses **absolute temp paths** and never touches the host's real `$HOME`:

```rust
let tmpdir = TempDir::new().unwrap();
let tool_env = ToolEnv::new(&tmpdir);

// All XDG paths isolated to tmpdir
let paths = tool_env.paths();  // XDG_DATA_HOME, XDG_CONFIG_HOME, etc. → tmpdir

// CLI binary gets isolated HOME
Command::new(cargo_bin("tome"))
    .env_clear()
    .env("HOME", tool_env.home())
    .env("XDG_DATA_HOME", tool_env.data_dir())
    // ... other XDG vars ...
```

### Thread-Local Injection Safety

Thread-local overrides (`MIGRATIONS_OVERRIDE`, `HARNESS_MODULES_OVERRIDE`, `SUMMARISER_OVERRIDE`, `SUBSTITUTION_CLOCK_OVERRIDE`, etc.) use RAII guards with `Drop` cleanup:

```rust
pub struct HarnessModulesGuard;
impl HarnessModulesGuard {
    pub fn install(modules: Vec<Box<dyn HarnessModule>>) {
        *HARNESS_MODULES_OVERRIDE.write().expect("poisoned") = Some(modules);
    }
}
impl Drop for HarnessModulesGuard {
    fn drop(&mut self) {
        *HARNESS_MODULES_OVERRIDE.write().expect("poisoned") = None;
    }
}
```

Guard survives panics; cleanup is guaranteed. For tests sharing the same injection slot, declare a process-wide `Mutex` to serialize access:

```rust
static OVERRIDE_MUTEX: Mutex<()> = Mutex::new(());

fn install_synthetic() -> (HarnessModulesGuard, MutexGuard<'static, ()>) {
    let lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let guard = HarnessModulesGuard::install(...);
    (guard, lock)  // Hold both; guard drops first
}
```

### Environment Variable Isolation

Tests that mutate `$HOME` wrap it in a `HomeGuard` with a process-wide `HOME_MUTEX`. Centralized pattern (Polish: PR-E consolidated approach):

```rust
static HOME_MUTEX: Mutex<()> = Mutex::new(());

pub struct HomeGuard {
    _previous: PrevHome,       // Drops FIRST, restores HOME
    _lock: MutexGuard<()>,     // Drops SECOND, releases mutex
}

#[test]
fn my_test() {
    let _lock = HOME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _home = HomeGuard::install(tmpdir);
    // ... test reads $HOME ...
    // Implicit drop: _home drops (restores HOME), then _lock releases
}
```

**Critical**: Field declaration order ensures restoration happens while the mutex is still held. Polish phase (PR-E) consolidated all `HomeGuard` usage across harness/workspace tests to use the centralized `tests/common/mod.rs` implementation.

### Determinism & Reproducibility

- **Stub embedder**: Deterministic output (same input → same vector every time)
- **Timestamps**: Pinned to known values (e.g., `OffsetDateTime::from_unix_timestamp(1_700_000_000)`)
- **Clock injection**: Phase 5 introduces `ClockOverrideGuard` for fixed-time testing in substitution
- **Mtime tests**: Sleep 1.5 seconds between reads to ensure filesystem granularity difference
- **No wall-clock dependencies**: No tests rely on current time or floating-point randomness

## Phase 5 US1 Test Additions

### Substitution Skeleton Tests

New `tests/substitution_skeleton.rs` (Phase 5 / F3) verifies the skeleton module compiles and override seams are reachable:

```rust
//! Phase 5 / F3 — substitution module skeleton smoke tests.

#[test]
fn render_returns_body_unchanged_in_skeleton() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = dummy_context(tmp.path());
    let body = "Hello, world! Static body with no placeholders.";
    let out = substitution::render(body, &ctx).expect("skeleton render");
    assert_eq!(out, body);
}

#[test]
fn clock_override_guard_installs_and_clears() {
    let when = time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let _guard = ClockOverrideGuard::install(when);
    // Timestamp can be injected; guard clears on drop
}
```

**Pattern**: Framework ships with skeleton; test-injection seams are immediately available. Actual logic lands in US1/US2/US3.

### Entry-Kind Indexing Tests

New `tests/entry_kind_indexing.rs` (Phase 5 / US1.a) exercises schema v3 with kind-discriminated entries:

```rust
//! Phase 5 / US1.a — kind-discriminated entry indexing. Exercises
//! `lifecycle::enable` against a hand-rolled fixture plugin that ships
//! BOTH `skills/*/SKILL.md` and `commands/*.md`.
```

Verifies:
- Skills and commands both indexed under schema v3 with `kind` column
- Identity tuple widened to `(catalog, plugin, kind, name)`
- Content-hash composition includes `when_to_use` field
- Embedding vectors stored per-entry

### Byte-Stable Wire-Shape Pins (Phase 5 US1)

New `tests/mcp_prompts.rs` includes JSON wire-shape pins for `PromptDescriptor`:

```rust
#[test]
fn mcp_prompts_get_returns_byte_stable_json_shape() {
    // Verify exact JSON structure matches contract
    // (T-M1 from Phase 5 spec contract)
    let output = /* ... prompt info ... */;
    let serialised = serde_json::to_string(&output).unwrap();
    let expected = r#"{"name":"...","description":"...","...}"#;
    assert_eq!(serialised, expected);
}
```

Prevents accidental breaking changes in the `listChanged: false` MCP contract.

## CI Integration

### GitHub Actions Test Pipeline

| Stage | Command | Time | Blocking |
|-------|---------|------|----------|
| Format | `cargo fmt --check` | ~1s | Yes |
| Lint | `cargo clippy --all-targets -- -D warnings` | ~10s | Yes |
| Typos | `typos` | <1s | Yes |
| Unit tests | `cargo test --lib` | ~1s | Yes |
| Integration tests | `cargo test --test '*'` | ~60s | Yes |
| MSRV check | `cargo +1.93 build` | ~30s | Yes |
| Binary size | Release binary <50 MB | ~5s | Yes |

Stages are run **sequentially**. Test output is captured; failures surface in job logs.

**Key gates**: Every PR must pass all quality checks. No manual override.

## Key Testing Principles

### 1. No Mocking (Except Embedder/Reranker)

Integration tests use **real** filesystem, **real** Git operations, **real** SQLite. Only the embedder/reranker are stubbed (reason: 625 MB models + ONNX Runtime overhead).

### 2. Cross-Process CLI Isolation

CLI tests invoke the compiled binary as a subprocess. This ensures no Rust global state leakage (ctrlc handlers, statics, file descriptors) and mirrors real user invocation.

### 3. Atomicity & Interrupt Testing

Tests model SIGINT as a deliberate closure-level `Err`, not as signal flipping. Rollback assertions verify that transaction rollbacks leave the database unchanged.

### 4. Exit Code Exhaustiveness

Every `TomeError` variant has a corresponding `#[test]` in `tests/exit_codes.rs` and/or `tests/exit_codes_e2e.rs`. The closed-enum design enforces this — adding a variant will fail CI until a test is added. Polish phase added e2e exit-code tests for codes 14, 16, 17, 18, 70, 7 via real CLI invocation.

### 5. Subsystem Enum Wire Stability

The `Subsystem` enum's wire format is byte-stable and version-locked. Tests serialize/deserialize every variant and assert the JSON output matches the documented colon-separated form:

```rust
#[test]
fn every_variant_round_trips_via_documented_wire_string() {
    let cases = vec![
        (Subsystem::Embedder, "\"embedder\""),
        (Subsystem::Catalog("upstream".into()), "\"catalog:upstream\""),
    ];
    for (variant, wire) in cases {
        let serialised = serde_json::to_string(&variant).unwrap();
        assert_eq!(serialised, wire);
        let parsed: Subsystem = serde_json::from_str(wire).unwrap();
        assert_eq!(parsed, variant);
    }
}
```

This prevents accidental breaking changes in the JSON output that external tools depend on.

### 6. JSON Wire-Shape Pin Tests

Extended in Phase 5 US1: New `tests/mcp_prompts.rs` pins `PromptDescriptor` JSON shape. Whenever a data type is serialized to `--json` output, add a byte-stable test:

```rust
#[test]
fn wire_shape_is_byte_stable() {
    let actual = serde_json::to_string(&value).unwrap();
    assert_eq!(actual, r#"expected exact JSON"#);
}
```

### 7. No Brittle String Assertions

Tests assert on **exit codes** (stable) and **filesystem state** (observable), rarely on exact stdout strings (fragile to refactoring). When output is checked, use fuzzy matching:

```rust
let stderr = String::from_utf8_lossy(&output.stderr);
assert!(stderr.contains("missing"), "expected 'missing' in stderr");  // OK
assert_eq!(stderr, "exact string", "...");  // Brittle; avoid
```

### 8. Framework Integration Tests Before Production

When a framework ships (e.g., substitution skeleton at F3), integration tests verify override seams immediately. Production logic ships in later slices using the tested hooks. **Benefit**: Zero refactor of injection points across slices.

---

## What Does NOT Belong Here

- Code style rules → `CONVENTIONS.md`
- Security testing details → `SECURITY.md`
- Architecture patterns → `ARCHITECTURE.md`
- CI/CD mechanics → GitHub Actions workflow `.yml` files

---

*This document describes HOW to test. Update when testing strategy changes.*
*Last refreshed 2026-05-26 against Phase 5 US1-complete source (954 tests, 127 suites).*
