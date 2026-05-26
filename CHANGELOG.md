# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.0] — 2026-05-26

### Phase 4 additions

User-visible

- `tome workspace <subcommand>` — named workspaces with central storage.
  `init <name>` creates a workspace in `<home>/.tome/workspaces/<name>/`
  with `settings.toml` + `RULES.md`. `--inherit-global` seeds the new
  workspace's enrolled catalogs from `global`'s `workspace_catalogs`
  rows (enablement not copied). `list` reports every workspace with
  catalog / plugin / skill / bound-project counts. `info [<name>]`
  carries the per-workspace diagnostic. `rename <old> <new>` rewrites
  every bound-project marker + the central DB row in one transaction;
  the workspace directory is renamed atomically. `regen-summary
  <name>` runs the bundled local summariser and writes the result to
  `[summaries]` in the workspace's `settings.toml` + propagates the
  long summary to `RULES.md` + every bound project's marker.
  `remove <name> [--force]` cascades through bound-project teardown +
  central DB rows + workspace dir + refcount-clean catalog caches in
  one advisory-lock window. `sync [<name>]` re-runs the harness
  integration sweep against every bound project.
- `tome workspace use <name> [--force] [--json]` — bind the current
  project to a workspace. Writes `.tome/config.toml` (marker only —
  pointer, not state) under the central advisory lock. Phase A
  commits the binding; Phase B runs harness sync without the lock so
  a slow harness FS doesn't block other Tome writes.
- `tome harness <subcommand>` — declare harnesses to integrate with.
  Bare `tome harness` lists the five shipped modules in lex order.
  `list [<workspace>]` reports the effective harness list per scope
  with composition source-chain. `use <name> --scope project|workspace|global [--force]`
  appends `<name>` to the chosen scope's settings file via
  `toml_edit::DocumentMut` (preserves comments + order) and runs sync
  if the effective list changes. `remove`, `info`, `sync` mirror the
  shape. `--force` on `use` overrides developer-owned MCP entries.
- Bundled local summariser — `qwen2.5-0.5b-instruct` (~400 MB GGUF,
  SHA-256 verified at use time) via `llama-cpp-2`. Sync inference;
  the backend singleton is process-global. Triggered automatically by
  every state-mutating skill operation (plugin enable/disable,
  reindex with content-hash changes, catalog update, catalog remove
  --force). FR-385 forward-progress: skill mutation commits BEFORE
  the summariser is invoked; failure exits 24 with the mutation
  retained.
- Layered settings + composition resolver — workspaces declare
  `harnesses = ["claude-code", "[workspaces.foo]", "!opencode"]`
  composing across scopes (project marker → workspace → global) with
  cycle detection (renders the walk-order chain) + bracketed
  references + `!`-prefixed exclusions. Composition errors exit 17;
  unknown harness names exit 18.
- `tome doctor` extended end-to-end with Phase 4 subsystems —
  `Subsystem` enum promoted to 11 typed variants (Embedder, Reranker,
  Index, Drift, Catalog(name), Schema, Summariser, Binding,
  BindingRulesCopy, HarnessRules(name), HarnessMcp(name)) with custom
  Serialize / Deserialize preserving the wire shape byte-for-byte.
  `--fix` repair classes for every Phase 4 subsystem. `--force`
  override for user-owned MCP entries. Orphan `.tome.tmp.*` staging
  dirs older than 1 hour are swept under five-layer defence-in-depth.
- `tome doctor --fix --force` requires `--fix` — `--force` alone exits
  2 (Usage).

Wire-shape changes

- `tome workspace init --json` envelope:
  `{name, path, catalogs_inherited, id}` (was `workspace_dir` /
  `inherited_catalogs`; no `id`).
- Doctor `harnesses[].name` hyphenated: `"claude-code"` (was
  `"claude_code"`); matches every other doctor harness field.
- Exit code 24 for `SummariserFailure` (originally specced as 20;
  reconciled to 24 in Phase 4 to avoid collision with Phase 2's
  `PluginNotFound`).

Configuration changes

- `~/.tome/` is the new root. The constitution v1.3.0 §Paths amendment
  dropped the `directories` crate; every Tome-owned path now lives
  under one absolute, canonicalised root. The Phase 3 XDG split
  (`config_dir` / `state_dir`) is gone.
- Single central `index.db` + `index.lock` per host (was one per
  workspace).
- `workspace_projects` table — 1:1 binding from project root path to
  workspace.

Security hardening

- `home_root()` validates `$HOME` is set, absolute, canonicalised.
  Relative or unset `$HOME` exits 2 (`Usage`), not 7 (`Io`).
- All Tome-owned config / settings file reads now go through
  `util::bounded_read_to_string` with per-class caps (1 MiB for
  Tome-owned, 256 KiB for plugin manifests, 1 MiB for harness MCP
  configs, 4 MiB for harness rules files). Over-cap reads return
  `Io(InvalidInput)`.
- `util::atomic_dir::land_directory` refuses to land through symlinks
  (plus `.old` aside cleanup).
- `doctor::orphan_cleanup::sweep_one` refuses to follow planted
  symlinks.
- All Tome-owned writes emit mode 0o600 on Unix (audit test pins).

New dependencies

- `llama-cpp-2 = "=0.1.146"` — exact-pinned for the bundled summariser.
- `encoding_rs = "0.8"` — required by `llama-cpp-2`'s `token_to_piece`.
- `toml_edit = "0.22"` — comment- and order-preserving TOML edits for
  settings + harness MCP configs.
- `filetime = "0.2"` (dev-dep) — mtime backdate for orphan-cleanup tests.

Dropped dependencies

- `directories` — replaced by `<home>/.tome/`-rooted `Paths`.

Test surface

- 916 → 954 tests across 125 → 127 suites (16 ignored).
- Polish phase added 38 tests across 5 PRs (PR-A through PR-E).

Binary size

- 26.31 MiB on macOS arm64. Well under the 50 MiB cap (constitution
  v1.2.0). Recorded in `RELEASE-BINARY-SIZE.md`.

## [0.3.0] — 2026-05-14

### Phase 3 additions

User-visible

- `tome mcp` — Model Context Protocol stdio server. Advertises two
  tools (`search_skills`, `get_skill`) so an agentic-coding harness can
  query the local skill index over the MCP protocol. Single-threaded
  tokio runtime; sync work via `spawn_blocking`. Stdout is reserved for
  protocol traffic; diagnostics land in `${XDG_STATE_HOME}/tome/mcp.log`
  (JSON-lines, 10 MiB rotation cap). Graceful shutdown on SIGINT,
  SIGTERM, or stdin close with a 5 s timeout for in-flight calls.
- `tome workspace info | init` — per-project workspaces. `init`
  atomically lands `.tome/` (sibling staging directory + rename;
  SIGINT-safe). `init --inherit-global` seeds the new workspace's
  catalogs from the global config (enablement not copied — lives in
  the index DB). `init --force` renames an existing `.tome/` aside.
  `info` is a read-only diagnostic.
- `tome doctor [--fix] [--verify] [--json]` — broad health check.
  Reports models, index integrity, catalog-cache state, workspace
  registry, drift, and locally-installed harnesses. `--fix` runs the
  three safe automatic repairs (model re-download, catalog re-clone,
  schema forward-migration). Exit 0 on healthy, 1 on degraded /
  unhealthy, 75 when `--fix` ran but un-fixable issues remain.
- Global `--workspace <PATH>` / `--global` flags on every command.
  Resolution priority: flag → `TOME_WORKSPACE` env → CWD walk →
  global fallback.
- Workspace registry — opt-in. Touch
  `${XDG_STATE_HOME}/tome/workspaces.txt` once to start tracking;
  `init` appends each new workspace. Used by the catalog refcount
  algorithm to keep a shared on-disk clone alive while any scope
  still references it.

Architecture / framework

- Per-scope `Paths::*_for(&Scope)` accessors. Every Phase 1 / Phase 2
  command now honours the resolved scope end-to-end.
- Content-addressed catalog clone refcount. Two scopes adding the same
  URL share one on-disk clone; removal only deletes when the last
  referencing scope drops the entry.
- Forward-only schema migration framework. Ships with zero registered
  migrations; per-step transactional atomicity; refuses newer-on-disk
  schemas with `SchemaVersionTooNew` (73). The first real migration
  lands in Phase 4+; e2e rails are tested via `MIGRATIONS_OVERRIDE`
  thread_local injection against synthetic fixtures.

New exit codes

- 60 `McpStartupFailed` — residual MCP startup failure.
- 61 `McpProtocolIo` — MCP transport-layer failure.
- 70 `WorkspaceMalformed` — workspace exists but config or index is
  unparsable.
- 71 `WorkspaceNotFound` — `--workspace <path>` or `TOME_WORKSPACE`
  names a path with no `.tome/` marker.
- 72 `WorkspaceConflict` — both `--workspace` and `--global` set.
- 73 `SchemaVersionTooNew` — on-disk schema is newer than this Tome
  supports.
- 74 `SchemaMigrationFailed` — a registered migration's apply step
  returned an error.
- 75 `DoctorFixNotSafe` — `tome doctor --fix` ran but un-fixable
  issues remain.

New dependencies

- `rmcp` (Model Context Protocol SDK). Scoped to `src/mcp/`.
- `tokio` (single-threaded runtime, signal handling). Scoped to
  `src/mcp/`. The sync-boundary discipline is structurally enforced
  by `tests/sync_boundary.rs`.
- `schemars` (JSON schemas for the MCP tool input/output types).

Security hardening

- `mcp.log` created with mode 0600 on Unix (workspace paths + scrubbed
  error chains; default umask would leave it world-readable on a
  shared machine).
- `get_skill` rejects symlinks in the resources list (defence against
  a hostile catalog author committing
  `skills/foo/credentials -> ~/.ssh/id_rsa`).
- Workspace registry validation: 1 MiB size cap, 10k entry cap, reject
  NUL bytes and `..` components.
- Workspace init refuses to overwrite a non-directory `.tome` marker.

### Removed / breaking

- None. Phase 1 / Phase 2 surfaces are unchanged.

## [Unreleased]

_Future work tracked in `specs/`._

## [0.2.0] (pre-Phase-3 baseline)

### Phase 2 additions

User-visible
- `tome plugin enable <catalog>/<plugin> [--json]` — parse the plugin's
  `plugin.json` + every `SKILL.md`, embed each skill description with
  `bge-small-en-v1.5`, persist into a local SQLite index. Atomic per
  plugin: SIGINT or embedder failure rolls back. Cheap re-enable when
  content hashes match (the embedder is not invoked).
- `tome plugin disable <catalog>/<plugin> [--force] [--json]` — flip
  the row's `enabled` flag without dropping vectors; re-enable stays
  fast. `--force` skips the confirm prompt; non-TTY without `--force`
  exits 54.
- `tome plugin list [--catalog] [--enabled-only] [--json]` —
  table/NDJSON of every registered plugin with status and skill count.
- `tome plugin show <catalog>/<plugin> [--json]` — rich per-plugin
  view with component breakdown.
- `tome plugin` (no subcommand) — interactive catalog → plugin →
  action flow. Non-TTY exits 54.
- `tome models download [--force] [--json]` — fetch the pinned BGE
  embedder + reranker into `${XDG_DATA_HOME}/tome/models/`. Atomic
  rename; SHA-256-verified against the registry pin.
- `tome models list [--verify] [--json]` — install state per model.
  `--verify` rehashes on disk.
- `tome models remove <name> [--force] [--json]` — manifest-first
  deletion. Non-TTY without `--force` exits 54.
- `tome query <text> [--top-k] [--catalog] [--plugin] [--no-rerank]
  [--strict] [--min-score] [--json]` — semantic search across enabled
  skills. KNN over `sqlite-vec` candidates, optionally re-ranked by
  `bge-reranker-base`. `--strict` returns exit 40 on empty results.
- `tome reindex [<scope>] [--force] [--json]` — rebuild the index for
  all enabled content, one catalog, or one plugin. Cheap-skip when
  content hashes are unchanged; `--force` re-embeds every skill.
- `tome status [--verify] [--json]` — read-only doctor / pre-flight.
  Reports embedder + reranker state, index integrity, drift, and an
  overall ok/degraded/unhealthy verdict. Non-zero exit on non-ok.
- `tome catalog update` extended to reindex every enabled plugin in
  each refreshed catalog (cheap-skip unchanged, re-embed modified,
  drop removed); plugins gone upstream auto-disable.
- `tome catalog remove --force` cascades disable + row drop for every
  enabled plugin in the catalog inside one advisory-lock window.
  Without `--force` and with enabled plugins present, exits 53.
- `tome --version` extended to three lines: tool, embedder, reranker
  (each name + version). `--json --version` emits the structured form.
- Phase 2 exit codes (closed-and-exhaustive): 20 plugin not found,
  21 already in state, 22 plugin manifest parse error, 23 skill
  frontmatter parse error, 30 model missing, 31 model corrupt, 32
  checksum mismatch, 33 model manifest parse error, 34 inference
  runtime init, 35 vector extension init, 36 embedding failure, 37
  reranker failure, 40 strict-query empty, 41/42 embedder drift, 50
  index busy, 51 integrity check, 52 schema too new, 53 catalog has
  enabled plugins, 54 not a terminal.

Project-level
- `rusqlite` (bundled SQLite, no system dep) + vendored `sqlite-vec`
  C extension (v0.1.9, MIT) compiled in via `build.rs`. The whole
  index — including 384-dim vectors — lives in one SQLite file.
- `fastembed-rs` wrapping `ort` (ONNX Runtime, CPU execution provider
  only). CUDA / CoreML / DirectML disabled. Models downloaded at
  runtime; not bundled.
- Advisory write lock at `${XDG_DATA_HOME}/tome/index.lock` via
  `std::fs::File::try_lock` (OFD-flock on macOS/BSD,
  `F_OFD_SETLK` on Linux). Held during every write; readers
  deliberately do not block.
- Tighter `config.toml` permission (0600 on Unix). Catalog URL is
  scrubbed before persistence (the URL-credential scrub regex now
  covers any RFC-3986 scheme, including `file://` and `ssh://`).
- Binary-size CI gate revised 10 MB → 50 MB (CONSTITUTION v1.2.0;
  `ort` static is the load-bearing dep, profile is `lto = "thin"`,
  `panic = "abort"`, `strip = "symbols"`).
- 257 tests across 39 integration suites.

### Changed

- **Hooks** — replaced `lefthook` with three versioned scripts under
  `.githooks/` wired through git's `core.hooksPath` config. The set of
  gates (fmt, typos, clippy, cog verify, cargo test) is unchanged; the
  delivery mechanism is now one less moving part. Bootstrap is `git
  config core.hooksPath .githooks` (one-time, per clone). Constitution
  bumped to v1.1.0 to reflect the workflow change. See
  `specs/002-phase-2-plugins-index/retro/P2.md` for the diagnosis that
  drove this migration.

### Phase 1 additions

User-visible
- `tome catalog add <source> [--name] [--ref] [--json]` — register a remote
  catalog. `<source>` accepts `owner/repo`, full Git URLs, or local paths
  (auto-converted to `file://`). SHA-shaped `--ref` values are pinned.
- `tome catalog list [--json]` — alphabetical table (human) or NDJSON
  records (JSON).
- `tome catalog show <name> [--json]` — manifest + registration metadata.
- `tome catalog update [<name>] [--json]` — refresh one or every catalog;
  SHA-pinned catalogs are a documented no-op.
- `tome catalog remove <name> [--force] [--json]` — confirmation prompt
  on TTY; `--force` required when stdin is not a TTY.
- Global `--json` and `-v`/`-vv` flags on every command; `--help` and
  `--version` provided automatically by clap.
- Closed-and-exhaustive exit codes: 0 success, 1 internal, 2 usage, 3
  catalog not found, 4 catalog already exists, 5 manifest invalid, 6 git
  failed, 7 I/O, 8 interrupted.

Project-level
- Initial project scaffold: Cargo crate, dual MIT/Apache licence,
  versioned git hooks under `.githooks/` (`fmt`, `clippy -D warnings`,
  `typos`, `cog verify`, `cargo test`) wired via `core.hooksPath` with no
  external manager, GitHub Actions CI matrix
  (`{ubuntu,macos} × {stable,MSRV}`), security workflow (`cargo audit`,
  `cargo deny`), 10 MB stripped-binary CI gate, `deny.toml` with the
  constitution's licence allowlist, `renovate.json`.
- Strict TOML parsing (`#[serde(deny_unknown_fields)]`) on every
  manifest and config struct. A structural-grep test rejects regressions.
- Credential scrubbing at the process-output boundary: every byte stream
  captured from a spawned `git` process passes through
  `catalog::git::scrub_credentials` before it reaches `tracing`,
  `anyhow::Error`, or any display path.
- Atomic registry persistence via `tempfile::NamedTempFile::persist`.
- Signal-aware `git` shell-outs: SIGINT during `clone` / `fetch` /
  `reset` kills the child and returns exit code 8.
- XDG-aware path resolution (`XDG_CONFIG_HOME`, `XDG_DATA_HOME`)
  honoured on macOS and Linux.
- Phase 1 specification under `specs/001-phase-1-foundations/`.
- Project constitution (`CONSTITUTION.md` v1.0.1).
