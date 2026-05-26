# Coding Conventions

> **Purpose**: Document code style, naming conventions, error handling, and common patterns used throughout the Tome codebase.
> **Generated**: 2026-05-27
> **Last Updated**: 2026-05-27
> **Phase**: 5 (commands-as-prompts + substitution layer; US2 shipped)

## Code Style

### Formatting & Linting

| Tool | Configuration | Command |
|------|---------------|---------|
| rustfmt | Cargo.toml `edition = "2024"` | `cargo fmt --check` |
| Clippy | Cargo.toml lint profile | `cargo clippy --all-targets --all-features -- -D warnings` |
| Typos | `.typos.toml` | `typos` |
| Git hooks | `.githooks/pre-commit` | `git config core.hooksPath .githooks` |

### Style Rules (Enforced)

| Rule | Convention | Example |
|------|------------|---------|
| Indentation | 4 spaces (Rust standard) | |
| Trailing commas | Always in multi-line | `vec![1, 2, 3,]` |
| Line length | No hard limit; readability-driven | |
| Comments | Explain *why*, not *what* | `// Merge both passes to close exfiltration vector per NFR-007` |
| Unsafe blocks | Always justified with SAFETY comment | `// SAFETY: we hold HOME_MUTEX for the lifetime of Self.` |

**Pre-commit hook** (`.githooks/pre-commit`) runs three gates in sequence:
1. `cargo fmt --check` — format violations fail the commit
2. `typos` — spelling errors fail the commit
3. `cargo clippy --all-targets --all-features -- -D warnings` — all warnings treated as errors

Bypass with `git commit --no-verify` only when unavoidable; document the reason in the commit body.

## Naming Conventions

### Files & Directories

| Type | Convention | Example | Notes |
|------|------------|---------|-------|
| Modules | snake_case | `src/substitution/builtins.rs` | Capability-organised |
| Test files | kebab-case with domain prefix | `tests/substitution_builtins.rs` | One test file per feature area |
| Fixtures | descriptive snake_case | `tests/fixtures/sample-plugin-catalog/` | |

### Code Elements

| Type | Convention | Example | Notes |
|------|------------|---------|-------|
| Variables | camelCase | `home_path`, `config_file` | |
| Constants | SCREAMING_SNAKE_CASE | `SHORT_MAX_CHARS = 800` | Single source of truth for shared constants (e.g. substitution length windows) |
| Functions | snake_case, verb-first | `lifecycle_paths()`, `resolve_builtin()` | Public APIs use `run_with_deps()` split for library reuse |
| Structs | PascalCase | `SubstitutionContext`, `HomeGuard` | RAII helpers suffix with `Guard` |
| Traits | PascalCase | `HarnessModule`, `Embedder` | Closed trait sets match Phase contract names |
| Enums | PascalCase | `TomeError`, `CompositionErrorKind` | Closed error enum enforces exit-code chain |
| Methods | snake_case | `.install()`, `.build()` | RAII guards implement `Drop` explicitly |

### Error Handling

| Type | Convention | Example |
|------|------------|---------|
| Custom errors | `#[derive(thiserror::Error)]` enum | `src/error.rs` — closed `TomeError` enum |
| Error variants | PascalCase, descriptive | `WorkspaceNotFound { name }`, `HarnessClash { path, command, first_arg }` |
| Logging | `tracing::error!`, `warn!`, `info!`, `debug!` | Structured fields for context |
| Result handling | Early return with `?` operator | `err_value?` propagates to caller |
| Anyhow wrapping | Applied only at application boundary | `anyhow::Context::context()` for chain context |

**Closed Error Enum Discipline**: The `TomeError` enum in `src/error.rs` has NO `Other`/`Unknown` arm. Every failure class has its own variant and exit code. Adding a variant forces edits to:
1. Exit-code table in the phase-specific contract
2. `tests/exit_codes.rs` — grep guard validates completeness
3. Spec FRs documenting the failure mode

This is structural enforcement: the compiler catches missing exit-code mappings.

## Common Patterns

### Silent Compute / Emit Wrapper

When a CLI command's compute path is reused by a non-CLI surface (MCP tools, library API, tests), split into:

```rust
// Silent compute path — no I/O side-effects, returns Outcome
pub fn pipeline(args: &Args, deps: &Deps) -> Result<Outcome, TomeError> { ... }

// Library entry point — pipeline + optional emit
pub fn run_with_deps(args: Args, deps: Deps) -> Result<Outcome, TomeError> {
    let outcome = pipeline(&args, &deps)?;
    // ... emit outcome if mode != Json
    Ok(outcome)
}

// CLI wrapper — resolves dependencies, calls run_with_deps
pub fn run(args: Args, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let deps = /* construct from scope */;
    run_with_deps(args, deps)?;
    Ok(())
}
```

Examples: `commands/query.rs`, `commands/status.rs`, `commands/reindex.rs`.

**Test boundary**: heavy-state paths (embedder load) use library API + `StubEmbedder`; light/error paths use CLI binary via `ToolEnv::cmd()`.

### RAII Test Isolation Helpers

Process-global mutable state (env vars, override slots, serialization mutexes) uses RAII guard structs:

```rust
pub struct HomeGuard {
    _previous: PrevHome,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl HomeGuard {
    pub fn install(new_home: &Path) -> Self {
        let lock = HOME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        // ... set HOME and return guard
    }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        // Restore previous value (SAFETY: we held the lock)
    }
}
```

Pattern established by Phase 4 / US3.c-1; extended in Phase 5 / US2:
- `HomeGuard` — serialises `$HOME` mutations via `HOME_MUTEX`
- `EnvVarGuard` — serialises arbitrary env-var mutations via per-file `ENV_MUTEX`
- `ClockOverrideGuard` — installs clock override into `SUBSTITUTION_CLOCK_OVERRIDE` slot
- `PluginDataDirGuard`, `WorkspaceDataDirGuard` — install data-dir overrides

Poisoned-mutex recovery via `PoisonError::into_inner()` — a panic in one test must not cascade into setup of the next.

### Single-Regex-Sweep with No-Rescan Invariant

When multiple stages of a transform must scan the same input (e.g. substitution's built-ins + env passthrough), compile a SINGLE regex that covers all stages and emit resolved values directly into the output buffer. This closes the exfiltration vector where a hostile plugin's `"version": "${TOME_ENV_GITHUB_TOKEN}"` could leak the host env var into the LLM context.

Pattern in `src/substitution/mod.rs::render()`:
1. Compile `combined_regex()` with all stages as alternations
2. Fast-path: if no match, return input unchanged (zero allocation)
3. Single `captures_iter()` pass: emit matched groups directly to output buffer
4. Never re-scan resolved values (NFR-007 / FR-051)

### Atomic Populated-Directory Landing

When a multi-file directory must appear either complete or not-at-all to readers:

```rust
let staging = tempfile::Builder::new()
    .prefix(".tome.tmp.")
    .tempdir_in(target_parent)?;
// Populate staging/
// ... chmod mode preservation on staged files ...
let staged_path = staging.keep()?.into_path();
std::fs::rename(staged_path, target)?; // POSIX-atomic when same filesystem
```

Replace-existing semantics: rename existing target aside to `.<name>.old/` first, rollback on final rename failure. Pattern established by Phase 4 / US1.b; refined in Phase 4 / US2.b for workspace initialization.

### Per-Entry Validation Before Collection

When a collection will be built under the advisory lock, validate each entry BEFORE acquiring the lock rather than deferring errors:

```rust
// Validate all entries first (cheap, no lock needed)
for entry in user_list {
    harness::lookup(&entry)?; // Fails fast with HarnessNotSupported
}
// Then acquire lock and build
let lock = index::lock::acquire()?;
```

Pattern: `HarnessNotSupported` returned immediately; `CompositionError` (structural issues) detected at resolution time, not at sync time.

## Module Organization

Tome's architecture is **capability-organised**, not layer-organised:

```
src/
├── catalog/          # Catalog manifest + git operations
├── commands/         # CLI command implementations (silent-compute + emit wrappers)
├── config/           # config.toml parsing (strict, #[serde(deny_unknown_fields)])
├── embedding/        # Embedder + reranker traits, model download, ONNX runtime
├── error.rs          # Closed TomeError enum (exit-code source of truth)
├── harness/          # Harness integration (claude-code, codex, cursor, etc.)
├── index/            # SQLite index, schema, migrations, vector search
├── mcp/              # MCP server (async island; sync boundary enforced by tests)
├── paths.rs          # XDG path resolution, phase-specific layouts
├── plugin/           # Plugin lifecycle (enable/disable/reindex, SKILL.md parsing)
├── presentation/     # Output formatting (tables, progress, colour, prompts)
├── settings/         # Workspace-scoped settings + composition resolver
├── substitution/     # Variable substitution pipeline (FR-022/051)
├── summarise/        # Summariser abstraction + LlamaModel inference
├── util/             # Atomic writes, error utilities
└── workspace/        # Workspace lifecycle, scope resolution
```

Each module is **root-owned** — no circular dependencies. Phase 3's `tests/sync_boundary.rs` enforces that only `src/mcp/` uses `tokio` / `async`.

## Strictness Boundary

**Tome-owned inputs** (config.toml, manifest.json, index schema) use `#[serde(deny_unknown_fields)]` — forward-incompatible changes are caught immediately. Tests in `tests/manifest_strictness.rs` grep for the marker across all relevant types.

**Third-party inputs** (plugin.json, SKILL.md frontmatter) parse **leniently** — unknown fields are ignored, allowing upstream extensions without breaking Tome's plugin consumption. This is explicitly documented in the phase-1 spec (FR-013a) and reinforced in all later phases.

## Git Conventions

### Commit Messages

Format: `type(scope): subject`

| Type | Usage | Examples |
|------|-------|----------|
| `feat` | New feature | `feat(plugin): add enable command` |
| `fix` | Bug fix | `fix(index): correct schema migration order` |
| `test` | Test additions | `test(substitution): add built-ins stage coverage` |
| `refactor` | Code restructure | `refactor(harness): consolidate sync paths` |
| `docs` | Documentation | `docs(CLAUDE.md): update phase-5 learnings` |
| `chore` | Maintenance | `chore(deps): pin llama-cpp-2 at 0.1.146` |

Enforced by `cocogitto` via `.githooks/commit-msg` hook.

### Branching

**Trunk-based development**: short-lived branches off `main`. PR strategy:
- **Small batches**: ~400 lines or 2 modules max as soft cap
- **Parallel slices**: each user story pre-planned into concrete 2–4 line slices
- **Four-reviewer pass**: contract audit, Rust-lens code review, test audit, security audit (after feature complete)

## Comments & Documentation

| Type | When to Use | Format |
|------|------------|--------|
| Module docstring | Every file in `src/` | `//! Public API and invariants for this module` |
| Item docstring | Public structs, traits, enums, functions | `/// What this does and why` |
| SAFETY comment | Every unsafe block | `// SAFETY: we hold HOME_MUTEX for the lifetime of Self.` |
| TODO | Planned work (document why it's deferred) | `// TODO (Phase 6): add concurrent reindex via rayon` |
| Inline comment | Complex logic, non-obvious invariants | `// Leftmost alternation guarantees env branch wins on TOME_ENV_* refs` |

**Phase annotations**: When a pattern, guard, or variant was introduced, cite the phase and user story:
```rust
// Phase 5 / US2 — environment variable substitution stage.
// Env vars are never re-scanned (no-rescan invariant, NFR-007).
```

This aids navigation of the phase-locked spec documents.

---

## What Does NOT Belong Here

- Test strategies → `TESTING.md`
- Security practices → `SECURITY.md`
- Architecture patterns → `ARCHITECTURE.md`
- Technology choices → `STACK.md`

---

*This document defines HOW to write code. Update when conventions change or new patterns are established.*
