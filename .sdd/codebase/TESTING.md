# Testing Strategy

> **Purpose**: Document test frameworks, patterns, organization, and coverage requirements.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-11

## Test Framework

| Type | Framework | Configuration | Invocation |
|------|-----------|---------------|-----------|
| Unit + Integration | Rust built-in (`cargo test`) | `Cargo.toml` `[dev-dependencies]` | `cargo test` |
| All tests | Parallel runner | Default (configured by cargo) | `cargo test --workspace` |

## Running Tests

| Command | Purpose | Scope |
|---------|---------|-------|
| `cargo test` | Run all unit + integration tests | All tests in `src/` and `tests/` |
| `cargo test --test catalog_add` | Run one integration test file | File `tests/catalog_add.rs` |
| `cargo test catalog_add::` | Run tests matching pattern | All tests in `catalog_add` module |
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
├── exit_codes.rs              # Unit: exhaustiveness check on TomeError
├── error_messages.rs          # Unit: error message format correctness
├── manifest_strictness.rs     # Unit: TOML deny_unknown_fields enforcement
├── path_validation.rs         # Unit: path escape/traversal validation
├── scrubbing.rs               # Unit: credential scrubbing regex
├── atomicity.rs               # Integration: write atomicity under interruption
└── fixtures/
    └── sample-catalog/        # Real Git repo (used as file:// source)
        ├── tome-catalog.toml
        ├── plugin-a/
        └── plugin-b/
```

### Test File Location

**Separation strategy:** All tests in `tests/` directory (not co-located with source).

| Category | Location | Style |
|----------|----------|-------|
| Unit tests | `tests/{test_name}.rs` | Test one concept (parser, error path, validator) |
| Integration tests | `tests/catalog_{cmd}.rs` | Test CLI command against real fixtures |
| Shared helpers | `tests/common/mod.rs` | Fixture builders, ToolEnv, command runners |
| Test fixtures | `tests/fixtures/` | Real git repos and TOML files |

## Test Patterns

### Integration Test Pattern

Each test in `tests/catalog_*.rs` follows this flow:

1. **Build fixture** — copy sample catalog to temp dir, run `git init && git add && git commit`
2. **Create isolated environment** — temp `$HOME`, `$XDG_CONFIG_HOME`, `$XDG_DATA_HOME`
3. **Run binary** — invoke `tome` binary as a subprocess with isolated env
4. **Assert exit code** — check `.status.code()` matches expected (0, 2, 3, etc.)
5. **Assert output** — parse stdout (human or `--json`) and validate content
6. **Assert side effects** — inspect `config.toml`, cache layout, registry state

Example from `tests/catalog_add.rs`:
```rust
#[test]
fn happy_path_human_mode() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();

    let out = env
        .cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .expect("spawn");

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Added catalog `sample-experts`"));

    let config_text = std::fs::read_to_string(env.config_file()).expect("config written");
    assert!(config_text.contains("[catalogs.sample-experts]"));
}
```

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
}
```

**Why:** Tests don't pollute the host's real config or cache. Each test has its own XDG layout.

### Test Data

Test fixtures are **real Git repos** checked into `tests/fixtures/sample-catalog/`:

```
tests/fixtures/sample-catalog/
├── .git/              # Real Git repository
├── tome-catalog.toml  # Valid manifest
├── plugin-a/          # Real plugin directories (with .keep files)
└── plugin-b/
```

When tests run:
1. Fixture is copied to temp dir
2. `git init -q -b main` initializes if needed
3. `git add -A && git commit -q -m init` creates initial commit
4. Tests then clone via `file://` URL (simulating network clone)

**No mocking of git or filesystem.** Real binaries, real trees, real I/O. This catches edge cases mocks hide.

## Test Categories

### Integration Tests (by command)

| Test File | Command | Coverage |
|-----------|---------|----------|
| `catalog_add.rs` | `tome catalog add <source>` | Happy path, name override, duplicates, missing manifest, credential scrubbing |
| `catalog_list.rs` | `tome catalog list` | Empty registry, multiple catalogs, `--json` output |
| `catalog_remove.rs` | `tome catalog remove <name>` | Confirmation prompt, `--force` flag, nonexistent catalog |
| `catalog_show.rs` | `tome catalog show <name>` | Metadata display, plugin list, JSON format |
| `catalog_update.rs` | `tome catalog update [name]` | Full sync, selective sync, failure handling |

### Unit Tests (by concern)

| Test File | Coverage |
|-----------|----------|
| `exit_codes.rs` | Every `TomeError` variant maps to exit code + category; exhaustiveness check |
| `error_messages.rs` | Error messages are user-friendly and point to schema/action |
| `manifest_strictness.rs` | TOML deny_unknown_fields enforced on all Deserialize structs; bad-manifest corpus (unknown fields, missing fields, invalid semver, invalid email, path traversal, duplicates) |
| `path_validation.rs` | Relative paths only; no absolute paths, no `..`, no escape outside catalog root |
| `scrubbing.rs` | Credential scrubbing regex: URL logins, SSH hosts, tokens, API keys, long hex |
| `atomicity.rs` | Interrupted writes (SIGINT during clone) leave registry/cache in consistent state |

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
#[test] fn missing_description_rejected() { ... }
#[test] fn missing_version_rejected() { ... }
#[test] fn missing_owner_rejected() { ... }
#[test] fn missing_owner_email_rejected() { ... }
#[test] fn non_semver_version_rejected() { ... }
#[test] fn non_email_owner_email_rejected() { ... }
#[test] fn missing_plugin_name_rejected() { ... }
#[test] fn missing_plugin_source_rejected() { ... }
#[test] fn duplicate_plugin_name_rejected() { ... }
#[test] fn malformed_toml_rejected_as_toml_parse() { ... }
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

## Coverage Strategy

No automatic coverage threshold enforced, but the test corpus is organized to be **exhaustive** per the spec:

- **Every error class is tested** — each `TomeError` variant appears in `exit_codes.rs` and often in command-specific tests
- **Bad-input corpus is explicit** — each parser/validator has a separate test file documenting what shapes are rejected
- **Integration tests hit all CLI paths** — every subcommand (`add`, `list`, `remove`, `show`, `update`) has dedicated tests
- **Edge cases are tested** — atomicity under interruption, credential scrubbing, path escapes, TOML strictness

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

The `lefthook` configuration runs these automatically. If any fails, the push is blocked and the output explains why.

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

### Testing a New CLI Command

1. Add subcommand to `src/cli.rs` (clap derive)
2. Add module under `src/commands/`
3. Create integration test file `tests/catalog_*.rs`
4. In test: build fixture, invoke binary, assert exit code + output
5. Check side effects: config file, cache layout, registry state
6. Run full test suite: `cargo test`

---

## What Does NOT Belong Here

- Code style rules → `CONVENTIONS.md`
- Security testing → `SECURITY.md`
- Architecture patterns → `ARCHITECTURE.md`

---

*This document describes HOW to test. Update when testing strategy changes, in the same PR that changes the code.*
