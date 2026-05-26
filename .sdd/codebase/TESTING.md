# Testing Strategy

> **Purpose**: Document test frameworks, patterns, organization, and coverage requirements.
> **Generated**: 2026-05-27
> **Last Updated**: 2026-05-27
> **Phase**: 5 (commands-as-prompts + substitution layer; US2 shipped)

## Test Framework

| Type | Framework | Configuration |
|------|-----------|---------------|
| Unit | Rust built-in (`#[test]`) | `Cargo.toml` test profile |
| Integration | Rust built-in (`tests/` dir) | CLI binary invocation + library API |
| E2E | CLI binary + file fixtures | Real git repos, real FS isolation |

### Running Tests

| Command | Purpose |
|---------|---------|
| `cargo test` | Run all unit + integration tests (uses stub embedder; fast, no models) |
| `cargo test --test atomicity` | Single integration test file |
| `cargo test catalog_add::` | One test by path |
| `cargo test -- --test-threads=1` | Run serially (for tests with shared global state) |
| `.githooks/pre-push` | Full test suite before push (enforced by git hook) |

**CI assertion**: `cargo test` completes in <5 min on CI hardware (parallelized across CPU cores). Stub embedder keeps the CI-fast property.

## Test Organization

### Directory Structure

```
tests/
├── common/
│   └── mod.rs                         # Shared test harness (fixtures, env isolation, helpers)
├── substitution_*.rs                  # Phase 5 / US2 — substitution pipeline tests
├── harness_*.rs                       # Phase 4 / US3 — harness integration tests
├── workspace_*.rs                     # Phase 4 / US2 — workspace lifecycle tests
├── plugin_*.rs                        # Phase 3 / US1 — plugin enable/disable tests
├── catalog_*.rs                       # Phase 2 / US7 — catalog lifecycle tests
├── models_*.rs                        # Phase 3 / US4 — model download/list/remove tests
├── query.rs                           # Phase 2 / US3 — search + rerank tests
├── mcp_*.rs                           # Phase 3 / US1 — MCP server tests
├── doctor_*.rs                        # Phase 3 / US4 — diagnostic + repair tests
├── exit_codes.rs                      # All phases — exit code table coverage
├── atomicity.rs                       # All phases — interrupt-safety tests
├── sync_boundary.rs                   # Phase 3 — structural tokio-isolation test
└── fixtures/
    ├── sample-catalog/                # Minimal git repo for catalog tests
    └── sample-plugin-catalog/         # Minimal plugin catalog for lifecycle tests
```

### Test File Location Strategy

**Integration-test layout** (all tests in `tests/` directory):
- Tests consume the `tome` library without `#[cfg(test)]` visibility
- Each test file imports `common::*` for shared fixtures and helpers
- One test file per feature area (e.g., `substitution_builtins.rs`, `substitution_env.rs`)
- Test counts reported by feature: `cargo test -- --list | wc -l`

**Phase boundaries**:
- Phase 1 (catalog) → `catalog_*.rs`
- Phase 2 (plugin index) → `plugin_*.rs`, `query.rs`, `models_*.rs`
- Phase 3 (MCP) → `mcp_*.rs`, `workspace_*.rs`
- Phase 4 (refactor) → `harness_*.rs`, `workspace_*.rs`, `doctor_*.rs`
- Phase 5 (substitution) → `substitution_*.rs`

## Test Patterns

### Standard Test Structure

```rust
#[test]
fn test_description_of_the_scenario() {
    // Arrange: set up test state (fixtures, temp dirs, env)
    let env = ToolEnv::new();
    let fixture = Fixture::build_sample();
    
    // Act: invoke the code under test
    let outcome = command::operation(&args, &env)?;
    
    // Assert: verify the outcome
    assert_eq!(outcome.status, Status::Success);
    assert!(outcome.path.exists());
}
```

**Naming**: `test_<verb>_<subject>_when_<condition>` (e.g., `test_enable_plugin_when_already_enabled_skips_embedder`).

### Library API Testing

When code has both a library entry point AND a CLI wrapper, test the library separately from the CLI:

```rust
#[test]
fn library_api_path() {
    // Call the silent-compute function directly (no emit)
    let outcome = commands::query::pipeline(&args, &deps)?;
    assert_eq!(outcome.skill_count, 3);
}

#[test]
fn cli_binary_path() {
    // Invoke the CLI binary via Command::new()
    let env = ToolEnv::new();
    let output = env.cmd()
        .arg("query")
        .arg("--help")
        .output()
        .expect("spawn");
    assert!(output.status.success());
}
```

**Pattern established**: Phase 3 / US1.b (query refactoring); refined Phase 4 across all commands.

### Environment Isolation

Every integration test uses a fresh, isolated `HOME`:

```rust
#[test]
fn test_uses_isolated_home() {
    let env = ToolEnv::new();  // Fresh TempDir per test
    let paths = Paths::from_home(env.home_path());
    // Test against isolated paths; host config never touched
}
```

**Global state serialization** (Phase 5 / US2):
- `HOME_MUTEX` serialises `HomeGuard` installations across parallel tests
- Per-file `ENV_MUTEX` serialises arbitrary env-var mutations (mirrors `OVERRIDE_MUTEX`)
- `OVERRIDE_MUTEX` serialises `HarnessModulesGuard` / substitution-layer overrides

### Test Injection Seams

For infrastructure that doesn't have a parameterized dependency injection path, use test-only `#[doc(hidden)] pub static` injection slots:

```rust
// In src/substitution/mod.rs:
#[doc(hidden)]
pub static SUBSTITUTION_CLOCK_OVERRIDE: OnceLock<Mutex<Option<OffsetDateTime>>> = 
    OnceLock::new();

// In tests/common/mod.rs:
pub struct ClockOverrideGuard;
impl ClockOverrideGuard {
    pub fn install(when: OffsetDateTime) -> Self {
        *tome::substitution::SUBSTITUTION_CLOCK_OVERRIDE
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(when);
        Self
    }
}

// In tests/substitution_builtins.rs:
#[test]
fn test_with_deterministic_clock() {
    let _clock = ClockOverrideGuard::install(FROZEN_TIME);
    // Clock reads now return FROZEN_TIME
}
```

Pattern established by Phase 3 / F7 (migrations); extended Phase 4 (harness modules); widened Phase 5 / US2 (substitution layer).

### RAII Guards for Test Isolation

Use `Drop` trait for clean teardown on panic:

```rust
let _home_guard = HomeGuard::install(env.home_path());
let _env_guard = EnvVarGuard::install("MY_VAR", "test_value");
let _clock_guard = ClockOverrideGuard::install(FROZEN_TIME);
// On function exit (success or panic), guards drop in LIFO order
// and restore previous state, even if test panicked.
```

**Poisoned-mutex recovery** (Phase 4 / P5 retro):
```rust
let lock = HOME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
```
A panic in one test must not cause setup failures in the next.

### Stage-Isolation Testing (Phase 5 / US2 Pattern)

When testing a pipeline with multiple logical stages (built-ins, env passthrough, arguments), test each stage independently BEFORE testing the full pipeline:

```rust
// Stage 1: built-ins only
#[test]
fn test_substitution_builtins_stage() {
    let ctx = SubstitutionContext { ... };
    let result = builtins::resolve_builtin("TOME_ENTRY_NAME", &ctx, None)?;
    assert_eq!(result, Some("hello".to_string()));
}

// Stage 2: env vars only
#[test]
fn test_substitution_env_stage() {
    let result = env::resolve_env("MY_VAR", None);
    // Env-var resolution doesn't error; returns default if missing
}

// Integration: full pipeline
#[test]
fn test_substitution_pipeline_integration() {
    let rendered = render("${TOME_ENTRY_NAME} says ${MY_VAR}", &ctx)?;
    assert_eq!(rendered, "hello says test_value");
}
```

Test files: `substitution_builtins.rs`, `substitution_env.rs`, `substitution_pipeline.rs` (Phase 5 / US2.d structure).

### Fixture-Based Testing

For catalog + plugin tests, copy pre-built fixtures into temp dirs:

```rust
#[test]
fn test_catalog_operation() {
    let fixture = Fixture::build_sample();  // Clones tests/fixtures/sample-catalog/
    let env = ToolEnv::new();
    
    // fixture.url is a file:// git repo the CLI can clone from
    env.cmd()
        .arg("catalog")
        .arg("add")
        .arg("test-catalog")
        .arg(fixture.url)
        .assert_success();
}
```

Fixtures are **real git repos** (initialized via `git init && git add -A && git commit`) so the CLI's `git clone` codepath is tested end-to-end.

### Sparse File Fabrication

For large model/artefact files, use sparse files (zero disk space on Linux/macOS):

```rust
let dir = paths.models_dir.join("bge-small-en-v1.5");
std::fs::create_dir_all(&dir)?;
let f = std::fs::File::create(&dir.join("model.onnx"))?;
f.set_len(45_000_000)?;  // 45 MB sparse file, ~0 KB on disk
```

Pattern established Phase 3 / US4; used in `models_download.rs`, `models_list.rs` for `--verify` checksum tests (zero bytes ≠ registry SHA-256, so `--verify` flips state to `checksum_mismatched`).

## Mocking Strategy

### Stub Embedder / Reranker / Summariser

The **closed trait set** (Phase 1–4) has stub implementations for testing:

```rust
pub trait Embedder { fn embed(&self, text: &str) -> Result<Vec<f32>>; }
pub trait Reranker { fn rerank(&self, query: &str, docs: &[&str]) -> Result<Vec<Score>>; }
pub trait Summariser { fn summarise(&self, text: &str) -> Result<String>; }

// In tests:
struct StubEmbedder;
impl Embedder for StubEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        Ok(vec![0.0; 768])  // Deterministic all-zeros vector
    }
}
```

**When to use**:
- Library API paths that call `lifecycle::enable` → use `StubEmbedder`
- CLI binary paths → never construct embedder (binary doesn't load real ONNX)
- MCP server tests → use real `FastembedEmbedder` (in-process server needs real models)

### No-op Registry Overrides

For tests that need synthetic harnesses or migrations, use `#[doc(hidden)] pub static` injection:

```rust
// Phase 4 / US3.c:
let guard = HarnessModulesGuard::install(vec![
    Box::new(NamedStubHarness::new("a")),
    Box::new(NamedStubHarness::new("x")),
]);
// resolve_effective_list now sees these synthetic harnesses

// Phase 3 / US5:
let guard = MigrationsGuard::install(&[Migration {
    from: 0,
    to: 1,
    name: "test_migration",
    apply: |_| Ok(()),
}]);
// schema-migration framework now runs test migration
```

### No Real Network / No Real Model Downloads

**CI-fast guarantee**: All tests use stub embedder or mocked infrastructure. Real model downloads / BGE inference are NOT tested in CI (would add 5–10 min and require real weights from HuggingFace).

- `models_download.rs` tests skip the real network path; CLI smoke tests cover exit codes
- `query.rs` uses `StubEmbedder` + `StubReranker`
- `mcp_*.rs` use stub embedders for tool handler tests (except where MCP actually needs real search)

**Manual verification**: Real BGE model testing is documented in retro PDFs as `SC-001` / `SC-002` for human review post-release.

## Test Organization by Phase

### Test Counts

Current suite (Phase 5 / US2 shipped):
- **954 tests** across **127 suites**
- **16 tests ignored** (marked `#[ignore]` for manual SC verification or nightly runs)
- Run time: <5 min with `cargo test` (parallel), <15 min with `.githooks/pre-push` (serial lint + test)

Phase progression:
- Phase 1: 156 tests / 25 suites
- Phase 2: 187 tests / 33 suites
- Phase 3: 374 tests / 50 suites
- Phase 4: 916 tests / 125 suites
- Phase 5 (US2): 954 tests / 127 suites

### Phase 5 / US2 Test Additions

Three new test files for substitution pipeline (stage isolation):

| File | Focus | Tests |
|------|-------|-------|
| `substitution_builtins.rs` | `{{TOME_*}}` placeholder resolution | ~20 |
| `substitution_env.rs` | `{{$VAR}}` env passthrough | ~15 |
| `substitution_data_dir.rs` | Data directory creation + override seam | ~10 |
| `substitution_pipeline.rs` | Full pipeline end-to-end | ~15 |

Each test file serializes on `OVERRIDE_MUTEX` to prevent clock/data-dir/env-var override races.

## Coverage Requirements

| Metric | Target | Mechanism |
|--------|--------|-----------|
| Exit code coverage | All Phase-N variants | `tests/exit_codes.rs` grep guard + CLI binary e2e tests |
| Error variant coverage | All `TomeError` variants | Compiler forces error mapping + test coverage |
| Library API coverage | All public surfaces | `common/` helpers reused across integration tests |
| Atomicity | SIGINT mid-transaction safety | `tests/atomicity.rs` simulates interrupts via `Err` returns |

### Coverage Exclusions

- `src/generated/` — auto-generated code (none currently)
- `src/mcp/` async runtime boilerplate — tokio internals not tested
- `build.rs` — sqlite-vec compilation (too low-level)

## Test Categories

### Smoke Tests

Minimal "does the CLI start" assertions:

```rust
#[test]
fn cli_help_succeeds() {
    ToolEnv::new()
        .cmd()
        .arg("--help")
        .assert_success();
}
```

Run before any other tests to catch startup-path regressions.

### Regression Tests

When a bug is fixed, the regression test is added FIRST (TDD discipline), then the fix is committed. Example:

```rust
#[test]
fn regression_cheap_reenable_skips_embedder() {
    // Bug: PR #X said enable-of-enabled doesn't re-embed, but it did
    // Fix: add cheap_reenable branch in lifecycle::enable
    // Test: verify embedder.call_count == 0 on second enable
}
```

### Atomicity Tests

Verify that partial operations (e.g., `git clone` partial → SIGINT) leave correct state:

```rust
#[test]
fn catalog_add_aborts_mid_git_clone() {
    // Simulate SIGINT by returning Err mid-way through git::clone
    // Verify: catalog not added to index, clone dir left clean
}
```

File: `tests/atomicity.rs` + phase-specific subfiles.

## CI Integration

### Test Pipeline

```yaml
# Enforced by .githooks/pre-push
1. cargo fmt --check
2. typos
3. cargo clippy --all-targets --all-features -- -D warnings
4. cargo test (all 954 tests in parallel)
```

**Blocking checks**: All four gates must pass before push is allowed.

### Binary Size Check

On Linux CI, after `cargo build --release`:
```bash
stat -c%s target/release/tome  # Asserts < 50 MB (NFR-001)
```

Current: ~26 MiB on macOS arm64 (well under cap).

## Test Infrastructure

### ToolEnv Helper

```rust
pub struct ToolEnv {
    pub home: TempDir,
}

impl ToolEnv {
    pub fn new() -> Self { /* isolated HOME */ }
    pub fn cmd(&self) -> Command { /* pre-configured Command */ }
}
```

Every test gets a fresh `HOME` → test isolation, zero host-config pollution.

### Paths Helper

```rust
pub fn lifecycle_paths(root: &Path) -> Paths {
    Paths::from_root(root.to_path_buf())
}
```

Parameterized `$HOME` → tests never mutation env vars; all isolation via temp dirs.

### Registry Seeds

```rust
pub fn stub_embedder_seed() -> MetaSeed {
    MetaSeed { name: "stub-embedder".into(), version: "0".into() }
}
```

When tests seed the central DB, they use matching `stub_*_seed()` values so the CLI binary later opens and sees consistent identities.

## Deferred Test Coverage

Documented in per-phase retroactive analysis documents:

- **T088**: Manual SC-001 / SC-002 verification against real BGE models (done post-release)
- **T093–T095**: MCP protocol purity, latency, SIGINT graceful shutdown (need real model override or stub injection)
- **T416**: Security hardening audit (all `tome`-owned writes emit 0o600 on Unix)
- **T419**: Binary size tracking (`RELEASE-BINARY-SIZE.md` recorded on each major phase)

---

## What Does NOT Belong Here

- Code style rules → `CONVENTIONS.md`
- Security testing details → `SECURITY.md`
- Architecture patterns → `ARCHITECTURE.md`

---

*This document describes HOW to test. Update when testing strategy changes or new patterns are established.*
