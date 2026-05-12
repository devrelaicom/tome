# Coding Conventions

> **Purpose**: Document code style, naming conventions, error handling, and common patterns.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-11

## Code Style

### Formatting Tools

| Tool | Configuration | Command |
|------|---------------|---------|
| rustfmt | `rustfmt.toml` | `cargo fmt --check` |
| Clippy (linter) | `clippy.toml` | `cargo clippy --all-targets --all-features -- -D warnings` |
| typos (spell check) | `_typos.toml` | `typos` |

### Style Rules

| Rule | Convention | Example |
|------|------------|---------|
| Edition | Rust 2024 | `edition = "2024"` in `Cargo.toml` |
| MSRV | Rust 1.93 | Declared in `Cargo.toml`; enforced in CI |
| Lints | `-D warnings` | All clippy warnings are errors in pre-commit and CI |
| Allow blocks | Justified comments required | `#[allow(dead_code)] // tests use subset of these` |
| Formatting | Automatic via rustfmt | Run before commit via lefthook |
| Typos | Spell-checked | Run in pre-commit hook |

## Naming Conventions

### Files & Directories

| Type | Convention | Example |
|------|------------|---------|
| Modules | snake_case | `src/catalog/git.rs`, `src/commands/catalog/add.rs` |
| Tests | descriptive, lowercase | `tests/catalog_add.rs`, `tests/exit_codes.rs` |
| Test fixtures | descriptive | `tests/fixtures/sample-catalog/` |
| Capabilities | feature-named | `catalog`, `config`, `paths`, `error`, `logging`, `output` |

### Code Elements

| Type | Convention | Example |
|------|------------|---------|
| Variables | snake_case | `manifest_path`, `catalog_root` |
| Constants | SCREAMING_SNAKE_CASE | `SCHEMA_URI`, `CANCELLED` |
| Functions | snake_case, verb prefix | `parse_and_validate()`, `scrub_credentials()` |
| Structs | PascalCase | `CatalogManifest`, `ManifestInvalid`, `ToolEnv` |
| Enums | PascalCase | `TomeError`, `Command`, `Mode` |
| Error variants | DescriptiveCase | `CatalogNotFound`, `ManifestInvalid`, `GitFailed` |
| Public module traits | PascalCase | Exported via stable public surface |

## Error Handling

### Error Architecture

**Closed enum pattern:** `TomeError` in `src/error.rs` is an exhaustive enumeration. Every concrete failure class is a variant. Adding a variant requires simultaneous updates to:
- `src/error.rs` (the enum)
- The `exit_code()` method (exit code mapping)
- The `category()` method (JSON error category)
- `tests/exit_codes.rs` (compile-time exhaustiveness check)
- The PRD and spec (for documentation)

This enforces the Unix principle: every failure class has a stable, documented exit code.

### Error Hierarchy

| Layer | Pattern | Location |
|-------|---------|----------|
| Module-level | `thiserror` enum | `src/error.rs` defines `TomeError` and `ManifestInvalid` |
| Cross-module context | `anyhow::Error` | Application boundary only (main.rs, tests) |
| Signal handling | `TomeError::Interrupted` | Exit code 8 (caught by `ctrlc` handler) |

### Exit Codes (Stable Contract)

| Code | Variant | Meaning |
|------|---------|---------|
| 0 | (success) | Command completed successfully |
| 1 | `Internal` | Programmer error (panic caught at top level) |
| 2 | `Usage` | Invalid usage: bad flag or argument |
| 3 | `CatalogNotFound` | Named catalog not registered |
| 4 | `CatalogAlreadyExists` | Attempt to register duplicate catalog |
| 5 | `ManifestInvalid` | Malformed `tome-catalog.toml` or `config.toml` |
| 6 | `GitFailed` | Git command failed (e.g. network error, bad ref) |
| 7 | `Io` | File I/O error (permission, disk full, etc.) |
| 8 | `Interrupted` | User pressed Ctrl+C; in-flight git processes killed |

### Error Message Style

Every error message:
- **Names the failure** ("catalog `foo` is not registered")
- **Points to the offending file** (path in error message)
- **References the schema** when relevant (`SCHEMA_URI` constant points to exact definition)
- **Suggests action** when recoverable ("try `--force` or check status.github.com")
- **Scrubs credentials** before display (URL login, SSH host, tokens, API keys)

Example:
```
error: manifest invalid: unknown field `plugins_extra` in /path/to/tome-catalog.toml: see https://github.com/.../catalog-manifest.schema.toml
```

### Credential Scrubbing

`catalog::git::scrub_credentials()` is applied to all captured `git` process output (stderr, stdout) before it reaches:
- `tracing` logs
- `anyhow::Error` messages
- Any display path in `--json` output

Four-layer scrubbing strategy (in order):
1. URL logins: `https://user:pass@host/` → `https://<host>/`
2. SSH logins: `git@host.com:` → `git@<host>:`
3. Key-value secrets: `token=<token>` → `token=<scrubbed>`
4. Long hex blobs: bare ≥40-char hex → `<scrubbed>` (but preserve `name=<hex>`)

## Common Patterns

### Module Structure (Capability-Organized)

```rust
src/
├── main.rs              // CLI entry point: parse → dispatch → handle errors → exit
├── cli.rs               // clap derive definitions (global --json, -v/-vv)
├── lib.rs               // Library surface (for integration tests)
├── commands/            // Command implementations
│   ├── mod.rs
│   └── catalog/         // `tome catalog` subcommands
│       ├── mod.rs
│       ├── add.rs
│       ├── remove.rs
│       ├── list.rs
│       ├── show.rs
│       ├── update.rs
│       └── source.rs
├── catalog/             // Catalog manifest + storage + git operations
│   ├── mod.rs
│   ├── manifest.rs      // Parser + validator (strict TOML)
│   ├── store.rs         // Registry persistence (atomic writes)
│   └── git.rs           // Git shell-outs + credential scrubbing + signal handling
├── config.rs            // config.toml schema + load/save
├── error.rs             // Closed TomeError enum + ManifestInvalid
├── output.rs            // Human/--json formatter, NO_COLOR, TTY detection
├── logging.rs           // tracing-subscriber wiring (stderr-only)
└── paths.rs             // XDG-aware paths

tests/
├── common/mod.rs        // Fixture builder, ToolEnv, helpers
├── catalog_add.rs       // Integration tests for `tome catalog add`
├── catalog_list.rs      // Integration tests for `tome catalog list`
├── catalog_remove.rs    // Integration tests for `tome catalog remove`
├── catalog_show.rs      // Integration tests for `tome catalog show`
├── catalog_update.rs    // Integration tests for `tome catalog update`
├── exit_codes.rs        // Exhaustiveness check: every TomeError → exit code
├── error_messages.rs    // Error message correctness
├── manifest_strictness.rs  // TOML deny_unknown_fields enforcement + corpus
├── path_validation.rs    // Path escape/traversal validation
├── scrubbing.rs         // Credential scrubbing regex coverage
├── atomicity.rs         // Registry/cache write atomicity under interruption
└── fixtures/            // Real Git repos (file:// sources for tests)
    └── sample-catalog/
```

### TOML Deserialization (Strict)

Every struct that derives `serde::Deserialize` carries `#[serde(deny_unknown_fields)]`:

```rust
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]  // Catch typos, misnamed keys
pub struct CatalogManifest {
    pub name: String,
    pub description: String,
    // ...
}
```

This is enforced by `tests/manifest_strictness.rs`, which parses the source and asserts that every `Deserialize`-deriving struct has the attribute. Violation causes compile failure in that test.

**Why:** Silent acceptance of unknown fields hides user typos (e.g. `plugin_source` instead of `plugins[].source`). By rejecting unknown fields at parse time, we force the user to read the schema and understand the correct structure.

### Process Execution (Git)

All `git` commands are invoked via `std::process::Command` (no `libgit2`):

```rust
let status = Command::new("git")
    .args(&["clone", url, path])
    .current_dir(work_dir)
    .env("GIT_AUTHOR_NAME", "Tome Test")
    .status()
    .map_err(|e| TomeError::GitFailed { catalog, detail: e.to_string() })?;
```

**Why:** Every developer machine has `git`. Shelling out avoids a large binary dependency and lets us inherit the user's configured credentials, SSH keys, and GPG signing.

Signal handling: `ctrlc::set_handler` flips `CANCELLED` atomic bool. Operations check `was_cancelled()` and kill spawned processes before returning `TomeError::Interrupted` (exit code 8).

### Comment Policy

- **Explain why, not what.** The reader knows Rust.
- **Document boundaries.** Module docstrings explain public contract.
- **Cite the spec.** Cross-reference FR/SC/R numbers where the requirement originates.
- **Flag subtle moves.** Regex patterns, error message classification logic, path normalization.

Example:
```rust
//! The closed `TomeError` enum is the single source of truth for exit codes.
//! Adding a variant here forces edits to `tests/exit_codes.rs`, FR-022 in the
//! spec, and the PRD's exit-code table — the compiler enforces the chain.
```

### Logging

`tracing` + `tracing-subscriber` write to stderr only, orthogonal to `--json` stdout.

| Directive | Effect |
|-----------|--------|
| No flag | silent (warnings and above if `RUST_LOG` is set) |
| `-v` | info level |
| `-vv` | debug level |
| `TOME_LOG` env var | overrides `-v` count |

Logs never contain user paths, credentials, or sensitive query details — all are scrubbed before `tracing::error!()` or similar.

## Git Conventions

### Commit Messages

**Format:** `type(scope): description`

Enforced by `cog verify` in the `commit-msg` lefthook.

| Type | Usage | Example |
|------|-------|---------|
| feat | New feature or capability | `feat(catalog): add --ref pinning` |
| fix | Bug fix | `fix(git): scrub URL credentials` |
| docs | Documentation-only | `docs(readme): update installation` |
| style | Formatting or whitespace | `style: wrap long lines` |
| refactor | Code restructure (no user change) | `refactor(manifest): extract validator` |
| test | Test-only changes | `test(exit_codes): add coverage` |
| chore | Dependency or tooling | `chore(deps): bump clap to 4.6` |

**Why Conventional Commits:** Powers changelog automation and lets reviewers triage diffs by intent.

### Branching

- **Trunk-based.** Short-lived feature branches off `main`.
- **Merge frequently.** Small batches; PRs under ~400 lines.
- **No rebasing onto main.** Rebase locally before pushing, then merge as-is to main.

### Pre-commit Hooks (lefthook)

Parallel execution:
- `cargo fmt --check` — format check
- `cargo clippy --all-targets --all-features -- -D warnings` — linting
- `typos` — spell check

### Commit-msg Hook

`cog verify --file {1}` — enforces conventional-commit format.

### Pre-push Hook

`cargo test --workspace` — full test suite before pushing.

## Development Workflow

### Local Setup

```sh
git clone https://github.com/devrelaicom/tome.git
cd tome
lefthook install     # One-time setup
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

All three quality gates must pass before a PR is ready.

### CI Matrix

- **Platforms:** `macos-latest`, `ubuntu-latest`
- **Toolchains:** Rust `stable`, Rust `1.93` (MSRV)
- **Required checks:** fmt, clippy, build, test
- **Weekly:** `cargo-audit` + `cargo-deny check`

Green CI on all combinations is required before merge.

---

## What Does NOT Belong Here

- Test strategies → `TESTING.md`
- Security practices → `SECURITY.md`
- Architecture patterns → `ARCHITECTURE.md`
- Technology choices → `STACK.md`

---

*This document defines HOW to write code. Update when conventions change, in the same PR that changes the code.*
