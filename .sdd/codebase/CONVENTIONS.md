# Coding Conventions

> **Purpose**: Document code style, naming conventions, error handling, and common patterns.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-13

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
| Formatting | Automatic via rustfmt | Run before commit via the `.githooks/pre-commit` hook |
| Typos | Spell-checked | Run in pre-commit hook |

## Naming Conventions

### Files & Directories

| Type | Convention | Example |
|------|------------|---------|
| Modules | snake_case | `src/catalog/git.rs`, `src/commands/catalog/add.rs` |
| Subcommands | snake_case, grouped by capability | `src/commands/plugin/{enable,list,show,disable}.rs` |
| Tests | descriptive, lowercase | `tests/catalog_add.rs`, `tests/plugin_enable.rs`, `tests/atomicity_enable.rs` |
| Test fixtures | descriptive | `tests/fixtures/sample-catalog/`, `tests/fixtures/sample-plugin-catalog/` |
| Capabilities | feature-named | `catalog`, `config`, `paths`, `error`, `plugin`, `index`, `embedding` |

### Code Elements

| Type | Convention | Example |
|------|------------|---------|
| Variables | snake_case | `manifest_path`, `catalog_root`, `plugin_dir` |
| Constants | SCREAMING_SNAKE_CASE | `SCHEMA_URI`, `CANCELLED`, `VECTOR_DIM` |
| Functions | snake_case, verb prefix | `parse_and_validate()`, `scrub_credentials()`, `embed()` |
| Structs | PascalCase | `CatalogManifest`, `PluginId`, `EnableOutcome` |
| Enums | PascalCase | `TomeError`, `Command`, `Mode`, `ModelKind` |
| Error variants | DescriptiveCase | `CatalogNotFound`, `ManifestInvalid`, `PluginAlreadyInState` |
| Public module traits | PascalCase | `Embedder`, `Reranker`, exported via stable public surface |

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
| 30 | `ModelMissing` | Required embedding model not present |

*See `tests/exit_codes.rs` for the exhaustive listing of all Phase 2–5 exit codes.*

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
│   ├── mod.rs           // Dispatcher and common helpers
│   ├── catalog/         // `tome catalog` subcommands
│   │   ├── mod.rs       // Dispatcher; cross-subcommand helpers
│   │   ├── add.rs       // `tome catalog add <source>`
│   │   ├── remove.rs    // `tome catalog remove <name>`
│   │   ├── list.rs      // `tome catalog list`
│   │   ├── show.rs      // `tome catalog show <name>`
│   │   ├── update.rs    // `tome catalog update [name]`
│   │   └── source.rs    // Git fetch / update orchestrator
│   ├── plugin/          // `tome plugin` subcommands (Phase 3–5)
│   │   ├── mod.rs       // Dispatcher; cross-subcommand helpers
│   │   ├── enable.rs    // `tome plugin enable <catalog>/<plugin>` (CLI side)
│   │   ├── disable.rs   // `tome plugin disable <catalog>/<plugin>` (Phase 5)
│   │   ├── list.rs      // `tome plugin list [catalog]`
│   │   ├── show.rs      // `tome plugin show <catalog>/<plugin>`
│   │   └── interactive.rs // `tome plugin` (bare, interactive browse) (Phase 4)
│   └── query.rs         // `tome query <text>` (Phase 3)
├── catalog/             // Catalog manifest + storage + git operations
│   ├── mod.rs
│   ├── manifest.rs      // Parser + validator (strict TOML)
│   ├── store.rs         // Registry persistence (atomic writes)
│   └── git.rs           // Git shell-outs + credential scrubbing + signal handling
├── config.rs            // config.toml schema + load/save
├── error.rs             // Closed TomeError enum + ManifestInvalid
├── output.rs            // Human/--json formatter, NO_COLOR, TTY detection
├── logging.rs           // tracing-subscriber wiring (stderr-only)
├── paths.rs             // XDG-aware paths (Phase 1 + Phase 2 index/models dirs)
├── plugin/              // Plugin discovery, manifest parsing, lifecycle (Phase 2)
│   ├── mod.rs
│   ├── manifest.rs      // `plugin.json` parser (lenient)
│   ├── frontmatter.rs   // SKILL.md YAML frontmatter parser (lenient + fallbacks)
│   ├── components.rs    // Skills/agents/commands/hooks walk
│   ├── identity.rs      // `<catalog>/<plugin>` address parsing and resolution
│   └── lifecycle.rs     // Enable/disable orchestrator (library API, testable)
├── index/               // SQLite-backed skill index + vector search (Phase 2)
│   ├── mod.rs
│   ├── db.rs            // rusqlite open, WAL, busy_timeout
│   ├── schema.rs        // CREATE TABLE statements
│   ├── migrations.rs    // Forward-only migrations under advisory lock
│   ├── vec_ext.rs       // sqlite-vec extension load
│   ├── skills.rs        // CRUD on skills table; content-hash diff
│   ├── query.rs         // KNN search + reranker invocation
│   ├── meta.rs          // Drift detection (model ident mismatch)
│   ├── integrity.rs     // PRAGMA integrity_check
│   └── lock.rs          // Advisory lockfile (WAL + Tome-owned file)
├── embedding/           // Embedding model management + inference (Phase 2)
│   ├── mod.rs           // Embedder and Reranker traits
│   ├── fastembed.rs     // fastembed-rs impl (ONNX Runtime, CPU-only)
│   ├── stub.rs          // Deterministic stub (tests only, compiled out)
│   ├── registry.rs      // MODEL_REGISTRY with pinned URLs + checksums
│   ├── download.rs      // reqwest::blocking + SHA-256 + atomic persist
│   └── runtime.rs       // ort Environment setup
└── presentation/        // CLI output / progress / colours / prompts (Phase 2)
    ├── mod.rs
    ├── tables.rs        // comfy-table helpers
    ├── progress.rs      // indicatif wrappers (TTY-aware)
    ├── colour.rs        // owo-colors + NO_COLOR
    └── prompt.rs        // inquire wrappers (refuse on non-TTY)

tests/
├── common/mod.rs        // Fixture builder, ToolEnv, lifecycle helpers, paths_for (Phase 5)
├── catalog_add.rs       // Integration tests for `tome catalog add`
├── catalog_list.rs      // Integration tests for `tome catalog list`
├── catalog_remove.rs    // Integration tests for `tome catalog remove`
├── catalog_show.rs      // Integration tests for `tome catalog show`
├── catalog_update.rs    // Integration tests for `tome catalog update`
├── plugin_enable.rs     // Library API tests for `plugin::lifecycle::enable`
├── plugin_disable.rs    // CLI-binary tests for `tome plugin disable` (Phase 5)
├── plugin_list.rs       // CLI-binary tests for `tome plugin list`
├── plugin_show.rs       // CLI-binary tests for `tome plugin show`
├── plugin_interactive.rs // PTY-driven tests for `tome plugin` interactive
├── plugin_repeated.rs   // FR-008: enable/disable idempotency edge case (Phase 5)
├── query.rs             // Library API tests for query path (embed + KNN)
├── atomicity_enable.rs  // Failure-injection tests for enable rollback (FR-004)
├── exit_codes.rs        // Exhaustiveness check: every TomeError → exit code
├── error_messages.rs    // Error message correctness
├── manifest_strictness.rs  // TOML deny_unknown_fields enforcement + corpus
├── path_validation.rs    // Path escape/traversal validation
├── scrubbing.rs         // Credential scrubbing regex coverage
├── atomicity.rs         // Registry/cache write atomicity under interruption
└── fixtures/            // Real Git repos + sample plugin catalogs
    ├── sample-catalog/
    └── sample-plugin-catalog/
```

### CLI Module Pattern (Phase 3–5)

Each subcommand group lives under `src/commands/{group}/` with one file per subcommand:

- **`src/commands/{group}/mod.rs`**: Dispatcher enum, cross-subcommand helpers (public via `pub(crate)`)
- **`src/commands/{group}/{subcommand}.rs`**: Subcommand logic; public function signature is `pub fn run(args, mode) -> Result<(), TomeError>`

Cross-module reach via `super::*` within a group is acceptable; cross-group via re-export is preferred.

**Phase 5 `disable` handler:** Mirrors `enable::run` in shape. Handles confirmation prompt (TTY gating via `output::stdin_is_tty() && output::stdout_is_tty()`, `--force` short-circuit, non-TTY refusal), calls library API `lifecycle::disable`, surfaces outcome. No embedder loaded; library API handles all atomic state changes and locking (FR-005, FR-007, FR-051).

Example from `src/commands/plugin/disable.rs`:
```rust
pub fn run(args: PluginDisableArgs, mode: Mode) -> Result<(), TomeError> {
    let id = PluginId::from_str(&args.id)?;
    let paths = Paths::resolve()?;
    let config = store::load(&paths.config_file)?;
    let _ = resolve_plugin_dir(&id, &config)?;  // Fail fast on bad address
    
    if !args.force && !(output::stdin_is_tty() && output::stdout_is_tty()) {
        return Err(TomeError::NotATerminal);
    }
    
    let outcome = lifecycle::disable(&id, &deps)?;
    // Present outcome
}
```

### Interactive CLI Pattern (Phase 4)

The `tome plugin` interactive flow (no subcommand) is implemented in `src/commands/plugin/interactive.rs`:

- **TTY gating:** Check `output::stdin_is_tty() && output::stdout_is_tty()` before constructing prompts (FR-051)
- **Multi-level loop structure:** Catalog selector → plugin browser → plugin view/action prompts
- **Navigation:** Back and Quit at each level return to parent with clean redraw
- **Prompt library:** `inquire` (Select, MultiSelect, Confirm) — refuses on non-TTY
- **Action menu:** Shown status (Enabled/Disabled) and allows [Enable/Disable, Back] per current state
- **Post-action redraw:** After `enable` or `disable`, redraw the plugin view with updated status and fresh action menu

Testing pattern (PTY-driven via `rexpect`):
1. Pre-enable plugins via library API (avoids loading `FastembedEmbedder` in CLI child)
2. Spawn `tome plugin` under pty with `NO_COLOR=1` for reliable prompt matching
3. Script interaction via `send_flush()`, `press_enter()`, `press_down()`
4. Assert final state via database queries and exit code

See `tests/plugin_interactive.rs` for full test cases and helper functions.

### Plugin Lifecycle Pattern (Phase 3–5)

**Library API design:** The `plugin::lifecycle` module is testable without loading real ONNX models. It takes:
- A plugin ID (`<catalog>/<plugin>`)
- A `LifecycleDeps` struct holding references to paths, config, and an `Embedder` trait object

CLI callers construct a real `FastembedEmbedder` (in `enable` only); tests construct a deterministic `StubEmbedder`.

**Closure-injected embedder:** Inside `index::skills::enable_plugin_atomic`, the function accepts:
```rust
F: FnMut(&str) -> Result<Vec<f32>, TomeError>
```

not a trait object. Callers adapt `&dyn Embedder` at the call site via a closure.

**Lock + Result release pattern:** Outer function owns the lock; helper functions own SQL:

```rust
let lock = acquire_lock(&deps.paths.index_lock)?;
let result = enable_locked(id, &plugin_dir, &plugin_version, deps);

match result {
    Ok((summary, warnings)) => {
        lock.release()?;  // Explicitly release; surface unlock errors
        Ok(EnableOutcome { ... })
    }
    Err(e) => {
        drop(lock);  // Drop releases best-effort; suppress error
        Err(e)
    }
}
```

**Idempotency:** Both `enable` (Phase 3) and `disable` (Phase 5) detect when a plugin is already in the requested state and return `TomeError::PluginAlreadyInState` (exit code 21) per FR-008.

### Banner Skipped in JSON Mode

NDJSON consumers expect structured records on stdout and nothing on stderr unless an error occurs. Established in `commands::plugin::enable` and `commands::query`:

```rust
if mode == Mode::Human {
    writeln!(out, "Enabling {}…", id)?;
}
```

Then conditionally emit human or JSON output.

### Warnings as Vec<String>

Outcome structs carry `warnings: Vec<String>` (not a structured warning enum) so the CLI layer prints them on stderr:

```rust
pub struct EnableOutcome {
    pub plugin: PluginId,
    pub summary: EnableSummary,
    pub duration: Duration,
    pub warnings: Vec<String>,  // FR-011 / FR-012 / FR-013c notices
}
```

### Resolver Invariant

Anything resolving `<catalog>/<plugin>` goes through `lifecycle::resolve_plugin_dir`. There is exactly one such function; duplicates have been consolidated.

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

**Exceptions (Strictness Boundary, FR-013a):** Third-party inputs (`plugin.json`, `SKILL.md` YAML frontmatter) parse leniently with `#[serde(default)]` fallbacks — forward-compat with upstream additions.

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

Enforced by `cog verify` in the `commit-msg` hook (versioned under `.githooks/`).

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

### Git Hooks (`.githooks/`)

Versioned under `.githooks/`. No external hooks manager. Opt-in once per clone with `git config core.hooksPath .githooks`.

**`.githooks/pre-commit`** runs sequentially:
- `cargo fmt --check` — format check
- `typos` — spell check
- `cargo clippy --all-targets --all-features -- -D warnings` — linting

**`.githooks/commit-msg`** runs `cog verify --file "$1"` — enforces conventional-commit format.

**`.githooks/pre-push`** drains the refspec stdin git provides, then runs `cargo test --workspace` — full test suite before pushing.

## Development Workflow

### Local Setup

```sh
git clone https://github.com/devrelaicom/tome.git
cd tome
git config core.hooksPath .githooks    # One-time, wires up the hooks above
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
