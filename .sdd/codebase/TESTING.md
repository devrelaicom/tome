# Testing Strategy

> **Purpose**: Document test frameworks, patterns, organization, and coverage requirements.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-13

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
├── common/mod.rs              # Shared fixtures and helpers
├── catalog_add.rs             # Integration: `tome catalog add` command
├── catalog_list.rs            # Integration: `tome catalog list` command
├── catalog_remove.rs          # Integration: `tome catalog remove` command
├── catalog_show.rs            # Integration: `tome catalog show` command
├── catalog_update.rs          # Integration: `tome catalog update` command
├── plugin_enable.rs           # Library API: `plugin::lifecycle::enable` (Phase 3)
├── plugin_list.rs             # CLI binary: `tome plugin list` (Phase 3)
├── plugin_show.rs             # CLI binary: `tome plugin show` (Phase 3)
├── query.rs                   # Library API: embed + KNN query path (Phase 3)
├── atomicity_enable.rs        # Failure-injection: enable rollback (Phase 3)
├── exit_codes.rs              # Unit: exhaustiveness check on TomeError
├── error_messages.rs          # Unit: error message format correctness
├── manifest_strictness.rs     # Unit: TOML deny_unknown_fields enforcement
├── path_validation.rs         # Unit: path escape/traversal validation
├── scrubbing.rs               # Unit: credential scrubbing regex
├── atomicity.rs               # Integration: write atomicity under interruption
└── fixtures/
    ├── sample-catalog/        # Real Git repo (used as file:// source)
    │   ├── tome-catalog.toml
    │   ├── plugin-a/
    │   └── plugin-b/
    └── sample-plugin-catalog/  # Phase 3 plugin catalog with sample plugins
        ├── tome-catalog.toml
        ├── plugin-alpha/       # Plugin with multiple skills
        └── plugin-beta/        # Plugin for query test coverage
```

### Test File Location

**Separation strategy:** All tests in `tests/` directory (not co-located with source).

| Category | Location | Style |
|----------|----------|-------|
| Unit tests | `tests/{test_name}.rs` | Test one concept (parser, error path, validator) |
| Integration tests (library API) | `tests/plugin_enable.rs`, `tests/query.rs`, `tests/atomicity_enable.rs` | Exercise library API (`tome::plugin::lifecycle::*`) with `StubEmbedder` |
| Integration tests (CLI binary) | `tests/plugin_list.rs`, `tests/plugin_show.rs` | Spawn `tome` binary as subprocess; used when no embedders are loaded |
| Shared helpers | `tests/common/mod.rs` | Fixture builders, ToolEnv, lifecycle helpers, CLI invocation |
| Test fixtures | `tests/fixtures/` | Real git repos and sample plugin catalogs |

## Test Patterns

### Library API Integration Test Pattern (Phase 3)

Tests for `plugin::lifecycle` and `index::query` drive the library API directly with a `StubEmbedder`:

1. **Build fixture** — copy sample plugin catalog to temp dir, initialize git
2. **Build paths** — plain-data `Paths` rooted at TempDir (no env mutation)
3. **Fabricate models** — write `ModelManifest` JSON so `ensure_models_present` passes
4. **Construct lifecycle deps** — include stub embedder, seed values
5. **Call library function** — e.g., `lifecycle::enable(&id, &deps)?`
6. **Assert outcome** — check return value, side effects (database rows, metadata)

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

**Why library API tests:** The `tome plugin enable` CLI command path loads `FastembedEmbedder` (real ONNX model files). The stub embedder is deterministic and lets tests run without any model artefacts.

### CLI-Binary Integration Test Pattern (Phase 3)

Tests for commands that don't load embedders (e.g., `plugin list`, `plugin show`) spawn the real binary:

1. **Build fixture** — copy plugin catalog to temp dir, initialize git
2. **Create isolated environment** — temp `$HOME`, `$XDG_CONFIG_HOME`, `$XDG_DATA_HOME`
3. **Write config** — use `write_config_for_cli` helper to bypass git fixture setup
4. **Run binary** — invoke `tome` binary as a subprocess with isolated env
5. **Assert exit code** — check `.status.code()` matches expected
6. **Assert output** — parse stdout (human or `--json`) and validate content

Used when embedders are not involved or interaction with the real binary is essential.

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

**`write_config_for_cli(paths: &Paths, config: &Config)`** — Write the supplied `Config` to `paths.config_file` as TOML so a child `tome` binary process can read it. Used by `plugin list` / `plugin show` tests that bypass `catalog add`.

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
| `catalog_show.rs` | CLI-binary | `tome catalog show <name>` — metadata display, plugin list, JSON format |
| `catalog_update.rs` | CLI-binary | `tome catalog update [name]` — full sync, selective sync, failure handling |
| `plugin_enable.rs` | Library API | `plugin::lifecycle::enable` — skill row insertion, content hash, fallbacks, atomicity (FR-004), idempotency, warnings |
| `plugin_list.rs` | CLI-binary | `tome plugin list [catalog]` — filtering by catalog, empty list, JSON format |
| `plugin_show.rs` | CLI-binary | `tome plugin show <catalog>/<plugin>` — skill details, metadata, JSON format |
| `query.rs` | Library API | KNN query + optional reranking — self-similarity, filtering, candidate pool, drift detection |
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

## Deterministic Stub Embedder (Phase 3)

**Location:** `src/embedding/stub.rs` (compiled into release binary; LTO eliminates it when unused)

**Properties:**
- **Determinism** — the same input always produces the same 384-element vector
- **Distinguishability** — different inputs produce vectors whose cosine similarity is `< 0.99`
- **Send + Sync** — safe to share across threads; uses `Arc<AtomicUsize>` for call-count tracking

**Construction:** Hash input with SHA-256, tile across 384-element vector, normalize to `[-1.0, 1.0]`, then L2-normalise.

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

## Test Organization by Concern (Phase 3)

### No Environment Mutation in Library API Tests

**Library API tests** (`plugin_enable.rs`, `query.rs`, `atomicity_enable.rs`) never touch `$HOME` or environment variables. They use `lifecycle_paths(root)` to build a plain-data `Paths` structure.

**CLI-binary tests** (`plugin_list.rs`, `plugin_show.rs`) are the *only* place env vars get touched, and that happens via `Command::env` on the spawned child.

### Test Scaffolding Lock-Step

Two parallel path builders are deliberately kept in lock-step:
1. **In-module helper:** `src/plugin/lifecycle.rs::tests::test_paths` (for unit tests within the module)
2. **Integration test helper:** `tests/common/mod.rs::lifecycle_paths` (for library API integration tests)

If one changes, the other must change too — enforced via manual code review.

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
- **Integration tests hit all CLI paths** — every subcommand (`catalog add/list/remove/show/update`, `plugin enable/list/show`) has dedicated tests
- **Library API tests exercise lifecycle** — `plugin_enable.rs` covers enable, fallbacks, warnings; `query.rs` covers KNN and reranking; `atomicity_enable.rs` covers rollback
- **Edge cases are tested** — atomicity under interruption (failure-injection), credential scrubbing, path escapes, TOML strictness, model drift

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
cargo test --workspace         # Full test suite
cargo audit                     # Security: vulnerable dependencies
cargo deny check                # License compliance
```

All checks must pass on both platforms (`ubuntu-latest`, `macos-latest`) and both toolchains (`stable`, MSRV `1.93`).

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

### Testing a New Plugin Command (Phase 3)

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
3. Use `ToolEnv`, `write_config_for_cli`
4. Spawn the binary, assert exit code + output
5. Run `cargo test plugin_*` to verify

For commands that load embedders (Phase 4+):
- CLI-only; no library API test needed
- Follow the `plugin list` / `plugin show` pattern

### Testing Query / Search (Phase 3)

1. Build fixture plugin catalog with multiple skills
2. Enable plugins via `lifecycle::enable` (stub embedder)
3. Open index via `index::open` with same stub seeds
4. Call `index::query::knn` with query vector
5. Assert hits, distances, optional reranking
6. Use `embedding_text(name, description)` to predict top-1 for self-similarity tests

---

## What Does NOT Belong Here

- Code style rules → `CONVENTIONS.md`
- Security testing → `SECURITY.md`
- Architecture patterns → `ARCHITECTURE.md`

---

*This document describes HOW to test. Update when testing strategy changes, in the same PR that changes the code.*
