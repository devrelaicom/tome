# Testing Strategy

> **Purpose**: Document test frameworks, patterns, organization, and coverage requirements.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-14

## Test Framework

| Type | Framework | Configuration | Invocation |
|------|-----------|---------------|-----------|
| Unit + Integration | Rust built-in (`cargo test`) | `Cargo.toml` `[dev-dependencies]` | `cargo test` |
| All tests | Parallel runner | Default (configured by cargo) | `cargo test --workspace` |

## Running Tests

| Command | Purpose | Scope |
|---------|---------|-------|
| `cargo test` | Run all unit + integration tests | All tests in `src/` and `tests/` |
| `cargo test --test plugin_enable` | Run one integration test file | File `tests/plugin_enable.rs` |
| `cargo test plugin_enable::` | Run tests matching pattern | All tests in `plugin_enable` module |
| `cargo test -- --nocapture` | Run with stdout/stderr visible | Useful for debugging output |
| `cargo test -- --test-threads=1` | Run tests sequentially | For debugging race conditions |

## Test Organization

### Directory Structure

```
tests/
├── common/mod.rs                  # Shared fixtures and helpers
├── catalog_add.rs                 # Integration: `tome catalog add` command
├── catalog_list.rs                # Integration: `tome catalog list` command
├── catalog_remove.rs              # Integration: `tome catalog remove` command
├── catalog_remove_cascade.rs      # Integration: cascade disable (Phase 9)
├── catalog_show.rs                # Integration: `tome catalog show` command
├── catalog_update.rs              # Integration: `tome catalog update` command
├── catalog_update_reindex.rs      # Library API: catalog update reindex path (Phase 7)
├── plugin_enable.rs               # Library API: `plugin::lifecycle::enable` (Phase 3)
├── plugin_disable.rs              # CLI binary: `tome plugin disable` (Phase 5)
├── plugin_list.rs                 # CLI binary: `tome plugin list` (Phase 3)
├── plugin_show.rs                 # CLI binary: `tome plugin show` (Phase 3)
├── plugin_interactive.rs          # PTY-driven: `tome plugin` interactive browse (Phase 4)
├── plugin_repeated.rs             # FR-008: enable/disable idempotency edge case (Phase 5)
├── models_download.rs             # CLI binary: `tome models download` (Phase 6)
├── models_list.rs                 # CLI binary: `tome models list` (Phase 6)
├── models_remove.rs               # CLI binary: `tome models remove` (Phase 6)
├── query.rs                       # Library API: embed + KNN query path (Phase 3)
├── reindex.rs                     # Library + CLI: `tome reindex` (Phase 7)
├── status.rs                      # Library API: `assemble_report` (Phase 8)
├── version_output.rs              # Compile-time content tests (Phase 8)
├── mcp_server.rs                  # MCP tool router + handler introspection (Phase 3)
├── mcp_lifecycle.rs               # MCP pre-flight exit codes (Phase 3)
├── atomicity_enable.rs            # Failure-injection: enable rollback (Phase 3)
├── exit_codes.rs                  # Unit: exhaustiveness check on TomeError
├── error_messages.rs              # Unit: error message format correctness
├── manifest_strictness.rs         # Unit: TOML deny_unknown_fields enforcement
├── path_validation.rs             # Unit: path escape/traversal validation
├── scrubbing.rs                   # Unit: credential scrubbing regex
├── atomicity.rs                   # Integration: write atomicity under interruption
└── fixtures/
    ├── sample-catalog/            # Real Git repo (used as file:// source)
    │   ├── tome-catalog.toml
    │   ├── plugin-a/
    │   └── plugin-b/
    └── sample-plugin-catalog/     # Phase 3 plugin catalog with sample plugins
        ├── tome-catalog.toml
        ├── plugin-alpha/          # Plugin with multiple skills
        └── plugin-beta/           # Plugin for query test coverage
```

### Test File Location

**Separation strategy:** All tests in `tests/` directory (not co-located with source).

| Category | Location | Style |
|----------|----------|-------|
| Unit tests | `tests/{test_name}.rs` | Test one concept (parser, error path, validator) |
| Integration tests (library API) | `tests/plugin_enable.rs`, `tests/query.rs`, `tests/reindex.rs`, `tests/catalog_update_reindex.rs`, `tests/status.rs` | Exercise library API with `StubEmbedder`, bypassing `Paths::resolve` + `FastembedEmbedder::load` |
| Integration tests (CLI binary) | `tests/plugin_list.rs`, `tests/plugin_show.rs`, `tests/plugin_disable.rs`, `tests/models_*.rs`, `tests/reindex.rs` (parse-error tests), `tests/catalog_remove_cascade.rs` | Spawn `tome` binary as subprocess; used when no embedders are loaded |
| Integration tests (PTY-driven) | `tests/plugin_interactive.rs` | Scripted pty session with `rexpect`; driven via real terminal I/O |
| Integration tests (MCP handler-level) | `tests/mcp_server.rs` | Call handler `async fn` directly inside `tokio::runtime::Builder::new_current_thread()` block (Phase 3) |
| Integration tests (MCP lifecycle) | `tests/mcp_lifecycle.rs` | CLI-binary tests for MCP pre-flight exit codes (Phase 3) |
| Compile-time content tests | `tests/version_output.rs` | Read `MODEL_REGISTRY` at compile time; assert output matches pinned models (Phase 8) |
| Shared helpers | `tests/common/mod.rs` | Fixture builders, ToolEnv, lifecycle helpers, `paths_for`, sparse-file fixtures (Phase 6) |
| Test fixtures | `tests/fixtures/` | Real git repos and sample plugin catalogs |
| In-module unit tests | `src/{module}/log.rs::tests` | Small filesystem operations (rotation, permission, idempotent no-ops) (Phase 3 / F8) |

## Test Patterns

### MCP Handler-Level Integration Test Pattern (Phase 3 / F2)

Tests for MCP tool handlers call the handler `async fn` directly inside a `tokio::runtime::Builder::new_current_thread()` context, avoiding a full MCP handshake (which would require real BGE models or a complex stub injection point).

Pattern:
1. **Build minimal state** — `McpState` with `StubEmbedder`, `StubReranker`, paths rooted in temp dir
2. **Construct tokio runtime** — single-threaded `new_current_thread()` with all features enabled
3. **Call handler async fn** — e.g., `search_skills::handle(state, input).await`
4. **Assert output** — check return value, error codes, structured JSON `data` field

**Tool router introspection (FR-108):** The `#[tool_router(vis = "pub")]` macro makes `Server::tool_router()` callable from integration tests. Use it to assert:
- Exactly the right tools are advertised
- Descriptions match contract wording
- No specific catalog/plugin/skill names leak into descriptions

Example from `tests/mcp_server.rs`:
```rust
#[test]
fn router_advertises_exactly_two_tools() {
    let names: Vec<String> = Server::tool_router()
        .list_all()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    assert_eq!(
        names,
        vec!["get_skill".to_string(), "search_skills".to_string()],
    );
}
```

**Error handling tests:** Handlers validate inputs and produce structured error codes:
- `plugin_without_catalog` when `plugin` is set but `catalog` is not
- `unknown_catalog` when the catalog is not in scope
- `unknown_skill` when a skill address is invalid
- Bounds checks on numeric args (e.g., `top_k` 1..=100)

Example from `tests/mcp_server.rs`:
```rust
let input = search_skills::Input {
    query: "find a tool".into(),
    top_k: 101,  // Out of bounds
    catalog: None,
    plugin: None,
};
let result = search_skills::handle(state, input).await;
assert!(result.is_err());
let err = result.unwrap_err();
assert_eq!(err.code, -32602);  // INVALID_PARAMS
// Check that error.data has structured code:
let data = err.data.as_ref().and_then(|d| d.get("code"));
```

### Library API Integration Test Pattern for Async (Phase 3 / US1)

When testing MCP tool handlers that call `spawn_blocking` for sync operations (index reads, model loading), set up the tokio runtime explicitly:

```rust
#[test]
fn test_search_skills_with_blocking_operations() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    
    let result = rt.block_on(async {
        let state = build_state(&env);
        search_skills::handle(state, input).await
    });
    
    assert!(result.is_ok());
}
```

This pattern allows tests to exercise the full handler pipeline without spawning an actual MCP server or loading 345 MB ONNX models in CI.

### Library API Integration Test Pattern (Phase 3–8)

Tests for `plugin::lifecycle`, `index::query`, `commands::reindex`, `commands::catalog::update`, and `commands::status` drive the library API directly with a `StubEmbedder`. This avoids loading real ONNX models in CI.

Pattern:
1. **Build fixture** — copy sample plugin catalog to temp dir, initialize git
2. **Build paths** — plain-data `Paths` rooted at TempDir via `lifecycle_paths(root)` (no env mutation)
3. **Fabricate models** — write `ModelManifest` JSON so `ensure_models_present` passes
4. **Construct lifecycle deps** — include stub embedder, seed values
5. **Call library function** — e.g., `lifecycle::enable(&id, &deps)?` or `assemble_report(&paths, false)?`
6. **Assert outcome** — check return value, side effects (database rows, metadata, report fields)

**Phase 8 addition:** Status report is testable via `assemble_report(&paths, verify)` (library API); the `run()` wrapper adds `std::process::exit(1)` for non-Ok cases. Tests call `assemble_report` directly:

```rust
#[test]
fn status_reports_healthy_when_models_ok() {
    let paths = lifecycle_paths(tmp.path());
    fabricate_models(&paths);
    // ... setup index with valid data ...

    let report = assemble_report(&paths, false).expect("status should succeed");
    assert_eq!(report.overall, OverallHealth::Ok);
    assert_eq!(report.embedder.state, "ok");
}
```

**Phase 7 addition:** Commands that batch-reindex (`tome catalog update` via `reindex_catalog_plugins`, `tome reindex` via `run_with_deps`) now expose library entry points:

```rust
// From tests/catalog_update_reindex.rs
let outcome = reindex_catalog_plugins(&name, &enabled, &deps)?;
assert_eq!(embedder.call_count(), expected);

// From tests/reindex.rs
let agg = run_with_deps(Scope::All, &plugins, &deps, false, Mode::Json)?;
```

Example from `tests/plugin_enable.rs`:
```rust
#[test]
fn enable_inserts_skill_rows_with_content_hash_and_enabled_flag() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);

    let embedder = StubEmbedder::new();
    let deps = LifecycleDeps {
        paths: &paths,
        config: &config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        allow_model_download: false,
    };
    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();

    let outcome = lifecycle::enable(&id, &deps).expect("enable should succeed");

    assert_eq!(outcome.summary.total_skills, 4);
    // ... assertions on outcome + database state
}
```

### CLI-Binary Integration Test Pattern (Phase 3–9)

Tests for commands that don't load embedders (e.g., `plugin list`, `plugin show`, `plugin disable`, `models list`, `models remove`, `status` read-only report, `catalog remove --force`) spawn the real binary.

Pattern:
1. **Build fixture** — copy plugin catalog to temp dir, initialize git
2. **Create isolated environment** — temp `$HOME`, `$XDG_CONFIG_HOME`, `$XDG_DATA_HOME`
3. **Write config** — use `write_config_for_cli` helper to bypass git fixture setup
4. **Run binary** — invoke `tome` binary as a subprocess with isolated env
5. **Assert exit code** — check `.status.code()` matches expected
6. **Assert output** — parse stdout (human or `--json`) and validate content

**Phase 9 addition:** `catalog remove --force` cascade path uses the CLI binary. The enable phase uses library API (`StubEmbedder`) to avoid loading real models, but the entire remove flow — including the cascade delete via `cascade_disable_for_catalog` — runs through the CLI binary since it's pure deletion with no embedder construction.

Example from `tests/catalog_remove_cascade.rs`:

```rust
#[test]
fn force_cascades_disable_and_removes_catalog() {
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    write_config_for_cli(&paths, &config);
    let embedder = StubEmbedder::new();
    enable_alpha(&paths, &config, &embedder);

    let out = env
        .cmd()
        .args(["--json", "catalog", "remove", "sample-plugin-catalog", "--force"])
        .output()
        .unwrap();
    assert!(out.status.success());

    // JSON record includes the cascade array.
    let v: Value = serde_json::from_slice(&out.stdout).expect("parse JSON");
    let cascade = v["removed"]["cascade"].as_array().expect("cascade array");
    assert!(!cascade.is_empty());
    assert_eq!(cascade[0]["plugin"], "sample-plugin-catalog/plugin-alpha");
}
```

**Phase 8 addition:** `status` can be tested via CLI binary without embedders (it's read-only). Example from `tests/status.rs`:

```rust
#[test]
fn status_exit_zero_when_healthy() {
    let env = ToolEnv::new();
    setup_models(&env);
    // ... populate index via library API ...

    let out = env.cmd()
        .args(["status"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("Embedder"));
}
```

**Phase 7 parse-error tests:** `tests/reindex.rs` includes 3 CLI-binary tests that cover parse errors and early exits without needing an embedder (unknown catalog → exit 3, malformed id → exit 2, empty install → exit 0). The heavy-state embed paths use the `run_with_deps` library entry point.

Used when embedders are not involved or interaction with the real binary is essential.

### PTY-Driven Integration Test Pattern (Phase 4)

Tests for interactive flows (`tome plugin` with no subcommand) use `rexpect` to drive a real pty session:

1. **Pre-enable plugins** — use library API (`lifecycle::enable` + `StubEmbedder`) to populate the index
2. **Spawn binary under pty** — `rexpect::spawn_command()` with timeout
3. **Script the interaction** — use `send_flush()`, `press_enter()`, `press_down()` helpers
4. **Match prompts** — `sess.exp_string("Pick a catalog")` finds prompt text
5. **Assert terminal state** — exit code, final stdout/stderr, post-interaction side effects (database rows)

**Terminal I/O Contract:**
- `rexpect::PtySession::send(bytes)` does NOT flush; single-byte writes (Enter, arrow keys) hang indefinitely
  - Use explicit `sess.flush()` after each write, or wrap via helper `send_flush(sess, bytes)`
- Enter key is `\r` (0x0D carriage return), not `\n`, under crossterm raw mode
- Down arrow is ANSI escape `\x1b[B`
- `rexpect::PtySession::process()` is private; use `.process().wait()` to collect exit status
- `rexpect::process::WaitStatus` re-exports `nix::sys::wait::WaitStatus`

**Environment setup for prompts:**
- Set `NO_COLOR=1` to strip ANSI cursor-positioning noise from inquire prompts
- After `NO_COLOR`, prompt text matches exactly and substring matching with `exp_string` is reliable
- Do not exercise the bare-CLI enable path in CI (it loads `FastembedEmbedder`, ~345 MB ONNX models)
  - Instead, pre-enable plugins via library API, then drive disable/navigate the interactive flow from the CLI

Example from `tests/plugin_interactive.rs`:
```rust
#[test]
fn interactive_disable_via_scripted_session_exits_zero_and_flips_state() {
    let (env, _fixture_tmp, paths) = setup_pre_enabled("sample-plugin-catalog");

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tome"));
    cmd.arg("plugin")
        .env("HOME", env.home_path())
        .env("NO_COLOR", "1")
        .env_remove("TOME_LOG");

    let mut sess = spawn_command(cmd, Some(30_000)).expect("spawn under pty");

    // Level 1 — catalog selector
    sess.exp_string("Pick a catalog").expect("catalog prompt");
    press_enter(&mut sess);

    // Level 2 — plugin browser
    sess.exp_string("Pick a plugin").expect("plugin prompt");
    press_enter(&mut sess);

    // Level 3 — plugin view + action
    sess.exp_string("Plugin:").expect("view header");
    sess.exp_string("Action").expect("action prompt");
    press_enter(&mut sess);

    // Confirm + exit assertions
    sess.exp_string("Disable sample-plugin-catalog/plugin-alpha?").expect("confirm");
    send_flush(&mut sess, "y\r");
    sess.exp_eof().expect("clean EOF");
    let status = sess.process().wait().expect("collect status");
    assert!(matches!(status, WaitStatus::Exited(_, 0)));
}
```

### Compile-Time Content Test Pattern (Phase 8)

For output that's parameterized by compile-time constants (e.g., `--version` including `MODEL_REGISTRY` identities), read the constant at compile time and assert the output matches.

**Why:** Model bumps automatically update the assertion without manual intervention.

Example from `tests/version_output.rs`:

```rust
#[test]
fn version_output_includes_embedder_and_reranker() {
    // Read MODEL_REGISTRY at compile time
    let embedder = MODEL_REGISTRY.iter().find(|e| e.kind == ModelKind::Embedder).unwrap();
    let reranker = MODEL_REGISTRY.iter().find(|e| e.kind == ModelKind::Reranker).unwrap();
    
    // Expected output is computed from pinned models
    let expected_embedder = format!("embedder: {} {}", embedder.name, embedder.version);
    let expected_reranker = format!("reranker: {} {}", reranker.name, reranker.version);

    // Spawn the binary
    let out = Command::new(env!("CARGO_BIN_EXE_tome"))
        .arg("--version")
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains(&expected_embedder));
    assert!(stdout.contains(&expected_reranker));
}

#[test]
fn version_json_format() {
    let embedder = MODEL_REGISTRY.iter().find(|e| e.kind == ModelKind::Embedder).unwrap();
    
    let out = Command::new(env!("CARGO_BIN_EXE_tome"))
        .args(["--version", "--json"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(json["embedder"]["name"].as_str(), Some(embedder.name));
}
```

This pattern is reusable for any future output that depends on constants.

### In-Module Unit Test Pattern (Phase 3 / F8)

For small file-system operations (rename, permission changes, idempotent no-ops), add unit tests in an `#[cfg(test)] mod tests` block inside the module under test. These operations are fast, deterministic, and don't require full integration scaffolding.

Pattern:
1. **Use `TempDir`** for isolated filesystem
2. **Create artefacts** via `File::create`, `set_len`, `write`
3. **Call the function** being tested
4. **Assert filesystem state** via `metadata`, `exists`, etc.

Example from `src/mcp/log.rs::tests` (4 unit tests for rotation policy):

```rust
#[test]
fn rotate_skips_when_under_cap() {
    let dir = TempDir::new().unwrap();
    let current = dir.path().join("mcp.log");
    let prev = dir.path().join("mcp.log.1");
    let mut f = File::create(&current).unwrap();
    f.write_all(b"hello\n").unwrap();
    drop(f);

    rotate_if_oversized(&current, &prev).unwrap();
    assert!(current.exists(), "small file must stay in place");
    assert!(!prev.exists(), "rotation must not run below the cap");
}

#[test]
fn rotate_renames_when_oversized() {
    let dir = TempDir::new().unwrap();
    let current = dir.path().join("mcp.log");
    let prev = dir.path().join("mcp.log.1");
    let f = File::create(&current).unwrap();
    f.set_len(ROTATE_AT_BYTES + 1).unwrap();
    drop(f);

    rotate_if_oversized(&current, &prev).unwrap();
    assert!(!current.exists(), "oversized current must be renamed away");
    assert!(prev.exists(), "rotation must produce a .1");
}
```

**Pattern applies to:**
- Log rotation policy (skip/rename/overwrite)
- File creation and permission setting
- Idempotent operations on missing/present artefacts
- Small database or index operations that don't require a full setup

**Do not use for:**
- Complex state machines (use integration tests)
- Operations that touch embedders or the network (use integration tests with library API)
- Interactive flows (use PTY-driven tests)

### Unit Test Pattern

For parsers, validators, and error paths:

```rust
fn parse(text: &str) -> Result<CatalogManifest, ManifestInvalid> {
    let (_t, root, manifest) = write_manifest(text);
    CatalogManifest::parse_and_validate(&manifest, &root, text.as_bytes())
}

#[test]
fn unknown_field_rejected() {
    let bad = "extra_field = \"value\"\n[manifest]\n...";
    let err = parse(&bad).unwrap_err();
    assert!(matches!(err, ManifestInvalid::UnknownField { ref key, .. } if key == "extra"));
}
```

Pattern:
1. **Arrange** — set up input (fixture, manifest text, command args)
2. **Act** — call function or spawn process
3. **Assert** — check result (exit code, error type, output content)

### Error Exhaustiveness Check Pattern

`tests/exit_codes.rs` uses compiler-enforced exhaustiveness:

```rust
fn build_each_variant() -> Vec<(TomeError, i32, &'static str)> {
    vec![
        (TomeError::Internal(...), 1, "internal"),
        (TomeError::Usage(...), 2, "usage"),
        // ... one entry per variant
    ]
}

#[test]
fn exhaustive_match_compile_check() {
    fn _code_for(err: &TomeError) -> i32 {
        match err {
            TomeError::Internal(_) => 1,
            TomeError::Usage(_) => 2,
            // ... exhaustive; adding a variant breaks compile
        }
    }
}
```

If a new `TomeError` variant is added, this test fails to compile until updated. This is intentional — it enforces that exit codes are documented for every error type.

## Test Fixtures and Helpers

### Phase 8 Library-Bypass Pattern

When a command's `run()` has side effects that prevent test usage (e.g., `std::process::exit` for health checks), expose a library-API function for pure logic.

**Example:** `commands::status` separates concerns:

```rust
/// Library API: testable, pure, returns StatusReport
pub fn assemble_report(paths: &Paths, verify: bool) -> Result<StatusReport, TomeError> { ... }

/// CLI API: wraps assemble_report, adds std::process::exit(1) for non-Ok cases
pub fn run(args: StatusArgs, mode: Mode) -> Result<(), TomeError> {
    let report = assemble_report(&paths, args.verify)?;
    emit(&report, mode)?;
    if !matches!(report.overall, OverallHealth::Ok) {
        std::process::exit(1);  // Non-recoverable state
    }
    Ok(())
}
```

Tests call `assemble_report` directly; only integration tests using the CLI binary exercise the exit semantics. This pattern is the firm shape for any future introspection command with exit-code semantics tied to report content.

### Phase 7 Library Entry Point Pattern

**Purpose:** Test subcommands that load embedders without pulling real model files into CI.

**Key:** Commands expose `pub fn run_with_deps(...)` entry points that accept a pre-configured `LifecycleDeps`.

**Functions:**
- `src/commands/reindex.rs::pub fn run_with_deps(scope, plugins, deps, force, mode)` — used by `tests/reindex.rs` library tests
- `src/commands/catalog/update.rs::pub fn reindex_catalog_plugins(catalog, enabled, deps)` — used by `tests/catalog_update_reindex.rs` library tests

**Usage pattern:**
```rust
#[test]
fn reindex_all_visits_every_enabled_plugin() {
    let embedder = StubEmbedder::new();
    let deps = LifecycleDeps {
        paths: &paths,
        config: &config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        allow_model_download: false,
    };
    
    // Call library entry point; no FastembedEmbedder loaded
    let agg = run_with_deps(Scope::All, &plugins, &deps, false, Mode::Json)?;
    
    // Assert via call_count() — embedder invoked (or not)
    assert!(embedder.call_count() > 0);
}
```

This is now the established pattern for testing CLI subcommands that need an embedder. Heavy-state paths use the library entry point with `StubEmbedder`; light/error paths use the CLI binary.

### Phase 9 Test Scope (feat + tests combined)

**Boundary:** When a feature implementation + its tests totals < ~250 lines AND the feature does not require extensive code review beyond what the tests themselves provide, combine feature + test slices in one PR. Otherwise, split feature from tests.

**Example:** Phase 9 `cascade_disable_for_catalog` + its tests in `tests/catalog_remove_cascade.rs` combined to ~220 lines total. The cascade is pure deletion; the tests are straightforward CLI-binary (no embedder); review burden is light.

**Pattern:** Enable via library API + `StubEmbedder` in the test setup (avoids real model files); remove flow via CLI binary (pure deletion, no embedder construction).

### Phase 6 Sparse-File Fixture Pattern

**Purpose:** Create realistic-size test artefacts without disk I/O cost.

**Key:** Use `std::fs::File::set_len(n)` to create sparse files filled with zeros at ~no disk cost.

**Function:** `fabricate_installed_model(paths: &Paths, entry: &ModelEntry)` — writes:
1. `manifest.json` with the real metadata (name, version, size, SHA-256)
2. One sparse file per `entry.files`, sized to `entry.size_bytes` for the main artefact (e.g., model weights)
3. Other auxiliary files (tokenizer.json, config.json) as 1-byte sparse files (present + non-empty)

**Properties:**
- The 280 MB reranker fixture consumes ~zero disk on Linux and macOS
- All bytes are zero, so SHA-256 intentionally DOES NOT match the registry pinned hash
- `models list --verify` uses this to flip the state to `checksum_mismatched` (test coverage for mismatch path)

**Usage:**
```rust
let paths = paths_for(&env);
fabricate_all_installed_models(&paths);  // Populate both embedder and reranker

let out = env.cmd()
    .args(["models", "list", "--verify"])
    .output()
    .unwrap();
// Assertions on output; reranker shows checksum_mismatched
```

**Reusable for any future test** that needs realistic-size fixtures without I/O. Common patterns:
- Pre-populate installed models for `models list --verify` tests
- Mock downloaded models to test skip paths without network access

Related helper: `fabricate_all_installed_models(paths: &Paths)` — convenience for populating the entire `MODEL_REGISTRY` at once.

### Phase 5 Lifecycle Helpers (`tests/common/mod.rs`)

**`paths_for(env: &ToolEnv) -> Paths`** — **Promoted in Phase 5 to common/mod.rs.** Resolves `ToolEnv` to the same `Paths` that the spawned CLI would resolve. Previously duplicated across `plugin_list.rs`, `plugin_show.rs`, `plugin_interactive.rs`, and now used by `plugin_disable.rs`, `plugin_repeated.rs`, all `models_*.rs` tests, `catalog_remove_cascade.rs` (Phase 9), and `reindex.rs` (Phase 7) — consolidated at the 4th caller.

```rust
pub fn paths_for(env: &ToolEnv) -> Paths {
    let home = env.home_path();
    Paths {
        config_dir: home.join(".config/tome"),
        config_file: home.join(".config/tome/config.toml"),
        data_dir: home.join(".local/share/tome"),
        catalogs_dir: home.join(".local/share/tome/catalogs"),
        index_db: home.join(".local/share/tome/index.db"),
        index_lock: home.join(".local/share/tome/index.lock"),
        models_dir: home.join(".local/share/tome/models"),
    }
}
```

### Phase 3 Lifecycle Helpers (`tests/common/mod.rs`)

Added in Phase 3 to support library API tests:

**`lifecycle_paths(root: &Path) -> Paths`** — Build a `Paths` rooted entirely under `root`. Mirrors the in-module helper so integration tests never touch `$HOME` or environment variables.

```rust
pub fn lifecycle_paths(root: &Path) -> Paths {
    Paths {
        config_dir: root.join("config"),
        config_file: root.join("config/config.toml"),
        data_dir: root.join("data"),
        catalogs_dir: root.join("data/catalogs"),
        index_db: root.join("data/index.db"),
        index_lock: root.join("data/index.lock"),
        models_dir: root.join("data/models"),
    }
}
```

**`fabricate_models(paths: &Paths)`** — Write `ModelManifest` JSON for every entry in `MODEL_REGISTRY` so the model-presence gate in `lifecycle::enable` is satisfied without a real download. Mirrors the in-module helper.

```rust
pub fn fabricate_models(paths: &Paths) {
    for entry in MODEL_REGISTRY {
        let dir = paths.models_dir.join(entry.name);
        std::fs::create_dir_all(&dir).expect("create model dir");
        let manifest = ModelManifest { /* ... */ };
        let body = serde_json::to_vec_pretty(&manifest).expect("serialise manifest");
        std::fs::write(dir.join("manifest.json"), body).expect("write manifest");
    }
}
```

**`copy_sample_plugin_catalog(into: &TempDir, name: &str) -> PathBuf`** — Copy the fixture skeleton and return the catalog root path.

**`config_with_catalog(catalog_name: &str, catalog_root: &Path) -> Config`** — Build a minimal `Config` with one catalog entry. The name is recorded both as the `BTreeMap` key and the inner `CatalogEntry.name`.

**`stub_embedder_seed()` / `stub_reranker_seed()`** — Return `MetaSeed` values matching the deterministic stub embedder/reranker. Used to construct `LifecycleDeps` and open the index.

**`write_config_for_cli(paths: &Paths, config: &Config)`** — Write the supplied `Config` to `paths.config_file` as TOML so a child `tome` binary process can read it. Used by `plugin list` / `plugin show` / `plugin disable` / `models_*` / `reindex` / `catalog_remove_cascade` tests that bypass `catalog add`.

### Phase 4 Interactive Helpers (PTY pattern)

**Helper functions in `tests/plugin_interactive.rs`:**

**`send_flush(sess: &mut PtySession, bytes: &str)`** — Send bytes to pty and flush explicitly. Workaround for `rexpect::PtySession::send` not flushing; required for single-byte writes to be visible to the child.

**`press_enter(sess: &mut PtySession)`** — Send `\r` (carriage return) and flush. Equivalent to pressing Enter in raw mode.

**`press_down(sess: &mut PtySession)`** — Send ANSI escape `\x1b[B` (down arrow) and flush.

### Fixture Builder (`tests/common/mod.rs`)

```rust
pub struct Fixture {
    pub tempdir: TempDir,
    pub repo_path: PathBuf,
    pub url: String,
}

impl Fixture {
    pub fn build_sample() -> Self {
        // Copies tests/fixtures/sample-catalog/ to a temp dir,
        // runs `git init && git add -A && git commit`,
        // returns file:// URL for cloning
    }
}
```

**Why:** Tests never touch the real filesystem or actual Git repos. Each test gets a disposable fixture Git repo with a real commit history.

### Isolated Environment (`ToolEnv`)

```rust
pub struct ToolEnv {
    pub home: TempDir,  // Temp $HOME
}

impl ToolEnv {
    pub fn new() -> Self { /* create isolated env */ }
    pub fn cmd(&self) -> Command { /* return pre-configured tome binary invocation */ }
    pub fn config_file(&self) -> PathBuf { /* .config/tome/config.toml */ }
    pub fn catalogs_dir(&self) -> PathBuf { /* .local/share/tome/catalogs */ }
    pub fn data_dir(&self) -> PathBuf { /* .local/share/tome */ }
}
```

**Why:** Tests don't pollute the host's real config or cache. Each test has its own XDG layout. The `cmd()` method pre-configures the binary invocation with isolated env vars.

### Test Data

Test fixtures are **real Git repos** checked into `tests/fixtures/`:

**`tests/fixtures/sample-catalog/`** — Phase 1 catalog fixture:
```
tests/fixtures/sample-catalog/
├── .git/              # Real Git repository
├── tome-catalog.toml  # Valid manifest
├── plugin-a/          # Real plugin directories (with .keep files)
└── plugin-b/
```

**`tests/fixtures/sample-plugin-catalog/`** — Phase 3 plugin catalog fixture:
```
tests/fixtures/sample-plugin-catalog/
├── .git/              # Real Git repository
├── tome-catalog.toml  # Valid manifest
├── plugin-alpha/      # Plugin with multiple SKILL.md files
│   ├── plugin.json
│   ├── SKILL.md (skill-a)
│   ├── SKILL.md (skill-b, name fallback)
│   ├── SKILL.md (skill-c, description fallback)
│   ├── SKILL.md (skill-d, extra frontmatter fields)
│   └── SKILL.md (skill-malformed-yaml-body, FR-013c skipped)
└── plugin-beta/       # Plugin for query test coverage
    ├── plugin.json
    └── SKILL.md files
```

When tests run:
1. Fixture is copied to temp dir
2. `git init -q -b main` initializes if needed
3. `git add -A && git commit -q -m init` creates initial commit
4. Tests then clone via `file://` URL (simulating network clone)

**No mocking of git or filesystem.** Real binaries, real trees, real I/O. This catches edge cases mocks hide.

## Test Categories

### Integration Tests (by command)

| Test File | Type | Coverage |
|-----------|------|----------|
| `catalog_add.rs` | CLI-binary | `tome catalog add <source>` — happy path, name override, duplicates, missing manifest, credential scrubbing |
| `catalog_list.rs` | CLI-binary | `tome catalog list` — empty registry, multiple catalogs, `--json` output |
| `catalog_remove.rs` | CLI-binary | `tome catalog remove <name>` — confirmation prompt, `--force` flag, nonexistent catalog |
| `catalog_remove_cascade.rs` | CLI-binary | `tome catalog remove --force` with enabled plugins — refuse without force, cascade delete + JSON array (Phase 9) |
| `catalog_show.rs` | CLI-binary | `tome catalog show <name>` — metadata display, plugin list, JSON format |
| `catalog_update.rs` | CLI-binary | `tome catalog update [name]` — full sync, selective sync, failure handling |
| `catalog_update_reindex.rs` | Library API | Catalog update reindex library path — cheap-skip on unchanged skills, embedder call-count assertions (Phase 7) |
| `plugin_enable.rs` | Library API | `plugin::lifecycle::enable` — skill row insertion, content hash, fallbacks, atomicity (FR-004), idempotency, warnings, cheap-reenable (FR-006) |
| `plugin_disable.rs` | CLI-binary | `tome plugin disable <catalog>/<plugin>` — TTY gating, `--force` short-circuit, non-TTY refusal (FR-007, FR-051) |
| `plugin_list.rs` | CLI-binary | `tome plugin list [catalog]` — filtering by catalog, empty list, JSON format |
| `plugin_show.rs` | CLI-binary | `tome plugin show <catalog>/<plugin>` — skill details, metadata, JSON format |
| `plugin_interactive.rs` | PTY-driven | `tome plugin` interactive flow — catalog selector, plugin browser, plugin view, action prompts, navigation (Back, Quit), non-TTY refusal (FR-050, FR-051) |
| `plugin_repeated.rs` | Mixed (Library + CLI) | FR-008: enable-of-enabled via library API, disable-of-disabled via CLI binary for exit-21 assertion (Phase 5) |
| `models_download.rs` | CLI-binary | `tome models download [model]` — idempotent skip when installed, `--verify` checksum, JSON envelope (Phase 6) |
| `models_list.rs` | CLI-binary | `tome models list` — state enumeration (missing/ok/checksum_mismatched), `--verify` flag, JSON format (Phase 6) |
| `models_remove.rs` | CLI-binary | `tome models remove <model>` — deletion, confirmation, cascade cleanup (Phase 6) |
| `query.rs` | Library API | KNN query + optional reranking — self-similarity, filtering, candidate pool, drift detection |
| `reindex.rs` | Library + CLI | `tome reindex [<scope>]` — library-API scope variants (All, Catalog, Plugin) via `run_with_deps`, CLI parse-error paths, empty install (Phase 7) |
| `status.rs` | Library API | `assemble_report` — subsystem health checks (embedder, reranker, index, drift), overall classification (Ok/Degraded/Unhealthy) (Phase 8) |
| `version_output.rs` | Compile-time content | `--version` output includes embedder/reranker identities; `--json` format (Phase 8) |
| `mcp_server.rs` | Handler-level Library | MCP tool router introspection (tool list, descriptions); handler input-validation (error codes, bounds checks) (Phase 3) |
| `mcp_lifecycle.rs` | CLI-binary | MCP pre-flight exit codes (workspace conflict, missing index, schema version, missing models) (Phase 3) |
| `atomicity_enable.rs` | Library API | Failure-injection: `StubEmbedder::with_force_fail_after(n)` → rollback guarantee (FR-004) |

### Unit Tests (by concern)

| Test File | Coverage |
|-----------|----------|
| `exit_codes.rs` | Every `TomeError` variant maps to exit code + category; exhaustiveness check |
| `error_messages.rs` | Error messages are user-friendly and point to schema/action |
| `manifest_strictness.rs` | TOML deny_unknown_fields enforced on all Deserialize structs; bad-manifest corpus (unknown fields, missing fields, invalid semver, invalid email, path traversal, duplicates) |
| `path_validation.rs` | Relative paths only; no absolute paths, no `..`, no escape outside catalog root |
| `scrubbing.rs` | Credential scrubbing regex: URL logins, SSH hosts, tokens, API keys, long hex |
| `atomicity.rs` | Interrupted writes (SIGINT during clone) leave registry/cache in consistent state |
| In-module tests (`src/mcp/log.rs::tests`) | Log rotation policy: skip/rename/overwrite, permission setting, idempotent no-ops |

## Deterministic Stub Embedder (Phase 3–7)

**Location:** `src/embedding/stub.rs` (compiled into release binary; LTO eliminates it when unused)

**Properties:**
- **Determinism** — the same input always produces the same 384-element vector
- **Distinguishability** — different inputs produce vectors whose cosine similarity is `< 0.99`
- **Send + Sync** — safe to share across threads; uses `Arc<AtomicUsize>` for call-count tracking

**Construction:** Hash input with SHA-256, tile across 384-element vector, normalize to `[-1.0, 1.0]`, then L2-normalise.

**Call-count tracking (Phase 5):** The `call_count()` method lets tests assert the embedder was or was not invoked. Example from `cheap_reenable_after_disable_invokes_embedder_zero_times`:

```rust
let embedder = StubEmbedder::new();
// First enable — embedder invoked
lifecycle::enable(&id, &deps)?;
assert!(embedder.call_count() > 0);

let count_after_first = embedder.call_count();
// Disable → re-enable with unchanged content — embedder NOT invoked (cheap path)
lifecycle::disable(&id, &deps)?;
lifecycle::enable(&id, &deps)?;
assert_eq!(embedder.call_count(), count_after_first);  // Zero new calls
```

**Failure Injection:**

```rust
pub fn with_force_fail_after(n: usize) -> Self {
    Self {
        force_fail_after: Some(n),
        call_count: Arc::new(AtomicUsize::new(0)),
    }
}
```

The counter is shared between clones via `Arc<AtomicUsize>` so the closure adaptation inside `enable_plugin_atomic` (which captures by reference) observes the same call count. Used in `atomicity_enable.rs` to inject mid-pipeline embedder failures and verify rollback (FR-004).

## Test Organization by Concern (Phase 3–9)

### No Environment Mutation in Library API Tests

**Library API tests** (`plugin_enable.rs`, `query.rs`, `atomicity_enable.rs`, `catalog_update_reindex.rs`, `reindex.rs`, `status.rs`) never touch `$HOME` or environment variables. They use `lifecycle_paths(root)` to build a plain-data `Paths` structure.

**CLI-binary tests** (`plugin_list.rs`, `plugin_show.rs`, `plugin_disable.rs`, `models_*.rs`, `reindex.rs` parse-error tests, `catalog_remove_cascade.rs`) are the *only* place env vars get touched, and that happens via `Command::env` on the spawned child.

**PTY-driven tests** (`plugin_interactive.rs`) mutate `env` only inside the pty spawning (via `Command::env`), not the parent process.

**MCP handler-level tests** (`mcp_server.rs`, `mcp_lifecycle.rs`) use isolated `ToolEnv` for CLI binary tests; handler-level tests use `lifecycle_paths` for pure state construction.

### Test Scaffolding Lock-Step

Two parallel path builders are deliberately kept in lock-step:
1. **In-module helper:** `src/plugin/lifecycle.rs::tests::test_paths` (for unit tests within the module)
2. **Integration test helper:** `tests/common/mod.rs::lifecycle_paths` (for library API integration tests)

If one changes, the other must change too — enforced via manual code review.

### Phase 9: Feature + Tests Combined (Optional)

When a feature implementation + its tests totals < ~250 lines AND the tests do not require specialized review, combine feature and tests in one PR slice. Otherwise, follow the default strategy of splitting feature slice from tests slice.

**Rationale:** Avoids artificial slice separation when the feature is inherently small and test burden is light.

**Example:** Phase 9 `cascade_disable_for_catalog` + `tests/catalog_remove_cascade.rs` combined. The cascade is pure deletion (~40 lines) and the test is straightforward CLI-binary (~140 lines); no complex mocking or extended testing required.

### Phase 8: Library-Bypass Pattern as Standard

Commands with side effects that prevent test usage (e.g., `std::process::exit` for health checks) now expose a library-API function for pure logic. The CLI `run()` wraps it and adds the exit semantics.

- Library API (`assemble_report`) — testable, no exit side effects
- CLI API (`run`) — adds `std::process::exit` for the appropriate status code

This is documented as the precedent for future introspection commands.

### Phase 7: Library Entry Points as the Standard

The library entry point pattern (`run_with_deps`, `reindex_catalog_plugins`) is now the established way to test CLI subcommands that need an embedder. This keeps models out of CI while still exercising the core logic.

- Heavy-state paths (logic that involves embedder invocations) → library entry point + `StubEmbedder`
- Light/error paths (parse, early exit, validation) → CLI binary

This is documented to be the precedent for future batch operations and subcommands.

### Phase 6: Sparse-File Fixtures (Universal Pattern)

The sparse-file fixture pattern is reusable for any test needing realistic-size artefacts without I/O. Phase 6 models tests established:
- **`fabricate_installed_model(paths, entry)`** — write manifest + sparse files for one model
- **`fabricate_all_installed_models(paths)`** — populate entire `MODEL_REGISTRY` at once

Usable by future tests for any large binary fixture (models, datasets, archives) where only existence and size matter, not actual content.

### Phase 5: Standard Helpers Promoted

`paths_for(env: &ToolEnv) -> Paths` was promoted to `tests/common/mod.rs` in Phase 5 after its 4th caller (`plugin_repeated.rs`). All CLI-binary tests now import it from common; consolidation complete. Phase 6 `models_*.rs` tests also use it, Phase 7 `reindex.rs` extends the pattern, and Phase 9 `catalog_remove_cascade.rs` consolidates it further, cementing it as the standard.

### YAML Frontmatter Quirk (Documented for Test Authors)

A leading colon (`: not valid yaml here`) is the most reliable way to provoke `InvalidYaml` inside otherwise-valid `---` delimiters:

```rust
let bad_frontmatter = r#"---
: not valid yaml here
---
"#;
```

This pattern is used in `tests/` when testing YAML parse error paths.

## Coverage Strategy

No automatic coverage threshold enforced, but the test corpus is organized to be **exhaustive** per the spec:

- **Every error class is tested** — each `TomeError` variant appears in `exit_codes.rs` and often in command-specific tests
- **Bad-input corpus is explicit** — each parser/validator has a separate test file documenting what shapes are rejected
- **Integration tests hit all CLI paths** — every subcommand (`catalog add/list/remove/show/update`, `plugin enable/disable/list/show`, `plugin` interactive, `models download/list/remove`, `reindex`, `status`) has dedicated tests
- **Library API tests exercise lifecycle** — `plugin_enable.rs` covers enable and cheap-reenable (FR-006), fallbacks, warnings; `query.rs` covers KNN and reranking; `atomicity_enable.rs` covers rollback; `catalog_update_reindex.rs` and `reindex.rs` cover batch reindex logic; `status.rs` covers health assessment
- **MCP tool handlers tested** — `mcp_server.rs` covers tool router introspection and handler input validation; `mcp_lifecycle.rs` covers pre-flight exit codes
- **Idempotency tested** — `plugin_repeated.rs` covers enable-of-enabled and disable-of-disabled (FR-008, exit 21)
- **Interactive flow tested end-to-end** — `plugin_interactive.rs` covers catalog selector, plugin browser, action prompts, navigation, non-TTY refusal
- **Cascade operations tested** — `catalog_remove_cascade.rs` covers refuse-on-enabled, cascade disable, JSON array envelope (Phase 9)
- **Compile-time content validated** — `version_output.rs` ensures `--version` output is synchronized with `MODEL_REGISTRY`
- **Edge cases are tested** — atomicity under interruption (failure-injection), credential scrubbing, path escapes, TOML strictness, model drift, sparse fixtures (Phase 6), batch reindex cheapness (Phase 7), health classification (Phase 8), cascade atomicity (Phase 9)

## Specimen Tests (Quality Corpus)

Each test file has a **comprehensive bad-input corpus** designed to be exhaustive per the spec.

### Manifest Strictness Corpus (`tests/manifest_strictness.rs`)

Every documented malformation is tested:

```rust
const GOOD: &str = r#"
name = "x"
description = "y"
version = "0.1.0"
[owner]
name = "n"
email = "n@e.co"
"#;

#[test] fn unknown_top_level_field_rejected() { ... }
#[test] fn unknown_owner_field_rejected() { ... }
#[test] fn unknown_plugin_field_rejected() { ... }
#[test] fn missing_name_rejected() { ... }
// ... one test per error variant
```

**One test per error variant** — ensures every documented failure is actually rejected and surfaces the correct `ManifestInvalid` type.

### Credential Scrubbing Corpus (`tests/scrubbing.rs`)

Regex-based scrubbing is validated against:
- URL logins: `https://user:pass@host/file.git`
- SSH hosts: `git@secret-host.com:repo.git`
- Key-value secrets: `token=abc123`, `Authorization: Bearer xyz`
- Long hex: `abc123...abc123` (40+ chars, bare or in context)

### Path Validation Corpus (`tests/path_validation.rs`)

Plugin sources must be:
- Relative paths only (no `/abs/path`)
- No parent traversal (no `../../escape`)
- Normalized (no extra `/./` or duplicate `/`)
- Resolved within catalog root (escape attempts rejected)

## CI Integration

### Test Pipeline

```
cargo build --release           # Full build (includes dependencies)
cargo fmt --check              # Format check
cargo clippy --all-targets -- -D warnings  # Linting
cargo test --workspace         # Full test suite (310 tests across 44 suites)
cargo audit                     # Security: vulnerable dependencies
cargo deny check                # License compliance
```

All checks must pass on both platforms (`ubuntu-latest`, `macos-latest`) and both toolchains (`stable`, MSRV `1.93`).

## Test Statistics

**Current:** 310 passed across 44 suites (as of 2026-05-14, end of Phase 3 Foundational US1):
- Unit tests (src/lib.rs): 66 (includes 4 new in-module tests for `mcp::log::tests` rotation policy)
- Integration tests (tests/): 244

Breakdown by test file:
- Library API (heavy-state logic): `plugin_enable.rs`, `query.rs`, `catalog_update_reindex.rs`, `reindex.rs`, `status.rs`, `atomicity_enable.rs`
- CLI binary (light-state / parse-error paths): `catalog_*.rs`, `plugin_list.rs`, `plugin_show.rs`, `plugin_disable.rs`, `models_*.rs`, `reindex.rs` parse tests, `mcp_lifecycle.rs`
- Handler-level (MCP introspection): `mcp_server.rs` (8 tests)
- PTY-driven (interactive flows): `plugin_interactive.rs`
- Unit (parsers, validators, error paths): `exit_codes.rs`, `error_messages.rs`, `manifest_strictness.rs`, `path_validation.rs`, `scrubbing.rs`, `atomicity.rs`, and in-module tests in `src/mcp/log.rs`

### Required Checks

| Check | Blocking | Trigger |
|-------|----------|---------|
| Format (cargo fmt) | Yes (pre-commit) | Every commit |
| Clippy (linting) | Yes (pre-commit) | Every commit |
| Build | Yes (CI) | Every push |
| Test | Yes (pre-push, CI) | Every push |
| Audit | Yes (weekly + PR) | Weekly cron + vulnerability reports |
| Deny (licenses) | Yes (weekly + PR) | Weekly cron + dependency changes |

## Test Execution Checklist

### Before Pushing

```sh
# Pre-commit (parallel)
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
typos

# Pre-push (sequential)
cargo test --workspace
```

The hook scripts in `.githooks/` run these automatically once `git config core.hooksPath .githooks` has been set in the clone. If any fails, the push is blocked and the output explains why.

### CI Validation

After push, CI runs:
1. Format check
2. Clippy (all targets, all features, `-D warnings`)
3. Build (release mode, binary size check)
4. Tests (on stable + MSRV, on ubuntu + macos)
5. Security (cargo-audit, cargo-deny)

Green on all 4 combinations is required before merge.

## Common Test Scenarios

### Testing a New Exit Code

1. Add variant to `TomeError` enum in `src/error.rs`
2. Implement `exit_code()` match arm
3. Implement `category()` match arm
4. Add entry to `build_each_variant()` in `tests/exit_codes.rs`
5. Add exhaustive-match arm in same file
6. File compiles and tests pass → done

### Testing a New TOML Field

1. Add field to struct in `src/catalog/manifest.rs` or `src/config.rs`
2. Add `#[serde(deny_unknown_fields)]` (already required)
3. Add test case to `tests/manifest_strictness.rs` verifying field is accepted
4. Add test case verifying unknown field with similar name is rejected
5. Run `cargo test manifest_strictness` to verify

### Testing a New Plugin Command (Phase 3–9)

For library API tests (no embedder loading):
1. Add module under `src/commands/plugin/`
2. Create integration test file `tests/plugin_*.rs` (library API)
3. Use `lifecycle_paths`, `fabricate_models`, `StubEmbedder`
4. Call library API directly: `lifecycle::enable`, `lifecycle::disable`, etc.
5. Assert outcome and database state
6. Run `cargo test plugin_*` to verify

For CLI tests (no embedder loading):
1. Reuse the library API test scaffolding
2. Create integration test file `tests/plugin_*.rs` (CLI binary)
3. Use `ToolEnv`, `paths_for`, `write_config_for_cli` (Phase 5)
4. Spawn the binary, assert exit code + output
5. Run `cargo test plugin_*` to verify

For interactive flows (PTY-driven):
1. Create integration test file `tests/plugin_*.rs` (PTY)
2. Pre-enable fixtures via library API (avoid loading embedders in CLI)
3. Spawn binary under pty via `rexpect::spawn_command()`
4. Use `send_flush()`, `press_enter()`, `press_down()` helpers
5. Match prompts via `sess.exp_string()`
6. Assert final state via database queries and exit code
7. Set `NO_COLOR=1` to make prompt matching reliable

For commands that load embedders (Phase 4+):
- CLI-only; no library API test needed
- Follow the `plugin list` / `plugin show` pattern

### Testing a New Models Command (Phase 6)

For CLI tests (no embedder loading):
1. Create integration test file `tests/models_*.rs` (CLI binary)
2. Use `ToolEnv`, `paths_for`, `write_config_for_cli` (Phase 5)
3. For fixtures with models present, use `fabricate_all_installed_models(paths)` (sparse-file pattern)
4. Spawn the binary, assert exit code + output
5. Run `cargo test models_*` to verify

Do not exercise the full network-download path in CI (would hit real `MODEL_REGISTRY` URLs). Test library-level download pipeline separately; CLI tests cover skip paths and JSON envelope.

### Testing a New Status/Health Command (Phase 8)

For library API tests (testable logic):
1. Create integration test file `tests/{command}.rs` (library API)
2. Use `lifecycle_paths`, `fabricate_models`, setup representative state
3. Call the library-API function directly: `assemble_report(&paths, verify)?`
4. Assert report fields, classification, and side effects
5. Run `cargo test {command}` to verify

For compile-time content tests:
1. If output is parameterized by constants (e.g., `MODEL_REGISTRY`), create a compile-time content test
2. Read constants at compile time
3. Compute expected output from constants
4. Spawn the binary and assert output matches
5. Model bumps automatically update assertions

For CLI exit-code tests:
1. Reuse the library API test scaffolding
2. Use `ToolEnv`, `paths_for`, `write_config_for_cli`
3. Spawn the binary with representative state
4. Assert exit code (0 for Ok, 1 for Degraded/Unhealthy)
5. Assert human + JSON output correctness

### Testing a New Batch Reindex Command (Phase 7)

For library API tests (heavy-state paths):
1. Create integration test file `tests/{command}_reindex.rs` (library API)
2. Use `lifecycle_paths`, `fabricate_models`, `StubEmbedder`
3. Expose a `pub fn run_with_deps(...)` entry point in the command module
4. Call the library entry point, passing `LifecycleDeps` with `StubEmbedder`
5. Assert embedder call-count to verify the cheap-skip invariant
6. Run `cargo test {command}_reindex` to verify

For CLI tests (parse/error paths):
1. Reuse the library API test scaffolding
2. Create integration test file `tests/{command}.rs` or extend existing (CLI binary)
3. Use `ToolEnv`, `paths_for`, `write_config_for_cli`
4. Spawn the binary with invalid scopes or empty install, assert exit codes
5. Run `cargo test {command}` to verify

Do not exercise the full embed path in CLI tests (would load real `FastembedEmbedder`). Parse errors and early exits use the CLI binary; heavy logic uses the library entry point.

### Testing a New Cascade Command (Phase 9)

For commands that batch-delete across multiple items (e.g., `catalog remove --force` cascade):

1. **Library API setup:** Pre-enable multiple plugins via `lifecycle::enable` + `StubEmbedder` to populate the index
2. **CLI binary test:** Drive the cascade via the CLI binary (no embedder construction needed for deletion)
3. **Isolation:** Use `ToolEnv`, `paths_for`, `write_config_for_cli`
4. **JSON validation:** If the cascade is exposed in `--json`, assert the optional array field structure:
   - Empty cascade case: field omitted (via `#[serde(skip_serializing_if = "Vec::is_empty")]`)
   - Non-empty cascade case: field present with one entry per deleted item
5. **State assertions:** Verify database rows are dropped and side effects complete

Example pattern from `tests/catalog_remove_cascade.rs`:
- Enable plugins via library API + `StubEmbedder` (setup only)
- Run CLI `catalog remove --force` (the cascade itself)
- Assert exit code, JSON envelope structure, and post-operation database state

### Testing a New MCP Tool Handler (Phase 3)

For handler-level tests (introspection + validation):
1. Create integration test file `tests/mcp_{command}.rs` (handler-level library)
2. Build minimal `McpState` with `StubEmbedder`, `StubReranker`, isolated paths
3. Build tokio runtime via `tokio::runtime::Builder::new_current_thread().enable_all()`
4. Call handler `async fn` directly: `search_skills::handle(state, input).await`
5. Assert output, error codes, and structured JSON `data` field
6. For router introspection, use `Server::tool_router().list_all()` to assert tool list and descriptions
7. Run `cargo test mcp_*` to verify

For pre-flight exit-code tests:
1. Create CLI-binary test file `tests/mcp_lifecycle.rs`
2. Use `ToolEnv`, `paths_for`, `fabricate_all_installed_models`
3. Set up conditions that trigger pre-flight failures (missing index, schema mismatch, missing models)
4. Spawn the binary with `mcp` subcommand, assert exit code
5. Run `cargo test mcp_lifecycle` to verify

### Testing Idempotency (Phase 5)

For two-state operations like enable/disable:
1. **Library API path:** Use `lifecycle::enable(&id, &deps)` twice; assert `TomeError::PluginAlreadyInState` on the second call
2. **CLI binary path:** Spawn `tome plugin disable <id> --force` twice; assert exit code 21 on the second call
3. **Mixed pattern:** Use library API for the first state transition, then CLI for the idempotent attempt (see `tests/plugin_repeated.rs` for example)

### Testing Query / Search (Phase 3)

1. Build fixture plugin catalog with multiple skills
2. Enable plugins via `lifecycle::enable` (stub embedder)
3. Open index via `index::open` with same stub seeds
4. Call `index::query::knn` with query vector
5. Assert hits, distances, optional reranking
6. Use `embedding_text(name, description)` to predict top-1 for self-similarity tests

### Testing Cheap Re-enable (Phase 5, FR-006)

Verify that re-enabling a plugin whose skill content is unchanged skips the embedder:

1. Create `StubEmbedder` instance
2. Call `lifecycle::enable(&id, &deps)` — embedder invoked, `call_count()` > 0
3. Call `lifecycle::disable(&id, &deps)` — library API (no embedder)
4. Call `lifecycle::enable(&id, &deps)` again — content hash matches, embedder NOT invoked
5. Assert `embedder.call_count()` unchanged from step 2

Example (from `plugin_enable.rs`):
```rust
let count_before = embedder.call_count();
lifecycle::disable(&id, &deps)?;
lifecycle::enable(&id, &deps)?;
assert_eq!(embedder.call_count(), count_before, "embedder should not be called on cheap re-enable");
```

### Testing Sparse Fixtures (Phase 6, Universal)

For any test needing large binary artefacts without disk I/O:

1. Call `fabricate_installed_model(paths, entry)` for one model, or
2. Call `fabricate_all_installed_models(paths)` to populate `MODEL_REGISTRY`
3. Artefacts are now present but zero-filled, so checksums intentionally mismatch
4. Use `--verify` flag to test mismatch detection path
5. Files consume ~no disk (sparse), so CI is fast even with 280 MB reranker

Example (from `models_download.rs`):
```rust
let paths = paths_for(&env);
fabricate_all_installed_models(&paths);
// Reranker is now present but checksummed-mismatched

let out = env.cmd()
    .args(["models", "list", "--verify", "--json"])
    .output()
    .unwrap();

// Assertions: reranker shows checksum_mismatched state
```

### Testing Batch Reindex Cheapness (Phase 7)

Verify that batch reindex operations skip unchanged skills:

1. Create `StubEmbedder` instance
2. Enable multiple plugins via `lifecycle::enable` — embedder invoked N times
3. Note the call count after initial setup
4. Modify one skill (change content) via direct database update (or fixture rebuild)
5. Call `reindex_catalog_plugins` or `run_with_deps(Scope::Catalog(...), ...)` — reindex only changed skills
6. Assert `embedder.call_count()` increased by ≤1 (only the changed skill)

Example (from `catalog_update_reindex.rs` and `reindex.rs`):
```rust
let embedder = StubEmbedder::new();
enable_alpha(&paths, &config, &embedder);  // call_count = N
let baseline = embedder.call_count();

// Modify one skill in the database
modify_skill_content(&paths, "skill-id", "new content");

// Reindex the catalog
let outcome = reindex_catalog_plugins("sample-plugin-catalog", &enabled, &deps)?;

// Only the changed skill should re-embed
assert_eq!(embedder.call_count() - baseline, 1);
```

---

## What Does NOT Belong Here

- Code style rules → `CONVENTIONS.md`
- Security testing → `SECURITY.md`
- Architecture patterns → `ARCHITECTURE.md`

---

*This document describes HOW to test. Update when testing strategy changes, in the same PR that changes the code.*
