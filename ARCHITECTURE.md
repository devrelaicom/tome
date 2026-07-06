# Architecture

Tome is a Rust CLI and MCP server that makes Claude Code's plugin ecosystem work
across other agentic coding harnesses (Cursor, Codex, Gemini CLI, OpenCode, and
around a dozen more). You register **catalogs** (git repos of plugins), enable
the **plugins** you want, and Tome builds a local semantic index (SQLite +
vector search, entirely on your machine) of their skills, commands, and agents.
From there it writes each harness's native config and serves the index over a
stdio MCP server, so an agent can search for and load exactly the skill it
needs mid-task. See [`README.md`](./README.md) for the user-facing tour.

This document orients a contributor: the module map and the small set of
load-bearing rules the codebase is built around.

> **Why this file exists:** docs.rs cannot build this crate — the `ort`
> dependency downloads the ONNX Runtime library at build time, and docs.rs
> builds in a network-isolated sandbox (see the `documentation` note in
> `Cargo.toml`). This file plus the module-level `//!` docs are the
> orientation surface instead. A local `cargo doc` works, but the first build
> needs network access for the same reason.

## Module map

Modules under `src/` are capability-shaped, not layer-shaped. Each line below
is condensed from the module's own `//!` header — read those headers for the
full story.

| Module | What it is |
|---|---|
| `authoring/` | Shared core behind `tome {catalog,plugin,skill} {create,convert,lint}`: a normalised artifact IR, one emitter, the lint rule registry, and the harness-ism rewriter. |
| `catalog/` | Catalog manifest schema + parsing, git shell-outs with credential scrubbing at the process-output boundary, and the atomic registry store. |
| `cli.rs` | `clap` derive definitions for the whole command tree. |
| `commands/` | One module per command surface; thin emit/exit wrappers over library compute paths. |
| `config.rs` | The unified global config document (`~/.tome/config.toml`) as one typed, strict struct. |
| `doctor/` | The `tome doctor` diagnostic: `assemble_report` is the silent-compute path, the CLI wrapper adds emit + exit semantics. |
| `embedding/` | Embedder + reranker traits over the ONNX-backed `fastembed` crate, the pinned model registry, profiles, and downloads. A deterministic stub serves the fast test suite. |
| `error.rs` | The closed `TomeError` enum — the single source of truth for exit codes. |
| `harness/` | The `HarnessModule` trait + static registry (one file per supported harness) and the sync orchestrator that writes each harness's native config. |
| `index/` | The central SQLite + `sqlite-vec` database: schema, forward-only migrations, advisory lock, and the query layer. |
| `logging.rs` | `tracing-subscriber` wiring; diagnostics go to stderr only, orthogonal to `--json`. |
| `mcp/` | `tome mcp`, the stdio MCP server — the one async island (`tokio`, `rmcp`). |
| `model_registry.rs` | Harness-target model-ID registry: a trimmed `models.dev` snapshot vendored in git and embedded at build time. |
| `output.rs` | Output mode (human vs `--json`) and a thin formatter abstraction. |
| `paths.rs` | Path resolution for the single-root layout: every Tome-owned path lives under `~/.tome/`. |
| `plugin/` | Plugin identity, lenient third-party manifest/frontmatter parsers, component enumeration, and the enable/disable lifecycle. |
| `presentation/` | Thin wrappers around `comfy-table`, `indicatif`, `owo-colors`, and `inquire` so commands don't carry crate knowledge. |
| `provider/` | BYOK/BYOM external model providers (summarisation, embedding, reranking) — hand-rolled sync HTTP over `reqwest::blocking`. |
| `settings/` | Layered settings (project / workspace / global) and the composition resolver that produces the effective harness list. |
| `substitution/` | The variable-substitution pipeline for entry bodies: built-ins → env passthrough → arguments, in a single regex sweep. |
| `summarise/` | The workspace summariser: Qwen2.5-0.5B-Instruct (GGUF) served via `llama-cpp-2`, sync. |
| `telemetry/` | Local-first, fire-and-forget telemetry. Zero foreground network: commands append one line to a local queue; delivery is a detached background flush. |
| `util/` | Cross-cutting helpers, chiefly `atomic_dir` (atomic populated-directory landing). |
| `workspace/` | Workspace context: scope resolution, project binding markers, and the junction-table scoping over the central index. |

Supporting directories: `vendor/sqlite-vec/` (pinned C source, compiled by
`build.rs`), `assets/` (embedded meta skills, harness plugin shims, the model
registry snapshot), and `tests/` (integration suites grouped into
per-capability binaries, shared harness in `tests/common/`).

## Load-bearing rules

Each rule below is enforced somewhere concrete. If a change fights one of
them, the design needs revisiting before the code does.

**Sync only, except `src/mcp/`.** The MCP server is the one place `async fn`,
`.await`, and `tokio::` are allowed; every other module is synchronous
(`reqwest::blocking`, `rusqlite`, `fastembed`, `llama-cpp-2` are all sync).
Enforced structurally by `tests/harness_settings/sync_boundary.rs`, which
scans `src/` for async constructs outside the island.

**A closed error set.** `TomeError` in `src/error.rs` has no catch-all
variant: every failure class gets its own variant with its own exit code, and
adding one forces edits to the exit-code tests and docs — the compiler
enforces the chain. Exit codes are a stability contract (constitution
Principle II).

**Atomic writes and the advisory lock.** State-changing writes stage into a
sibling temp location and land with a single rename — `util::atomic_dir`
(`land_directory`) for directories, `tempfile`-staged writes elsewhere.
Mutating index commands (`plugin enable`/`disable`, `reindex`, migrations)
acquire the advisory lockfile in `src/index/lock.rs` before opening their
SQLite transaction; read-only commands (`query`, `status`) never touch it.

**The strictness boundary.** Tome-owned inputs (the global config, model
manifests, index meta) deserialise with `#[serde(deny_unknown_fields)]` — a
typo fails loudly. Third-party inputs (plugin manifests, `SKILL.md`
frontmatter) parse leniently, because Tome does not control their authors.
The boundary is pinned by the `manifest_strictness` integration test
(`tests/index_query_misc/manifest_strictness.rs`).

**Credential scrubbing at the boundary.** Output from spawned `git`
processes, and download/HTTP error chains, pass through
`catalog::git::scrub_credentials` before reaching `tracing`, error chains, or
any display path. New code that surfaces process or network output routes
through the same chokepoint.

**The 50 MB release-binary cap.** CI asserts the stripped release binary
stays under 50 MB (the `size` job in `.github/workflows/ci.yml`, run on
pushes to `main`). Dependency additions are the usual way to break it, and
they are constitutionally gated anyway.

## Where to go next

- [`CONTRIBUTING.md`](./CONTRIBUTING.md) — local setup, hooks, commit and PR
  conventions.
- [`CONSTITUTION.md`](./CONSTITUTION.md) — the principles behind the rules
  above, and the amendment process for changing them.
- [`site/docs/`](./site/docs/) — the user-facing documentation, published at
  [tome-mcp.netlify.app](https://tome-mcp.netlify.app).
