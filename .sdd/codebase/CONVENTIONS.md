# Coding Conventions

> **Purpose**: Document code style, naming conventions, error handling, and common patterns.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-14

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
| Single-purpose commands | snake_case, flat | `src/commands/status.rs`, `src/commands/query.rs`, `src/commands/reindex.rs` |
| Multi-subcommand groups | snake_case dir + subcommand files | `src/commands/plugin/{enable,disable,list,show,interactive}.rs`, `src/commands/models/{download,list,remove}.rs` |
| Tests | descriptive, lowercase | `tests/catalog_add.rs`, `tests/plugin_enable.rs`, `tests/status.rs` |
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

*See `tests/exit_codes.rs` for the exhaustive listing of all Phase 2–9 exit codes.*

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
├── main.rs                   # entry: parse → dispatch → map errors → exit
├── lib.rs                    # re-exports
├── cli.rs                    # clap derive defs + global flags
├── error.rs                  # closed TomeError enum + ExitCode mapping
├── config.rs                 # config.toml (strict)
├── paths.rs                  # XDG paths (Phase 1) + index_db, models_dir, index_lock (Phase 2)
├── output.rs                 # human/--json formatter, NO_COLOR, TTY detection
├── logging.rs                # tracing-subscriber wiring
├── catalog/                  # Phase 1
│   ├── manifest.rs           # tome-catalog.toml (strict)
│   ├── store.rs              # registry persistence (atomic) — Phase 2 hooks cascade
│   └── git.rs                # git shell-outs + scrub_credentials
├── commands/
│   ├── mod.rs                # Dispatcher and common helpers
│   ├── catalog/              # `tome catalog` subcommands
│   │   ├── mod.rs            # Dispatcher; cross-subcommand helpers
│   │   ├── add.rs            # `tome catalog add <source>`
│   │   ├── remove.rs         # `tome catalog remove <name>`
│   │   ├── list.rs           # `tome catalog list`
│   │   ├── show.rs           # `tome catalog show <name>`
│   │   ├── update.rs         # `tome catalog update [name]` + reindex library entry point (Phase 7)
│   │   └── source.rs         # Git fetch / update orchestrator
│   ├── plugin/               # `tome plugin` subcommands (Phase 3–5)
│   │   ├── mod.rs            # Dispatcher; cross-subcommand helpers
│   │   ├── enable.rs         # `tome plugin enable <catalog>/<plugin>` (CLI side)
│   │   ├── disable.rs        # `tome plugin disable <catalog>/<plugin>` (Phase 5)
│   │   ├── list.rs           # `tome plugin list [catalog]`
│   │   ├── show.rs           # `tome plugin show <catalog>/<plugin>`
│   │   └── interactive.rs    # `tome plugin` (bare, interactive browse) (Phase 4)
│   ├── models/               # `tome models` subcommands (Phase 6)
│   │   ├── mod.rs            # Dispatcher; cross-subcommand helpers
│   │   ├── download.rs       # `tome models download [model]` (CLI side)
│   │   ├── list.rs           # `tome models list`
│   │   └── remove.rs         # `tome models remove <model>`
│   ├── query.rs              # `tome query <text>` (Phase 3)
│   ├── reindex.rs            # `tome reindex [<scope>]` + library entry point (Phase 7)
│   ├── status.rs             # `tome status [--verify]` — single-file, read-only (Phase 8)
│   └── workspace/            # `tome workspace` subcommands (Phase 4 / US2)
│       ├── mod.rs            # Dispatcher
│       ├── info.rs           # `tome workspace info` (Phase 4 / US2)
│       └── init.rs           # `tome workspace init` (Phase 4 / US2)
├── catalog/                  # Catalog manifest + storage + git operations
│   ├── mod.rs
│   ├── manifest.rs           # Parser + validator (strict TOML)
│   ├── store.rs              # Registry persistence (atomic writes)
│   └── git.rs                # Git shell-outs + credential scrubbing + signal handling
├── config.rs                 # config.toml schema + load/save
├── error.rs                  # Closed TomeError enum + ManifestInvalid
├── output.rs                 # Human/--json formatter, NO_COLOR, TTY detection
├── logging.rs                # tracing-subscriber wiring (stderr-only)
├── paths.rs                  # XDG-aware paths (Phase 1 + Phase 2 index/models dirs)
├── plugin/                   # Plugin discovery, manifest parsing, lifecycle (Phase 2)
│   ├── mod.rs
│   ├── manifest.rs           # `plugin.json` parser (lenient)
│   ├── frontmatter.rs        # SKILL.md YAML frontmatter parser (lenient + fallbacks)
│   ├── components.rs         # Skills/agents/commands/hooks walk
│   ├── identity.rs           # `<catalog>/<plugin>` address parsing and resolution
│   └── lifecycle.rs          # Enable/disable orchestrator (library API, testable)
├── index/                    # SQLite-backed skill index + vector search (Phase 2)
│   ├── mod.rs
│   ├── db.rs                 # rusqlite open, WAL, busy_timeout
│   ├── schema.rs             # CREATE TABLE statements
│   ├── migrations.rs         # Forward-only migrations under advisory lock
│   ├── vec_ext.rs            # sqlite-vec extension load
│   ├── skills.rs             # CRUD on skills table; content-hash diff
│   ├── query.rs              # KNN search + reranker invocation
│   ├── meta.rs               # Drift detection (model ident mismatch)
│   ├── integrity.rs          # PRAGMA integrity_check
│   └── lock.rs               # Advisory lockfile (WAL + Tome-owned file)
├── embedding/                # Embedding model management + inference (Phase 2)
│   ├── mod.rs                # Embedder and Reranker traits
│   ├── fastembed.rs          # fastembed-rs impl (ONNX Runtime, CPU-only)
│   ├── stub.rs               # Deterministic stub (tests only, compiled out)
│   ├── registry.rs           # MODEL_REGISTRY with pinned URLs + checksums
│   ├── download.rs           # reqwest::blocking + SHA-256 + atomic persist
│   └── runtime.rs            # ort Environment setup
├── mcp/                      # MCP server (Phase 3)
│   ├── mod.rs                # Server entry + lifecycle
│   ├── server.rs             # rmcp ServerHandler + tool router (Phase 3)
│   ├── state.rs              # McpState shared across tool handlers
│   ├── tools/                # Tool implementations (Phase 3)
│   │   ├── mod.rs            # Tool module docs + submodule re-exports
│   │   ├── search_skills.rs  # `search_skills` input/output + handler
│   │   └── get_skill.rs      # `get_skill` input/output + handler
│   ├── runtime.rs            # tokio::runtime setup + logging
│   ├── preflight.rs          # MCP startup pre-flight checks (scope, index, schema, drift, models)
│   └── log.rs                # JSON-lines file appender + rotation (Phase 3 / F8)
├── workspace/                # Workspace context (Phase 4 / US2)
│   ├── mod.rs
│   ├── scope.rs              # Scope enum + ScopeSource (resolution provenance)
│   ├── resolution.rs         # Workspace discovery algorithm (slice F3)
│   ├── info.rs               # `tome workspace info` (Phase 4 / US2)
│   ├── init.rs               # `tome workspace init` (Phase 4 / US2)
│   └── inventory.rs          # Workspace registry (opt-in workspaces.txt)
└── presentation/             # CLI output / progress / colours / prompts (Phase 2)
    ├── mod.rs
    ├── tables.rs             # comfy-table helpers
    ├── progress.rs           # indicatif wrappers (TTY-aware)
    ├── colour.rs             # owo-colors + NO_COLOR
    └── prompt.rs             # inquire wrappers (refuse on non-TTY)

tests/
├── common/mod.rs             # Fixture builder, ToolEnv, lifecycle helpers, paths_for (Phase 5)
├── catalog_add.rs            # Integration tests for `tome catalog add`
├── catalog_list.rs           # Integration tests for `tome catalog list`
├── catalog_remove.rs         # Integration tests for `tome catalog remove`
├── catalog_remove_cascade.rs # Integration tests for cascade disable (Phase 9)
├── catalog_show.rs           # Integration tests for `tome catalog show`
├── catalog_update.rs         # Integration tests for `tome catalog update`
├── catalog_update_reindex.rs # Library API tests for catalog update reindex path (Phase 7)
├── plugin_enable.rs          # Library API tests for `plugin::lifecycle::enable`
├── plugin_disable.rs         # CLI-binary tests for `tome plugin disable` (Phase 5)
├── plugin_list.rs            # CLI-binary tests for `tome plugin list`
├── plugin_show.rs            # CLI-binary tests for `tome plugin show`
├── plugin_interactive.rs     # PTY-driven tests for `tome plugin` interactive
├── plugin_repeated.rs        # FR-008: enable/disable idempotency edge case (Phase 5)
├── models_download.rs        # CLI-binary tests for `tome models download` (Phase 6)
├── models_list.rs            # CLI-binary tests for `tome models list` (Phase 6)
├── models_remove.rs          # CLI-binary tests for `tome models remove` (Phase 6)
├── query.rs                  # Library API tests for query path (embed + KNN)
├── reindex.rs                # Library + CLI tests for `tome reindex` (Phase 7)
├── status.rs                 # Library API tests for `assemble_report`; CLI-only for run() (Phase 8)
├── version_output.rs         # Compile-time content tests for `--version` output (Phase 8)
├── mcp_server.rs             # MCP tool router + handler introspection tests (Phase 3)
├── mcp_lifecycle.rs          # MCP pre-flight exit codes (Phase 3)
├── workspace_info.rs         # Library API tests for `workspace::info::assemble` (Phase 4 / US2)
├── workspace_init.rs         # Library API + CLI tests for `workspace::init` (Phase 4 / US2)
├── atomicity_enable.rs       # Failure-injection tests for enable rollback (FR-004)
├── exit_codes.rs             # Exhaustiveness check: every TomeError → exit code
├── error_messages.rs         # Error message correctness
├── manifest_strictness.rs    # TOML deny_unknown_fields enforcement + corpus
├── path_validation.rs        # Path escape/traversal validation
├── scrubbing.rs              # Credential scrubbing regex coverage
├── atomicity.rs              # Registry/cache write atomicity under interruption
└── fixtures/                 # Real Git repos + sample plugin catalogs
    ├── sample-catalog/
    └── sample-plugin-catalog/
```

### CLI Module Pattern (Phase 3–8)

**Multi-subcommand groups** live under `src/commands/{group}/` with one file per subcommand:

- **`src/commands/{group}/mod.rs`**: Dispatcher enum, cross-subcommand helpers (public via `pub(crate)`)
- **`src/commands/{group}/{subcommand}.rs`**: Subcommand logic; public function signature is `pub fn run(args, mode) -> Result<(), TomeError>`

Cross-module reach via `super::*` within a group is acceptable; cross-group via re-export is preferred.

The pattern is consistent across:
- `plugin` (enable, disable, list, show, interactive)
- `models` (download, list, remove)
- `catalog` (add, remove, list, update/reindex, show)
- `workspace` (info, init) — Phase 4 / US2 addition

**Single-purpose commands** (Phase 8 addition) stay as flat `src/commands/<name>.rs`:

- `src/commands/query.rs` — search subcommand (Phase 3)
- `src/commands/reindex.rs` — explicit reindex subcommand (Phase 7)
- `src/commands/status.rs` — health report subcommand (Phase 8)

Each single-file command may expose library entry points for testability (see "Library Entry Points" below).

**Phase 7 addition:** Commands that load embedders (`plugin enable`, `models download`, `catalog update`, `reindex`) now expose a second public entry point `pub fn run_with_deps(...)` that accepts a pre-configured `LifecycleDeps`. This allows tests to drive the library API with `StubEmbedder` without loading real ONNX models in CI.

Examples:
- `src/commands/reindex.rs::pub fn run_with_deps(scope, plugins, deps, force, mode)` — used by `tests/reindex.rs` library tests
- `src/commands/catalog/update.rs::pub fn reindex_catalog_plugins(catalog, enabled, deps)` — used by `tests/catalog_update_reindex.rs` library tests

**Phase 8 addition:** Commands with side effects that prevent test usage (e.g., `std::process::exit` for degraded health) expose a library-API function for pure logic (`assemble_report`), leaving the CLI `run()` for exit semantics. Example from `src/commands/status.rs`:

```rust
/// Library-API entry point: testable, no std::process::exit
pub fn assemble_report(paths: &Paths, verify: bool) -> Result<StatusReport, TomeError> { ... }

/// CLI entry point: adds std::process::exit(1) for non-Ok cases
pub fn run(args: StatusArgs, mode: Mode) -> Result<(), TomeError> {
    let report = assemble_report(&paths, args.verify)?;
    emit(&report, mode)?;
    if !matches!(report.overall, OverallHealth::Ok) {
        std::process::exit(1);  // Non-recoverable health state
    }
    Ok(())
}
```

Tests call `assemble_report` directly; the `run` wrapper is for CLI dispatch only.

**Phase 4 / US2 addition:** Commands with non-exit library entry points follow the same pattern. `workspace::info::assemble` is pure compute; `workspace::init` takes a library-only signature. CLI wrappers emit and handle exit semantics.

### --version Pre-Parse Hook (Phase 8)

The `--version` flag requires special handling:

1. **Problem:** Clap's auto-handler can't include embedder/reranker identities in the output, nor can it honour the `--json` flag.
2. **Solution:** Set `disable_version_flag = true` on the `Cli` derive in `src/cli.rs`, then intercept in `main.rs` before clap parsing:

```rust
let raw: Vec<String> = std::env::args().collect();
if raw.iter().skip(1).any(|a| a == "--version" || a == "-V") {
    let json = raw.iter().any(|a| a == "--json");
    commands::status::print_version(json);
    std::process::exit(0);
}

let cli = Cli::parse();  // Now --version won't be seen by clap
```

**Pattern:** This pre-parse hook pattern applies to any flag that needs to short-circuit clap's default behavior. For now, only `--version` uses it.

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

### Atomic Directory Creation Pattern (Phase 4 / US2)

When creating a directory structure that must be atomic (never partially visible), use the `tempfile::Builder::tempdir_in(target_root)` + `TempDir::keep()` + `std::fs::rename` pattern:

1. Create a temp directory **inside the target root** (not in `$TMPDIR`) using `tempfile::Builder::new().prefix(".dot.tmp.").tempdir_in(&absolute)` so the final rename is on the same filesystem (POSIX-atomic)
2. Write all content into the staging directory
3. Set permissions (e.g., `0o700` on Unix) on the staging directory before content lands, so the security window starts at creation
4. Rename the staging directory to the final name via `std::fs::rename`, which is atomic as a single operation
5. Only on success is the directory visible to readers

This ensures that readers never see a partial `.tome/` directory. Used in `src/workspace/init.rs` for atomic workspace initialization.

Example from `src/workspace/init.rs`:
```rust
let staging = tempfile::Builder::new()
    .prefix(".tome.tmp.")
    .tempdir_in(&absolute)
    .map_err(TomeError::Io)?;

// Write content...
std::fs::write(staging.path().join("config.toml"), config_bytes)?;

// Atomic rename (only step visible to readers)
std::fs::rename(staging.path(), &marker)?;
```

### Emit-Only Serialize Records (Phase 4 / US2)

When an outcome or report struct is emitted as JSON output but never deserialized, omit `#[serde(deny_unknown_fields)]`. These are API output records, and strict field validation is unnecessary.

- `WorkspaceInfo` — used only for `--json` emission; uses `#[derive(Serialize)]` without field strictness
- `InitOutcome` — used only for `--json` emission; uses `#[derive(Serialize)]` without field strictness
- `ScopeSource` — enum serialized for `--json` output; uses `#[serde(rename_all = "snake_case")]` to match contract values (`"flag" | "global_flag" | "env" | "cwd_walk" | "global_fallback"`)

**Rationale:** Only Tome-owned **inputs** (config, manifests) need strict deserialization. Output records are emit-only and benefit from permissive forward-compat.

### Silent Compute / Emit Wrapper Pattern (Phase 3 / US1 and Phase 4 / US2)

When a CLI command's compute path is reused by non-CLI surfaces (MCP, library API, direct calls), split into:
1. **Silent compute function** (`pipeline` or `assemble`) — performs all work, no side effects, returns pure result
2. **Emit wrapper** (`run_with_deps` or `run`) — calls the compute function, then emits per mode (Human or JSON)

Tests + alternative callers use the compute function; the CLI uses the emit wrapper.

Examples:
- `query::pipeline(args, deps) -> Result<QueryOutcome>` (silent) + `query::run_with_deps(args, deps, mode) -> Result<()>` (emit)
- `workspace::info::assemble(scope, paths) -> Result<WorkspaceInfo>` (silent); CLI emits via `run()`
- `workspace::init(target, inherit, force, paths) -> Result<InitOutcome>` (library signature, no emit); CLI emits via `run()`

### Batch Operations: Locking Patterns (Phase 7–9)

Two complementary locking patterns coexist. Choose based on operation semantics:

**Pattern 1: Per-Item Atomicity (Phase 7, `tome catalog update`)**

Each item in a batch acquires its own advisory lock, operates independently, then releases:

```rust
for id in enabled {
    let outcome = lifecycle::reindex_plugin(&id, deps)?;  // lock per plugin
    summary.aggregate(outcome);
}
```

**When to use:** Batch operations where each item is semantically independent, and a failure partway through should surface progress on earlier items (N independent operations).

**Pattern 2: Single-Lock-Per-Batch (Phase 9, `cascade_disable_for_catalog`)**

Acquire the lock once, mutate everything, release once:

```rust
pub fn cascade_disable_for_catalog(
    paths: &Paths,
    catalog: &str,
    plugins: &[String],
    embedder_seed: MetaSeed,
    reranker_seed: MetaSeed,
) -> Result<u32, TomeError> {
    let lock = acquire_lock(&paths.index_lock)?;
    let result = (|| -> Result<u32, TomeError> {
        let conn = index::open(...)?;
        for plugin in plugins {
            let dropped = delete_by_plugin(&conn, catalog, plugin)?;
            total = total.saturating_add(dropped);
        }
        Ok(total)
    })();
    
    match result {
        Ok(total) => { lock.release()?; Ok(total) }
        Err(e) => { drop(lock); Err(e) }
    }
}
```

**When to use:** Batch operations that are semantically a single user action ("destroy this catalog"), where the entire operation succeeds or fails atomically. All items participate in one transaction.

**Rationale:** The operation's semantic contract determines the locking pattern. `catalog remove --force` is "delete this catalog's plugins" (all-or-nothing atomicity); `catalog update` is "refresh N independent catalogs" (per-item independence).

### Batch Reindex Operations (Phase 7)

**Per-plugin atomicity:** Each `lifecycle::reindex_plugin` call acquires its own advisory lock. The spec doesn't require cross-plugin atomicity for `tome catalog update` or `tome reindex`. This is documented to set the precedent for future batch ops.

**Example from `src/commands/catalog/update.rs`:**
```rust
pub fn reindex_catalog_plugins(
    catalog_name: &str,
    enabled: &[PluginId],
    deps: &LifecycleDeps,
) -> Result<ReindexSummary, TomeError> {
    let mut summary = ReindexSummary::default();
    
    for id in enabled {
        // Each call acquires lock independently
        let outcome = lifecycle::reindex_plugin(&id, deps)?;
        summary.aggregate(outcome);
    }
    
    Ok(summary)
}
```

### Banner Skipped in JSON Mode

NDJSON consumers expect structured records on stdout and nothing on stderr unless an error occurs. Established in `commands::plugin::enable` and extended across all NDJSON producers:

```rust
if mode == Mode::Human {
    writeln!(out, "Enabling {}…", id)?;
}
```

Then conditionally emit human or JSON output. Applied to:
- `plugin::enable`
- `plugin::disable`
- `models::download`
- `models::list`
- `models::remove`
- `catalog::update` ("Reindexed plugins" block emits only NDJSON when `--json`)
- `reindex`

### Optional JSON Array Pattern (Phase 9)

When a JSON envelope may include an optional field that is sometimes empty (e.g., cascade array in `catalog remove --json`), use the `#[serde(skip_serializing_if = "Vec::is_empty")]` attribute:

```rust
#[derive(Serialize)]
pub struct RemovedRecord {
    pub name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cascade: Vec<CascadeRecord>,
}
```

This keeps the JSON compact when the array is empty (normal `catalog remove` with no enabled plugins) and includes it when non-empty (cascade case with `--force`). Applied to:
- `src/commands/catalog/remove.rs::RemovedRecord::cascade`

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

**Emit-only records (Phase 4 / US2):** Output-only structs (`WorkspaceInfo`, `InitOutcome`) omit the strictness attribute. These are never deserialized; they are emit-only. Their forward-compat is naturally permissive.

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

### Helper Promotion as Deliberate Process (Phase 8)

When a helper function is used by multiple independent modules (usually 4+ callers), it's promoted from `pub(crate)` to `pub` to mark it as an intentional public API surface. Recent promotions:

- `ModelState` (enum) — used by `status.rs`, `models.rs`, tests
- `cheap_state(...)` — used by `models::list`, `status.rs`, tests
- `read_manifest(...)` — used by `plugin::show`, `plugin::enable`, `plugin_enable.rs` tests
- `primary_file_path(...)` — used by multiple model-related functions
- `human_mb(...)` — formatting helper for model sizes
- `registry_seeds()` / `embedder_entry()` / `reranker_entry()` — used by `status.rs`, tests

**Policy:** Promotion is intentional — the type system tells us "this internal helper is now a public API surface". Document it via the function's doc comment and in this guide.

### Test Injection Points (Phase 3 / F7 and F8)

**Reachable from integration tests:** When a test injection point must be reachable from integration tests under `tests/` AND from `#[cfg(test)]` unit tests AND potentially from production code in tightly scoped diagnostic scenarios, gate it with `#[doc(hidden)] pub static` instead of `#[cfg(test)]`. The `#[doc(hidden)]` keeps it out of the published API docs; the `pub` makes it visible across the integration-test crate boundary (which doesn't inherit `cfg(test)`).

Document the test-only intent in a doc comment:

```rust
/// Test-only injection point. Phase 7's `tests/schema_migration_e2e.rs`
/// registers a synthetic migration table for a single scenario, then
/// clears the slot. Production [`apply_pending`] reads through
/// [`active_migrations`] which falls back to [`MIGRATIONS`].
///
/// Public surface intentionally — integration tests live outside the
/// crate and `#[cfg(test)]` items aren't visible there. Doc-hidden to
/// keep it out of the published API; the only legitimate caller is a
/// test.
#[doc(hidden)]
pub static MIGRATIONS_OVERRIDE: RefCell<Option<&'static [Migration]>> =
    const { RefCell::new(None) };
```

Example usage: `src/index/migrations.rs::MIGRATIONS_OVERRIDE` for schema-migration e2e tests.

**Small filesystem operations in-module:** Unit tests for small file-system operations (file creation, rotation, permissions, idempotent no-ops) live in `#[cfg(test)] mod tests` blocks inside the module under test. These operations (rename, `set_len` for sparse fixtures, metadata reads) are fast and deterministic, making them suitable for in-module unit tests.

Example: `src/mcp/log.rs::tests` module contains 4 unit tests for rotation policy (skip under cap, rename when oversized, overwrite existing prev, noop when absent).

### MCP Tool Definitions (Phase 3 / F2)

**rmcp macro visibility:** The `#[tool_router(vis = "pub")]` attribute on the tool router makes the macro-generated `tool_router()` function visible to integration tests (`tests/mcp_server.rs`) so test code can introspect the advertised tool list and descriptions. The visibility argument is parsed via `darling` and documented by `rmcp-macros`.

**Tool handler signatures:** Each MCP tool handler in `src/mcp/tools/{search_skills,get_skill}` exposes:
- `Input` / `Output` types derived from `Deserialize`, `Serialize`, and `JsonSchema` (for `rmcp`'s tool advertisement)
- A `pub async fn handle(state: Arc<McpState>, input: Input) -> Result<Output, McpError>` function

The `#[tool]` macro in `src/mcp/server.rs` delegates to these free functions, keeping per-tool logic modular. The `#[tool_handler]` attribute on the `ServerHandler` impl routes `list_tools` / `call_tool` through the generated router.

**Description as doc comment:** The `#[tool]` macro's `description` argument accepts only string literals, BUT the macro falls back to doc comments on the handler method (preferred for long descriptions). This allows descriptions ≤350 chars to be code comments and automatically picked up by rmcp-macros.

**Domain-specific error codes via JSON `data`:** MCP error envelopes carry the contract's structured codes (`unknown_catalog`, `plugin_without_catalog`, `unknown_skill`, etc.) inside `ErrorData.data` as `{"code": "...", ...}`. The JSON-RPC numeric code (`INTERNAL_ERROR` / `INVALID_PARAMS`) is generic; the application-level code is the precise identifier.

Example from `src/mcp/tools/search_skills.rs`:
```rust
return Err(McpError::invalid_params(
    "plugin requires catalog",
    Some(json!({ "code": "plugin_without_catalog" })),
));
```

### MCP Async Patterns (Phase 3 / F1–F2)

**Single-threaded tokio runtime:** The MCP server runs `tokio::runtime::Builder::new_current_thread()` to avoid a pool of background threads. All I/O-bound operations (index reads, model loading) are sync and block the event loop, so a thread pool would not help.

**`spawn_blocking` for sync I/O inside async handlers:** Every MCP tool handler that touches rusqlite or fastembed wraps the call in `tokio::task::spawn_blocking` to keep the event loop responsive while the expensive operation completes. This is an async boundary — a utility for structured concurrency without changing the architecture (still single-threaded, not CPU-parallel).

Example from `src/mcp/tools/search_skills.rs`:
```rust
let reranker_arc = state
    .reranker
    .get_or_try_init(|| async move {
        tokio::task::spawn_blocking(move || {
            FastembedReranker::load(reranker_entry, &reranker_dir)
        })
        .await
        .map_err(|e| TomeError::McpStartupFailed { ... })?
        .map(|r| Arc::new(r) as Arc<dyn Reranker>)
    })
    .await
    .map_err(tome_to_mcp)?
    .clone();
```

**MCP-local async boundary:** Async lives strictly inside `src/mcp/`; the rest of Tome stays synchronous. This is enforced by a structural test (see `tests/mcp_lifecycle.rs`).

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
