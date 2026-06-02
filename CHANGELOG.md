# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.6.0] — 2026-05-29

### Phase 6 additions

User-visible

- **Real Claude Code hooks.** A plugin's `hooks/hooks.json` is rewritten
  (`${CLAUDE_PLUGIN_ROOT}` / `${CLAUDE_PLUGIN_DATA}` → absolute paths;
  `${CLAUDE_PROJECT_DIR}` / `${CLAUDE_SESSION_ID}` left verbatim) and merged
  into `.claude/settings.local.json` by deep structural equality. Removal
  re-derives + structural-matches, so a hook you hand-edit is never deleted;
  the committed `settings.json` is never touched.

- **`GUARDRAILS.md` prose fallback.** A plugin's `hooks/GUARDRAILS.md` renders
  as a per-plugin marker region in each harness's rules file (Claude Code
  suppresses it when the plugin also ships hooks), or as a fully-Tome-owned
  Cursor sibling (`.cursor/rules/TOME_GUARDRAILS.md`, deleted when empty). A
  verbatim body containing a managed-marker line is refused (exit 46).

- **Native agent translation across four harnesses.** A plugin's
  `agents/<name>.md` is translated to each harness's native agent format
  (claude-code / codex / cursor / opencode); Gemini CLI has no native-agent
  support and is skipped. Agents are indexed (`kind='agent'`) but never
  embedded or returned by `search_skills`.

- **Optional agent-as-MCP-prompt personas (off by default).** With
  `expose_agents_as_personas = true` (resolved at the MCP startup scope), each
  enabled agent is also exposed as an MCP prompt persona, plus a reserved
  `drop-persona`. Double opt-in; an advisory caveat rides the prompt surface.

- **Phase 4 rules-file correction.** Claude Code's rules sink is now
  `CLAUDE.md` (with `.claude/CLAUDE.md` fallback), not `AGENTS.md`.

- **`tome harness sync`** now reconciles all three new sinks (hooks →
  guardrails → agents, fixed order) with per-plugin forward progress and a
  deterministic first-error precedence. **`tome doctor`** gains five read-only
  Phase 6 reports (hooks / guardrails / agents / privilege-escalation /
  personas); `--fix` repairs only the safe derivable cases. **`tome plugin
  show`** lists agents + `hooks.json` / `GUARDRAILS.md` presence + the resolved
  persona name.

- **Plugin-agent privilege governance.** Privileged agent fields
  (`hooks` / `mcpServers` / `permissionMode`) pass through to Claude Code by
  default, are auditable via doctor's privilege-escalation report (which always
  reads the unstripped source), and are strippable via the opt-in layered
  `strip_plugin_agent_privileges` setting.

### Internal additions

- **Four new exit codes (43–46)** — `HookSpecParseError` (43),
  `HookSettingsWriteFailed` (44), `AgentTranslationFailed` (45),
  `GuardrailsWriteFailed` (46). Closed-enum discipline preserved (no
  `Other`/`Unknown` arm).

- **`EntryKind` gains an `Agent` variant**; schema migrates v3 → v4 via a
  marker-only no-op (the free-text `kind` column admits `agent` with no DDL).

- **No new top-level dependency and no new top-level module** — `hooks.rs`,
  `guardrails.rs`, `agents.rs` live inside `src/harness/`; the persona registry
  reuses the Phase 5 prompt machinery. Leanest phase since Phase 1.

- **Test suite** grew from 151 to 175 integration suites (+24); ≈1427 test
  functions. Every new emit-only type carries a byte-stable JSON wire-shape pin
  (NFR-011); a phase-wide 4-reviewer pass over the assembled surface returned 0
  blockers and a clean security result.

## [0.5.0] — 2026-05-27

### Phase 5 additions

User-visible

- **Commands as first-class entries alongside skills.** Plugins can now ship
  `commands/<name>.md` files in addition to `skills/<name>/SKILL.md`. The
  unified `skills` table gains a `kind` discriminator (`skill` | `command`);
  schema migrates v2 → v3 with structurally-equivalent backfill defaults
  (skills default to searchable=true / user_invocable=false; commands
  default to searchable=true / user_invocable=true).

- **User-invocable entries surface as MCP prompts.** A new `prompts/list` +
  `prompts/get` capability on the MCP server advertises each user-invocable
  entry as a slash command. Sanitised + collision-resolved prompt names
  (counter-suffixing per the contract).

- **Variable substitution layer** — Tome built-ins (`${TOME_SKILL_DIR}`,
  `${TOME_PLUGIN_DATA}`, `${TOME_WORKSPACE_DATA}`, etc.; 12 total),
  environment passthrough via `${TOME_ENV_FOO}` with default-value syntax,
  Claude Code-compatible argument substitution (`$ARGUMENTS`,
  `$ARGUMENTS[N]`, `$N`, `$name`), and `ARGUMENTS:` append-fallback when
  caller-supplied arguments aren't referenced in the body. Single-sweep
  regex enforces the NFR-007 no-rescan invariant structurally.

- **Middle-tier MCP discovery tool `get_skill_info`** — between
  `search_skills` (top-k results, descriptions truncated) and `get_skill`
  (full body). Returns the full description, `when_to_use`, plugin
  version, user_invocable flag, absolute path, and a capped resource
  enumeration of the entry's directory tree.

- **`tome plugin show` extended** with Skills + Commands grouping,
  per-entry `searchable=` / `user_invocable=` / `[dormant]` annotations
  + derived prompt name. JSON output mirrors the grouped shape.

- **`tome plugin list` extended** with per-kind count format
  `<n> skills, <m> commands`.

- **`tome doctor` extended** with three Phase 5 read-only surfaces:
  `prompts` (registered + collisions), `orphan_data_dirs`,
  `entry_counts` (per-kind + `pending_re_embedding`). Each field emits
  `None` only when `ScopeSource::GlobalFallback`. FR-124 read-only
  invariant structurally enforced.

- **`when_to_use` frontmatter field** is now embedded for semantic
  search alongside `description` + body.

- **Pre-push hook slim-down** — `.githooks/pre-push` now runs the
  pre-commit chain (fmt / typos / clippy) only, not
  `cargo test --workspace`. CI's 4-way matrix runs the full suite +
  full-features build on every PR.

### Internal additions

- **Schema migration v2 → v3** registered via the Phase 3 framework.
  New `kind`, `searchable`, `user_invocable`, `when_to_use` columns;
  unique constraint widened to `(catalog, plugin, kind, name)` via the
  SQLite 12-step table-rebuild pattern.

- **`src/substitution/` module** hosts the hand-rolled substitution
  engine. Single-sweep `combined_regex()` enforces the no-rescan
  invariant; 4-stage pipeline (built-ins / env / arguments / append-
  fallback).

- **New `src/mcp/{prompts.rs, prompt_name.rs, prompt_collision.rs,
  tools/get_skill_info.rs, substitution_helpers.rs}`** — prompt
  registry, derivation algorithm, collision-resolution (counter-suffix
  on lex order), middle-tier discovery tool, shared substitution-
  context builder.

- **`Paths::plugin_data_root()`** + `plugin_data_dir_for(catalog, plugin)`
  + `workspace_data_dir_for(workspace, catalog, plugin)` — single source
  of truth for the `<root>/plugin-data/` + per-workspace
  `<root>/workspaces/<ws>/plugin-data/` layouts.

- **`MCP_SLASH_PREFIX`** constant in `src/mcp/mod.rs` — canonical
  `/mcp__tome__` prefix consumed by `tome doctor` rendering.

- **Frontmatter parser widened** to the Phase 5 lenient field set.
  `MAX_ARGUMENTS = 256` cap at the parser boundary.
  `MAX_DESCRIPTION_MAX_CHARS = 100_000` soft cap in search_skills +
  warning surface in plugin show.

### Bug fixes

- **Path traversal in `resolve_entry_body_path`** (US1.d BLOCKER S-H1).
  Refuses `..` components + absolute paths in DB-stored relative paths.

- **Data exfiltration vector via substitution** (US2.d BLOCKER B2). A
  hostile plugin author could set `"version":
  "${TOME_ENV_GITHUB_TOKEN}"` in `plugin.json` and leak operator env via
  the `${TOME_PLUGIN_VERSION}` built-in. Fix: single-sweep regex union
  pattern enforces structurally that resolved values never re-enter the
  scanner.

- **DoS amplifier in `truncate_description`** (US4.d HIGH C-2). Bounded
  `char_indices` walk replaces the prior O(n) shape. Polish PR-B
  propagated the same fix to `prompts::truncate_description` (M-1).

- **Latent `get_skill` MCP tool path resolution** (surfaced during
  US1.b). Was treating relative `row.path` strings as absolute via
  `PathBuf::from`. Promoted to the shared `resolve_entry_body_path`
  helper with the S-H1 boundary check.

### Exit codes

Six new exit codes:

- **9** — `PluginDataDirWriteFailed` (MCP-only).
- **25** — `WorkspaceDataDirWriteFailed` (MCP + `tome workspace rename`).
- **26** — `PromptArgumentMismatch`.
- **27** — `EntryNotFound`.
- **28** — `SubstitutionFailed`.
- **29** — `InvalidArgumentFrontmatter`.

Each is wired 1:1 to a `TomeError` variant per the closed-error-set
discipline.

### Dependencies

No new top-level dependencies. `regex` was promoted from transitive to
direct at Phase 5 start; no binary-size impact (already linked).

### Tests

954 → 1193 tests (+239) across 127 → 151 suites; ignored 16 unchanged.

### Polish phase notable

The phase-wide 4-reviewer pass found 0 BLOCKERS and 7 majors. Applied:
M-1 (`prompts::truncate_description` bounded char_indices walk — lifts
US4.d fix), M-2 (shared `substitution_helpers::build_context_for_entry`),
M-3 (canonical `EntryKind` dispatch over stringly-typed match),
M-4 (promoted `validate_db_stored_path` helper). Security audit clean
across the board.

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
