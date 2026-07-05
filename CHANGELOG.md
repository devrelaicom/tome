# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### BYOK/BYOM — external model providers

Each of Tome's three model capabilities can now be pointed at an external
provider instead of the bundled local model; the bundled local model stays the
default when a capability is left unconfigured (no behaviour change).

- **Added** a `[providers.<name>]` registry (`kind` = `openai` | `anthropic` |
  `gemini` | `voyage`, optional `base_url`, optional inline `api_key`) referenced
  by `provider`/`model` on `[summariser]` and the new `[embedding]` /
  `[reranker]` sections. Summarisation supports OpenAI-compatible/Anthropic/
  Gemini; embedding supports OpenAI-compatible/Voyage; reranking supports Voyage.
  OpenAI-compatible covers local servers (Ollama, LM Studio) via an explicit
  `base_url`.
- **Added** `tome models test <summariser|embedding|reranker>` — one real
  round-trip against the active configured model (remote or bundled), reporting
  latency + validated shape, writing no state.
- **Added** a `tome doctor` provider report (kind + credential-resolvable, and
  with `--verify` a reachability check) and a corrupt-index check (cost-aware
  `--fix`: bundled-local auto-reindexes; remote prints the command).
- **Credentials** resolve from `TOME_<NAME>_API_KEY` → inline `api_key` → none;
  generic third-party env vars (e.g. `OPENAI_API_KEY`) are never read, and
  credentials never appear in logs or error output.
- **Safety:** every remote embedding is content-validated fail-closed
  (non-empty, finite, non-zero-norm, correct dimension) at index time and query
  time, on both the CLI and the MCP `search_skills` path — a malformed remote
  embedding can never be written to the index or used for KNN. Switching the
  embedding model surfaces an explicit "run `tome reindex`" error rather than
  silently mixing vectors; no automatic (possibly paid) reindex is triggered.
- New exit codes **93** `ProviderConfigInvalid`, **94** `ProviderRequestFailed`,
  **95** `RemoteEmbeddingInvalid`. No new dependency; no index schema change.
- v1 non-goals: streaming, batch embedding, per-workspace overrides, reranking
  via non-Voyage providers.

### Unified global config (`~/.tome/config.toml`) — breaking changes

All global Tome settings now live in **one file**: `~/.tome/config.toml`. The
previously separate `~/.tome/settings.toml` (harness settings) and
`~/.tome/telemetry/config.toml` (opt-out) are no longer read.

**Migration steps for existing installs:**

- **Harness settings** (`~/.tome/settings.toml` → `[harness]` in config.toml):
  re-run `tome harness use <name>` for each harness you had configured globally;
  this writes the `[harness] enabled` list for you. Delete the old file for
  tidiness.
- **Telemetry opt-out** (`~/.tome/telemetry/config.toml` → `[telemetry]
  enabled = false` in config.toml): if you had run `tome telemetry off`,
  re-run it. Delete `~/.tome/telemetry/config.toml` for tidiness.
- **`[catalogs]` table** in the old config was already dead (the DB is
  authoritative); a stale table is ignored and will be dropped on next write.
- A malformed `~/.tome/config.toml` now surfaces as **exit 5** on foreground
  commands; best-effort paths (telemetry, logging, colour, summariser trigger)
  degrade gracefully.
- New `--no-color` global flag (also `NO_COLOR` env var and `[output] color =
  "never"` in config).

The complete config schema and every key's default is documented in
[README.md § Global config reference](./README.md#global-config-reference).

### Additional harness support

- **Eleven new harnesses.** `tome harness use` / `tome sync` now configure
  `copilot-cli`, `copilot` (VS Code), `devin`, `cline`, `junie`, `jetbrains-ai`,
  `antigravity`, `pi`, `crush`, `zed`, and `kiro` — on top of the existing
  Claude Code, Codex, Cursor, Gemini, and OpenCode. For each, Tome registers the
  **Tome MCP server** (where the harness exposes a writable config) and delivers
  the tiered skill-routing directive in the harness's **rules sink**, and — where
  supported — at **session start** via a session-start command hook or a
  Tome-shipped TypeScript plugin shim (executed by the harness's own runtime).
- **Opt-in `generic` / `goose` targets.** `generic` writes a universal
  `AGENTS.md` + project-root `./mcp.json`; `generic-op` (aliased `goose`) emits a
  `tome-op` Open Plugins bundle (manifest + `hooks/hooks.json` + `.mcp.json` +
  `AGENTS.md` region). Both are reachable **by name only** — never auto-detected,
  never included in `--all`.
- **`antigravity-cli` → `gemini` alias.** Resolves to the shared `~/.gemini`
  tree; aliases are resolved before de-duplication so an alias + its target never
  double-write.
- **Multi-harness selection.** `tome harness use` with no arguments configures
  every auto-detected harness; with names, exactly those (variadic); with
  `--all`, every supported real harness. `tome sync --harness <name>` is
  repeatable to reconcile a chosen subset.
- **`tome harness info <name>`** prints the exact paste-able Tome MCP-config
  snippet — the recovery path for the **manual-MCP** harnesses (`jetbrains-ai`
  is UI-only; `pi` needs the pi-mcp-adapter).
- **Self-healing rules preamble.** Every Tome-written rules sink opens with a
  harness-agnostic preamble: if the agent can't see the Tome MCP tools, it
  instructs the user to run `tome harness use <name>` (or
  `tome harness info <name>`) and restart.
- **`tome status` / `tome doctor`** report each harness's MCP state — `ok`,
  `manual`, `unverified`, or `drift`.
- The Antigravity session hook and a few first-match-wins sink behaviours are
  confirmed against a live install before shipping; a harness whose session hook
  can't be confirmed falls back to rules-only steering.

### Telemetry

- **Anonymous, opt-out usage telemetry.** Tome now collects bucketed counts,
  closed enum values, and a random per-install UUID to understand which features
  are used and where the tool breaks — **never** queries, file paths, project
  names, or any free-form text. A second, **catalog-attributed** stream sends the
  *published* name of a plugin only when its catalog resolves (at emit time) to a
  hardcoded, in-repo, PR-only allowlist (today: one — Midnight); the source is the
  gate, never the name. Both streams share one local-only install UUID.
- **Zero foreground cost.** A command or MCP tool call only appends one ≤4 KiB
  line to a local JSONL queue — no network, no blocking. Delivery is a best-effort
  background flush (a detached CLI child / an MCP timer), HTTPS-only, never within
  a 10-minute first-run grace period.
- **`tome telemetry {status,on,off,inspect,flush,reset,purge}`** — inspect and
  control telemetry; `status`/`inspect` are read-only, `off`/`TOME_TELEMETRY=0`
  disable it, `purge` deletes all local state. CI is auto-disabled. `tome doctor`
  reports the telemetry subsystem read-only.
- **Published & pinned.** [`TELEMETRY.md`](./TELEMETRY.md) documents exactly what
  is collected and is kept in sync with the code by a byte-for-byte pin test.
- New exit codes **90/91/92** (`TelemetryEndpointUnreachable` /
  `TelemetryConfigInvalid` / `TelemetryQueueCorrupt`).

### Model profiles

- **Model tiering — `small` / `medium` / `large` profiles.** A profile selects
  which embedder + reranker Tome uses, trading disk and CPU for retrieval
  quality. `small` = `bge-small-en-v1.5` (384-d) + `bge-reranker-base`;
  `medium` *(default)* = `bge-base-en-v1.5` (768-d) + `bge-reranker-large`;
  `large` = `bge-large-en-v1.5` (1024-d) + `bge-reranker-v2-m3`. Every embedder
  and reranker is a single-file quantized BGE model (MIT); the shared
  `qwen2.5-0.5b-instruct` summariser (Apache-2.0) is unchanged across profiles.
- **`tome models profile [<small|medium|large>]`** — show the active profile and
  its embedder/reranker (with per-model install state), or set it. The active
  profile is a global property stored in `index.db` `meta` (`model_profile`);
  it is not per-workspace. `--json` supported.
- **Switching the embedder requires a reindex, never a migration.** When a
  profile switch changes the embedder (and therefore the embedding dimension),
  `tome models profile <tier>` prints a clear `run \`tome reindex\`` notice and
  does **not** auto-rebuild or attempt to convert existing vectors. Re-embedding
  from the source skills is the only path; the existing drift→reindex mechanism
  is the single resolver and still blocks partial re-embeds (`plugin enable`,
  `catalog update`) until the whole index is rebuilt. A profile switch that only
  changes the reranker needs no reindex (and hints `tome models download` when
  the new reranker isn't installed).
- **Existing installs auto-map to `small`.** An index created before profiles
  existed was built with `bge-small-en-v1.5`, so it is mapped to the `small`
  profile on first open — no reindex, no re-download, seamless.
- **`tome models download` / `tome models list` are profile-aware.** `download`
  defaults to the active profile's `{embedder, reranker, summariser}`; pass
  `--all` to fetch every model in every profile. `list` annotates each row with
  the profile(s) that reference it and marks the active set (`*` / JSON
  `profiles` + `active`).

### Meta skills

- **`tome meta {list,add,remove}`** — install Tome's own bundled, trusted
  `SKILL.md` guides (native skills that teach an agent how to use Tome itself)
  into the harnesses that consume native skills (Claude Code, Cursor, Codex,
  OpenCode — not Gemini). `add` targets every detected skill-capable harness at
  project scope by default (`--global` installs under your home; `--harness
  <name>` targets named harnesses); installs land atomically and refuse to
  follow symlinks. The first bundled skill, **`convert-marketplace`**, guides a
  Claude Code marketplace → Tome conversion and reports back for confirmation
  before registering anything.
- **`tome doctor` meta-skill drift** — a read-only check reports a `stale` or
  `missing-but-expected` install for every detected harness × scope the
  installer would target; `tome doctor --fix` re-installs from the embedded
  copy.
- **MCP `meta` tool + `add-tome-conversion-skill` prompt** — from a running
  `tome mcp` server, the host harness (stamped into the server's args at
  `harness sync`) can install a meta skill; a reserved prompt drives it.

### Authoring & conversion

- **`tome {catalog,plugin,skill} create <NAME>`** — scaffold a new native Tome
  artifact from a built-in template, valid and lint-clean out of the box.
  `tome skill create <name>` wraps the skill in a minimal plugin by default
  (`--plugin-name <p>` sets the wrapping plugin → `p:<name>`; `--bare` emits a
  naked skill). `--output <dir>` chooses where it lands; `--into <dir>` drops a
  plugin into an existing catalog (registering it) or a skill into an existing
  plugin.

- **`tome {catalog,plugin,skill} convert <SOURCE>`** — convert a Claude Code
  marketplace/plugin/skill, a Codex project, or a native `SKILL.md` from Cursor /
  OpenCode / Cline / generic Agent Skills into the native Tome format. `SOURCE`
  may be a local path, an `owner/repo` shorthand, or a Git URL (fetched into a
  temp clone that is always cleaned up). Harness-isms are rewritten
  (`${CLAUDE_*}` → `${TOME_*}`, legacy `$1..$9` → 0-based), and anything Tome
  cannot represent (monitors, themes, LSP, output-styles, …) is reported as a
  warning — or, with `--strict`, aborts before writing anything. `--dry-run`
  prints the plan; `--from <harness>` overrides source-format detection.

- **`tome {catalog,plugin,skill} lint <PATH>`** — validate a native Tome
  artifact for CI: manifest validity, `name == directory`, missing descriptions,
  residual harness-isms, and unsupported components, reporting every finding in
  one run. `--strict` fails on warnings too; `--autofix` applies the
  mechanically-safe fixes (harness-ism rewrites, `name == dir`). Exit codes are
  CI-friendly (errors → 85, strict warnings → 86).

- **Native manifest cutover.** Tome now reads a plugin's native
  `tome-plugin.toml` (a legacy `plugin.json`-only plugin reports
  `not converted` with a `convert` hint). The downloaded-model manifest is TOML;
  `tome doctor --fix` migrates a legacy JSON manifest in place (no re-download).
  A new `${TOME_PROJECT_DIR}` substitution built-in resolves to the project root.

## [0.7.0] — 2026-06-16

### Fixed

- **Telemetry: eliminate the detached-flusher process storm (#225).** A burst of
  concurrent `tome` exits could each fork a background `telemetry flush` child
  before the one-minute throttle stamp gated them (a check-then-stamp TOCTOU), and
  the integration suite — every command spawned against a fresh temp root —
  multiplied that into a storm that wedged the run and wrote to the real `~/.tome`.
  The exit hook now claims its spawn window under the telemetry flush lock
  (double-checked), bounding forks to ≤ 1 per window per root even under
  concurrent starts; the test harness force-disables telemetry for every spawned
  `tome`; and every CI workflow pins `TOME_TELEMETRY=0`.

### Changed

- **Release automation no longer re-opens duplicate "Release" PRs.** release-plz
  now treats git tags as the source of truth for what's released (`git_only`)
  instead of the lagging crates.io registry, and the tag job runs before the
  release-PR job. Merging a release PR no longer spawns a duplicate one during the
  binary-build window. (`cargo publish` stays gated behind a green binary build.)

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
  v1.2.0).

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

## [0.7.13](https://github.com/devrelaicom/tome/compare/v0.7.12...v0.7.13) - 2026-07-03

### Added

- *(mcp)* get_skill include_resource_bodies; get_skill_info wildcard name + available-list ([#333](https://github.com/devrelaicom/tome/pull/333)) ([#392](https://github.com/devrelaicom/tome/pull/392))
- *(cli)* lint accepts multiple sources; convert source typed as path ([#326](https://github.com/devrelaicom/tome/pull/326)) ([#391](https://github.com/devrelaicom/tome/pull/391))
- *(cli)* workspace use --create + picker, init --bind, optional regen-summary name ([#321](https://github.com/devrelaicom/tome/pull/321)) ([#390](https://github.com/devrelaicom/tome/pull/390))
- *(mcp)* search_skills kind and min_score input filters ([#320](https://github.com/devrelaicom/tome/pull/320)) ([#389](https://github.com/devrelaicom/tome/pull/389))
- *(cli)* tier set/clear bulk retiering via --plugin, globs, and --all ([#317](https://github.com/devrelaicom/tome/pull/317)) ([#388](https://github.com/devrelaicom/tome/pull/388))
- *(cli)* reindex accepts multiple scopes, globs, and --catalog/--plugin flags ([#316](https://github.com/devrelaicom/tome/pull/316)) ([#387](https://github.com/devrelaicom/tome/pull/387))
- *(cli)* plugin enable/disable accept multiple ids and wildcard globs ([#314](https://github.com/devrelaicom/tome/pull/314)) ([#386](https://github.com/devrelaicom/tome/pull/386))
- *(cli)* add tome completions <shell> (clap_complete) ([#385](https://github.com/devrelaicom/tome/pull/385))
- *(harness)* make harness info name optional; standardize meta --harness field ([#327](https://github.com/devrelaicom/tome/pull/327)) ([#384](https://github.com/devrelaicom/tome/pull/384))
- *(authoring)* --from ValueEnum, drop convert --name, lint --dry-run requires --autofix, scope --no-fetch to catalog ([#383](https://github.com/devrelaicom/tome/pull/383))
- *(query)* --kind filter, repeatable --catalog/--plugin, variadic query text ([#319](https://github.com/devrelaicom/tome/pull/319)) ([#382](https://github.com/devrelaicom/tome/pull/382))
- *(cli)* TOME_JSON/TOME_NO_COLOR env, -w short, status <workspace> positional ([#323](https://github.com/devrelaicom/tome/pull/323)) ([#381](https://github.com/devrelaicom/tome/pull/381))
- *(catalog)* branch/tag aliases, forge shorthands, commit echo; drop inert update --force ([#329](https://github.com/devrelaicom/tome/pull/329)) ([#380](https://github.com/devrelaicom/tome/pull/380))
- *(mcp)* add get_skill raw (no-substitution) body mode ([#331](https://github.com/devrelaicom/tome/pull/331)) ([#378](https://github.com/devrelaicom/tome/pull/378))
- *(plugin)* add list --filter/--tier and show --details ([#377](https://github.com/devrelaicom/tome/pull/377))
- *(models)* add test --verify, download --profile, and profile ValueEnum ([#328](https://github.com/devrelaicom/tome/pull/328)) ([#376](https://github.com/devrelaicom/tome/pull/376))
- *(cli)* make harness/models/meta remove + meta add variadic with --all ([#315](https://github.com/devrelaicom/tome/pull/315)) ([#375](https://github.com/devrelaicom/tome/pull/375))
- *(create)* wire --description, --author, and --dry-run flags ([#325](https://github.com/devrelaicom/tome/pull/325)) ([#374](https://github.com/devrelaicom/tome/pull/374))
- *(mcp)* thread per-argument descriptions into prompts/list ([#312](https://github.com/devrelaicom/tome/pull/312)) ([#373](https://github.com/devrelaicom/tome/pull/373))
- *(telemetry)* lead first-run stderr with a welcome before the opt-out notice ([#313](https://github.com/devrelaicom/tome/pull/313)) ([#372](https://github.com/devrelaicom/tome/pull/372))
- *(plugin)* surface the interactive tome plugin browser in hints ([#311](https://github.com/devrelaicom/tome/pull/311)) ([#371](https://github.com/devrelaicom/tome/pull/371))
- *(output)* style human error prefix and dim embedded hint lines ([#310](https://github.com/devrelaicom/tome/pull/310)) ([#370](https://github.com/devrelaicom/tome/pull/370))
- *(harness)* surface opt-in targets in harness use --all ([#306](https://github.com/devrelaicom/tome/pull/306)) ([#369](https://github.com/devrelaicom/tome/pull/369))
- *(cli)* add global --non-interactive and unify skip-confirmation flags ([#305](https://github.com/devrelaicom/tome/pull/305)) ([#368](https://github.com/devrelaicom/tome/pull/368))
- *(sync)* fan bare tome sync out to bound projects ([#303](https://github.com/devrelaicom/tome/pull/303)) ([#367](https://github.com/devrelaicom/tome/pull/367))
- *(query)* add effective-knobs header and Type column to human output ([#366](https://github.com/devrelaicom/tome/pull/366))
- *(workspace)* note when [workspace] default shadows a project marker ([#302](https://github.com/devrelaicom/tome/pull/302)) ([#365](https://github.com/devrelaicom/tome/pull/365))
- *(workspace)* mark current row + relative last_used in workspace list ([#300](https://github.com/devrelaicom/tome/pull/300)) ([#364](https://github.com/devrelaicom/tome/pull/364))
- *(authoring)* give convert --json diagnostic lines lint-finding parity ([#299](https://github.com/devrelaicom/tome/pull/299)) ([#363](https://github.com/devrelaicom/tome/pull/363))
- *(convert)* bridge successful convert into lint/harness-use ([#362](https://github.com/devrelaicom/tome/pull/362))
- *(convert)* add --allow to demote strict-blocking rules ([#297](https://github.com/devrelaicom/tome/pull/297)) ([#361](https://github.com/devrelaicom/tome/pull/361))
- *(error)* structured retryable/remediation error fields ([#296](https://github.com/devrelaicom/tome/pull/296)) ([#360](https://github.com/devrelaicom/tome/pull/360))
- *(doctor)* surface unrepresented plugin hooks ([#292](https://github.com/devrelaicom/tome/pull/292)) ([#359](https://github.com/devrelaicom/tome/pull/359))
- *(mcp)* align the three-tier workflow — surface get_skill_info everywhere + match its not-found codes to get_skill ([#295](https://github.com/devrelaicom/tome/pull/295)) ([#358](https://github.com/devrelaicom/tome/pull/358))
- *(harness)* slim the always-injected session directive ([#294](https://github.com/devrelaicom/tome/pull/294)) ([#357](https://github.com/devrelaicom/tome/pull/357))
- *(cli)* make empty/zero states actionable ([#293](https://github.com/devrelaicom/tome/pull/293)) ([#356](https://github.com/devrelaicom/tome/pull/356))
- *(provider)* surface missing BYOK credential at config-resolve time ([#291](https://github.com/devrelaicom/tome/pull/291)) ([#355](https://github.com/devrelaicom/tome/pull/355))
- *(harness)* per-harness fidelity preview (tome harness preview) ([#288](https://github.com/devrelaicom/tome/pull/288)) ([#354](https://github.com/devrelaicom/tome/pull/354))
- *(config)* add `tome config show` and `tome config validate` ([#286](https://github.com/devrelaicom/tome/pull/286)) ([#353](https://github.com/devrelaicom/tome/pull/353))
- *(mcp)* actionable signal on empty/weak search_skills results ([#285](https://github.com/devrelaicom/tome/pull/285)) ([#352](https://github.com/devrelaicom/tome/pull/352))
- *(doctor)* fresh-install onboarding for status + doctor ([#283](https://github.com/devrelaicom/tome/pull/283)) ([#351](https://github.com/devrelaicom/tome/pull/351))
- *(status)* distinct exit code for degraded vs unhealthy health ([#282](https://github.com/devrelaicom/tome/pull/282)) ([#350](https://github.com/devrelaicom/tome/pull/350))
- *(plugin)* --sync flag on enable/disable to apply changes to harnesses ([#280](https://github.com/devrelaicom/tome/pull/280)) ([#349](https://github.com/devrelaicom/tome/pull/349))
- *(output)* add onboarding next: hints on success and recovery hint: on first-run errors ([#348](https://github.com/devrelaicom/tome/pull/348))
- *(plugin)* show relative last-indexed and populate last_upstream_change ([#309](https://github.com/devrelaicom/tome/pull/309)) ([#347](https://github.com/devrelaicom/tome/pull/347))
- *(mcp)* TOME_MCP_LOG to quiet or redirect the mcp.log file sink ([#307](https://github.com/devrelaicom/tome/pull/307)) ([#346](https://github.com/devrelaicom/tome/pull/346))
- *(workspace)* add `tome workspace current` for prompts and scripts ([#301](https://github.com/devrelaicom/tome/pull/301)) ([#345](https://github.com/devrelaicom/tome/pull/345))
- *(provider)* env-tunable retry count via TOME_PROVIDER_MAX_RETRIES ([#343](https://github.com/devrelaicom/tome/pull/343))

### Documentation

- *(mcp)* clarify read-tool schemas + default meta action ([#332](https://github.com/devrelaicom/tome/pull/332)) ([#379](https://github.com/devrelaicom/tome/pull/379))

### Fixed

- *(harness)* launcher-tolerant ownership for the hook + recovery sinks (#337 phase B) ([#342](https://github.com/devrelaicom/tome/pull/342))
- *(harness)* launcher-tolerant ownership for the standard MCP sink (#337 phase A) ([#341](https://github.com/devrelaicom/tome/pull/341))
- *(mcp)* make commands reachable through the MCP surface ([#289](https://github.com/devrelaicom/tome/pull/289)) ([#340](https://github.com/devrelaicom/tome/pull/340))
- *(config)* let doctor & status diagnose a malformed config instead of bricking ([#287](https://github.com/devrelaicom/tome/pull/287)) ([#339](https://github.com/devrelaicom/tome/pull/339))
- *(harness)* resolve absolute launcher for the tome-op MCP bundle ([#290](https://github.com/devrelaicom/tome/pull/290)) ([#338](https://github.com/devrelaicom/tome/pull/338))
- *(telemetry)* widen CI auto-disable detection ([#284](https://github.com/devrelaicom/tome/pull/284)) ([#335](https://github.com/devrelaicom/tome/pull/335))

## [0.7.12](https://github.com/devrelaicom/tome/compare/v0.7.11...v0.7.12) - 2026-06-30

### Other

- Native plugin-hook translation (5 harnesses: Devin/Codex/Cursor/Gemini/Copilot-CLI) ([#318](https://github.com/devrelaicom/tome/pull/318))

## [0.7.11](https://github.com/devrelaicom/tome/compare/v0.7.10...v0.7.11) - 2026-06-29

### Other

- Native-agent expansion Phase 2: 6 new harnesses (Gemini/Copilot/Kiro/Goose/Devin/Pi) ([#276](https://github.com/devrelaicom/tome/pull/276))

## [0.7.10](https://github.com/devrelaicom/tome/compare/v0.7.9...v0.7.10) - 2026-06-29

### Fixed

- *(release)* exclude CI-only refresh_model_registry bin from dist archives ([#274](https://github.com/devrelaicom/tome/pull/274))

## [0.7.9](https://github.com/devrelaicom/tome/compare/v0.7.8...v0.7.9) - 2026-06-29

### Fixed

- *(deps)* bump anyhow to 1.0.103 (RUSTSEC unsoundness) ([#272](https://github.com/devrelaicom/tome/pull/272))

## [0.7.8](https://github.com/devrelaicom/tome/compare/v0.7.7...v0.7.8) - 2026-06-29

### Added

- *(model-registry)* models.dev-sourced registry + map_model overhaul; fix OpenCode/Cursor agent emit ([#267](https://github.com/devrelaicom/tome/pull/267))

## [0.7.7](https://github.com/devrelaicom/tome/compare/v0.7.6...v0.7.7) - 2026-06-26

### Added

- *(telemetry)* migrate to gauge-telemetry kernel (keep 2-tier streams) ([#265](https://github.com/devrelaicom/tome/pull/265))

## [0.7.6](https://github.com/devrelaicom/tome/compare/v0.7.5...v0.7.6) - 2026-06-25

### Other

- Phase 12 — BYOK/BYOM model providers ([#263](https://github.com/devrelaicom/tome/pull/263))

## [0.7.5](https://github.com/devrelaicom/tome/compare/v0.7.4...v0.7.5) - 2026-06-23

### Added

- *(config)* unify global config into ~/.tome/config.toml ([#255](https://github.com/devrelaicom/tome/pull/255))

## [0.7.4](https://github.com/devrelaicom/tome/compare/v0.7.3...v0.7.4) - 2026-06-22

### Added

- model tiering — small/medium/large profiles + dimension-free vector storage ([#247](https://github.com/devrelaicom/tome/pull/247))

## [0.7.3](https://github.com/devrelaicom/tome/compare/v0.7.2...v0.7.3) - 2026-06-19

### Other

- Phase 11: Additional harness support (5 → ~16 harnesses) ([#252](https://github.com/devrelaicom/tome/pull/252))

## [0.7.2](https://github.com/devrelaicom/tome/compare/v0.7.1...v0.7.2) - 2026-06-18

### Added

- MCP live-sync + unified `tome sync` + `session-start` ([#239](https://github.com/devrelaicom/tome/pull/239))

## [0.7.1](https://github.com/devrelaicom/tome/compare/v0.7.0...v0.7.1) - 2026-06-17

### Added

- Tome-owned SessionStart hook for Codex ([#238](https://github.com/devrelaicom/tome/pull/238))
- tiered skill-routing instructions ([#237](https://github.com/devrelaicom/tome/pull/237))
- *(lint)* warn when a skill exceeds the get_skill MCP token budget ([#235](https://github.com/devrelaicom/tome/pull/235))

## [0.6.2](https://github.com/devrelaicom/tome/compare/v0.6.1...v0.6.2) - 2026-06-16

### Fixed

- *(summarise)* set n_batch to the context size and degrade post-commit summary-trigger failures ([#229](https://github.com/devrelaicom/tome/pull/229))

## [0.6.1](https://github.com/devrelaicom/tome/compare/v0.6.0...v0.6.1) - 2026-06-16

### Added

- *(status)* bookshelf redesign — colored ASCII shelf + grouped panel + enriched report ([#224](https://github.com/devrelaicom/tome/pull/224))

## [0.6.0](https://github.com/devrelaicom/tome/releases/tag/v0.6.0) - 2026-06-16

### Added

- *(telemetry)* P10 US5 — verifiable transparency (TELEMETRY.md + doctor) ([#215](https://github.com/devrelaicom/tome/pull/215))
- *(telemetry)* P10 US4 — named adoption signal for allowlisted catalogs only ([#214](https://github.com/devrelaicom/tome/pull/214))
- *(telemetry)* P10 US3 — best-effort delivery off the foreground path ([#213](https://github.com/devrelaicom/tome/pull/213))
- *(telemetry)* P10 US2 — anonymous stream captured at zero foreground cost ([#212](https://github.com/devrelaicom/tome/pull/212))
- *(telemetry)* P10 US1 (MVP) — consent, identity & off switch ([#211](https://github.com/devrelaicom/tome/pull/211))
- *(telemetry)* typed event API + buckets + config/enabled gate + clock/transport seams ([#210](https://github.com/devrelaicom/tome/pull/210))
- *(telemetry)* module skeleton, ErrorCategory refactor, exit codes 90-92, getrandom/rustix ([#209](https://github.com/devrelaicom/tome/pull/209))
- *(meta)* US4 — doctor meta-skill drift report + --fix re-install ([#203](https://github.com/devrelaicom/tome/pull/203))
- *(meta)* US3 — MCP meta tool + reserved prompt + host-harness stamping ([#201](https://github.com/devrelaicom/tome/pull/201))
- *(meta)* author convert-marketplace guided skill + unsupported-component rubric ([#200](https://github.com/devrelaicom/tome/pull/200))
- *(meta)* tome meta {list,add,remove} CLI over the shared install path ([#199](https://github.com/devrelaicom/tome/pull/199))
- *(meta)* Phase 9 foundational — embed pipeline + shared compute + harness skill-emit trait ([#198](https://github.com/devrelaicom/tome/pull/198))
- *(authoring)* create + built-in templates (P8 US4-1) ([#196](https://github.com/devrelaicom/tome/pull/196))
- *(authoring)* lint --autofix + lint command surface (P8 US3) ([#194](https://github.com/devrelaicom/tome/pull/194))
- *(authoring)* lint rules + native-artifact parser (P8 US3) ([#193](https://github.com/devrelaicom/tome/pull/193))
- *(authoring)* convert --into injection (P8 US2) ([#191](https://github.com/devrelaicom/tome/pull/191))
- *(authoring)* remote SOURCE fetch with guaranteed cleanup (P8 US2) ([#190](https://github.com/devrelaicom/tome/pull/190))
- *(authoring)* CC marketplace → catalog convert + detect fix (P8 US2) ([#189](https://github.com/devrelaicom/tome/pull/189))
- *(authoring)* Codex project importer (Tier 2 synthesis) (P8 US2) ([#188](https://github.com/devrelaicom/tome/pull/188))
- *(authoring)* native SKILL.md convert + tome skill convert (P8 US2) ([#187](https://github.com/devrelaicom/tome/pull/187))
- *(authoring)* convert pipeline + `tome plugin convert` (P8 US2) ([#186](https://github.com/devrelaicom/tome/pull/186))
- *(authoring)* Claude Code → IR plugin importer (P8 US2) ([#185](https://github.com/devrelaicom/tome/pull/185))
- *(authoring)* untrusted-read guard + source-format detection (P8 US2) ([#184](https://github.com/devrelaicom/tome/pull/184))
- *(doctor)* migrate legacy model manifest + report unconverted plugins (US1) ([#182](https://github.com/devrelaicom/tome/pull/182))
- *(substitution)* add ${TOME_PROJECT_DIR} builtin (US1) ([#181](https://github.com/devrelaicom/tome/pull/181))
- *(plugin)* read native tome-plugin.toml; PluginNotConverted on legacy (US1 cutover) ([#179](https://github.com/devrelaicom/tome/pull/179))
- *(authoring)* harness-ism rewriter and lint runner framework (P8 foundational B) ([#178](https://github.com/devrelaicom/tome/pull/178))
- *(authoring)* TomePluginManifest, artifact IR, and the emitter ([#177](https://github.com/devrelaicom/tome/pull/177))
- *(harness)* symlink-safe write primitive across all sinks (FR-007; intermediate-component guard) ([#153](https://github.com/devrelaicom/tome/pull/153))
- *(phase-6)* US5 — privilege governance + doctor extensions ([#136](https://github.com/devrelaicom/tome/pull/136))
- *(phase-6)* US4 — agent personas via MCP prompts ([#135](https://github.com/devrelaicom/tome/pull/135))
- *(phase-6)* US3 — guardrails fallback + Claude Code rules-file correction ([#134](https://github.com/devrelaicom/tome/pull/134))
- *(phase-6)* US2 — real Claude Code hooks ([#133](https://github.com/devrelaicom/tome/pull/133))
- *(phase-6)* US1 — native agents across four harnesses ([#132](https://github.com/devrelaicom/tome/pull/132))
- *(phase-6)* Foundational — error codes 43-46, EntryKind::Agent, HarnessModule trait ([#131](https://github.com/devrelaicom/tome/pull/131))
- *(phase-5/us5c)* reviewer pass + Phase 7 closeout ([#127](https://github.com/devrelaicom/tome/pull/127))
- *(phase-5/us5b)* plugin show + doctor — Phase 5 surfaces ([#125](https://github.com/devrelaicom/tome/pull/125))
- *(phase-5/us4c+d)* search_skills extensions + reviewer pass (HIGH DoS fix + closeout) ([#123](https://github.com/devrelaicom/tome/pull/123))
- *(index)* when_to_use contributes to embedding_text (US4.b verification) ([#122](https://github.com/devrelaicom/tome/pull/122))
- *(phase-5/us4a)* get_skill_info middle-tier MCP tool + resource enumeration ([#121](https://github.com/devrelaicom/tome/pull/121))
- *(phase-5/us3c+d)* substitution end-to-end + reviewer pass (0 BLOCKERS; substitution layer COMPLETE) ([#120](https://github.com/devrelaicom/tome/pull/120))
- *(phase-5/us3a+b)* argument substitution Stage 3 + ARGUMENTS append-fallback Stage 4 ([#119](https://github.com/devrelaicom/tome/pull/119))
- *(phase-5/us2c+d)* substitution end-to-end + reviewer pass (2 BLOCKERS fixed; data exfiltration vector closed) ([#118](https://github.com/devrelaicom/tome/pull/118))
- *(phase-5/us2b)* substitution env passthrough + data-dir + rename relocation ([#117](https://github.com/devrelaicom/tome/pull/117))
- *(phase-5/us2a)* substitution built-ins stage + clock injection + path sanitisation ([#116](https://github.com/devrelaicom/tome/pull/116))
- *(phase-5/us1c)* prompts/get + substitution wiring + shared path resolver ([#114](https://github.com/devrelaicom/tome/pull/114))
- *(phase-5/us1b)* MCP prompts capability — prompts/list + name derivation + collisions ([#113](https://github.com/devrelaicom/tome/pull/113))
- *(phase-5/us1a)* schema v3 + frontmatter widening + commands walk + EntryKind ([#112](https://github.com/devrelaicom/tome/pull/112))
- *(phase-5)* foundational F1+F2+F3 — error variants, regex, substitution skeleton ([#111](https://github.com/devrelaicom/tome/pull/111))
- *(phase-4)* apply Polish PR-C selected majors (C-M1 + C-M9 + C-M12 + R-M3 + R-M4 + R-M5 + R-M7 + R-M8 + R-M12) ([#105](https://github.com/devrelaicom/tome/pull/105))
- *(doctor)* --fix handlers for Phase 4 subsystems + --force override + orphan cleanup ([#100](https://github.com/devrelaicom/tome/pull/100))
- *(summarise)* trigger wiring + MCP cached-short readout + forward-progress (FR-380/381/382/385/425) ([#95](https://github.com/devrelaicom/tome/pull/95))
- *(harness)* tome harness command surface (bare/list/use/remove/info/sync) ([#91](https://github.com/devrelaicom/tome/pull/91))
- *(settings)* composition validation rules (workspace-ref, bad-exclusion, unsupported) ([#89](https://github.com/devrelaicom/tome/pull/89))
- *(workspace)* sync CLI command (per-workspace + all-workspaces) ([#85](https://github.com/devrelaicom/tome/pull/85))
- *(workspace)* remove with 5-step cascade + refcount-clean catalog caches ([#84](https://github.com/devrelaicom/tome/pull/84))
- *(workspace)* rename + regen-summary + sync helper for bound projects ([#83](https://github.com/devrelaicom/tome/pull/83))
- *(harness)* claude-code production harness + end-to-end bind test ([#78](https://github.com/devrelaicom/tome/pull/78))
- *(harness)* sync algorithm orchestrator + StubHarness end-to-end ([#77](https://github.com/devrelaicom/tome/pull/77))
- *(harness)* mcp_config primitives (JSON + TOML) + idempotence ([#76](https://github.com/devrelaicom/tome/pull/76))
- *(harness)* rules_file primitives + StubHarness fixture ([#75](https://github.com/devrelaicom/tome/pull/75))
- *(migrations)* register phase_4_v1_to_v2; bootstrap emits v2 directly ([#67](https://github.com/devrelaicom/tome/pull/67))
- *(settings)* layered settings parser + composition resolver skeleton ([#66](https://github.com/devrelaicom/tome/pull/66))
- *(harness)* harness skeleton (HarnessModule trait, 5 stub impls, rules_file + mcp_config stubs) ([#65](https://github.com/devrelaicom/tome/pull/65))
- *(summarise)* summariser skeleton (Summariser trait, LlamaBackend singleton, StubSummariser, prompts) ([#64](https://github.com/devrelaicom/tome/pull/64))
- *(catalog)* refuse remove on enabled plugins; cascade with --force ([#32](https://github.com/devrelaicom/tome/pull/32))
- *(status,version)* add tome status; extend --version ([#29](https://github.com/devrelaicom/tome/pull/29))
- *(reindex)* add tome reindex subcommand ([#27](https://github.com/devrelaicom/tome/pull/27))
- *(catalog)* reindex enabled plugins on update; auto-disable orphans ([#26](https://github.com/devrelaicom/tome/pull/26))
- *(reindex)* add reindex_plugin_atomic + lifecycle wrappers ([#25](https://github.com/devrelaicom/tome/pull/25))
- *(plugin)* add interactive catalog/plugin browse flow ([#16](https://github.com/devrelaicom/tome/pull/16))
- *(query)* add tome query with reranker + strict mode ([#13](https://github.com/devrelaicom/tome/pull/13))

### Changed

- *(models)* model manifest json -> toml (US1 cutover) ([#180](https://github.com/devrelaicom/tome/pull/180))
- *(phase-7)* remove dead reference_count; sweep stale doc-comments; strip internal citations from --help (FR-016) ([#158](https://github.com/devrelaicom/tome/pull/158))
- *(phase-7)* move reconcile_hooks; sync.rs is now a thin orchestrator (no behaviour change) ([#144](https://github.com/devrelaicom/tome/pull/144))
- *(phase-7)* move reconcile_guardrails into harness/reconcile/guardrails.rs (no behaviour change) ([#143](https://github.com/devrelaicom/tome/pull/143))
- *(phase-7)* move reconcile_agents into harness/reconcile/agents.rs (no behaviour change) ([#142](https://github.com/devrelaicom/tome/pull/142))
- *(catalog)* rewire onto workspace_catalogs junction; derive metadata from filesystem ([#70](https://github.com/devrelaicom/tome/pull/70))
- *(plugin,index)* thread resolved workspace name through SQL queries ([#69](https://github.com/devrelaicom/tome/pull/69))
- *(workspace)* WorkspaceName newtype + Scope reshape (workspace names, not paths) ([#68](https://github.com/devrelaicom/tome/pull/68))
- *(util)* promote atomic-populated-directory helper to src/util/atomic_dir.rs ([#62](https://github.com/devrelaicom/tome/pull/62))
- *(paths)* collapse XDG-separated paths under <home>/.tome/; drop workspace inventory; read-only DB open across read paths ([#60](https://github.com/devrelaicom/tome/pull/60))
- tidy small code-review findings ([#37](https://github.com/devrelaicom/tome/pull/37))

### Dependencies

- *(deps)* audit serde_json/preserve_order + toml_edit scope ([#63](https://github.com/devrelaicom/tome/pull/63))

### Documentation

- *(readme)* restructure for consumer onboarding + fix stale examples ([#217](https://github.com/devrelaicom/tome/pull/217))
- *(phase-7)* P9 closeout — CHANGELOG v0.6.0 + CLAUDE.md + retro; fix 2 stale test comments ([#166](https://github.com/devrelaicom/tome/pull/166))
- *(review)* phase-7 phase-wide findings + disposition ([#163](https://github.com/devrelaicom/tome/pull/163))
- *(phase-7)* README front door + SECURITY.md + trust-model doc (FR-021/022/010) ([#162](https://github.com/devrelaicom/tome/pull/162))
- *(phase-7)* crate discovery metadata + docs.rs config + CHANGELOG/[Unreleased] reorder (FR-025) ([#160](https://github.com/devrelaicom/tome/pull/160))
- *(phase-7)* amend constitution → v1.4.0 (authorise release tooling) ([#140](https://github.com/devrelaicom/tome/pull/140))
- *(phase-7)* land planning artifacts + beta-readiness audits ([#138](https://github.com/devrelaicom/tome/pull/138))
- *(phase-6)* planning artifacts — spec, plan, research, contracts, tasks ([#130](https://github.com/devrelaicom/tome/pull/130))
- *(codebase,retro,claude-md)* close Phase 4 Polish (PR-G) ([#109](https://github.com/devrelaicom/tome/pull/109))
- *(review)* Phase 4 Polish phase-wide reviewer findings + disposition (PR-A) ([#103](https://github.com/devrelaicom/tome/pull/103))
- *(codebase,retro,claude-md)* refresh after Phase 4 / US5 ([#102](https://github.com/devrelaicom/tome/pull/102))
- *(codebase,retro,claude-md)* refresh after Phase 4 / US4 ([#98](https://github.com/devrelaicom/tome/pull/98))
- *(codebase,retro,claude-md)* refresh after Phase 4 / US3 ([#93](https://github.com/devrelaicom/tome/pull/93))
- *(codebase,retro,claude-md)* refresh after Phase 4 / US2 ([#87](https://github.com/devrelaicom/tome/pull/87))
- *(codebase,retro,claude-md)* refresh after Phase 4 / US1 ([#81](https://github.com/devrelaicom/tome/pull/81))
- *(codebase,retro,claude-md)* refresh after Phase 4 / F1–F11 Foundational ([#73](https://github.com/devrelaicom/tome/pull/73))
- README + CHANGELOG + contract reconciliation + v0.3.0 closeout ([#58](https://github.com/devrelaicom/tome/pull/58))
- Phase 10 retro and task closeout ([#41](https://github.com/devrelaicom/tome/pull/41))
- Phase 2 README, CHANGELOG, CLAUDE.md updates ([#40](https://github.com/devrelaicom/tome/pull/40))
- *(spec)* reconcile Phase 2 contracts with shipped behaviour ([#35](https://github.com/devrelaicom/tome/pull/35))
- Phase 9 codebase refresh + retro ([#33](https://github.com/devrelaicom/tome/pull/33))
- Phase 8 codebase refresh + retro ([#31](https://github.com/devrelaicom/tome/pull/31))
- Phase 7 codebase refresh + retro ([#28](https://github.com/devrelaicom/tome/pull/28))
- Phase 6 codebase refresh + retro ([#24](https://github.com/devrelaicom/tome/pull/24))
- Phase 5 codebase refresh + retro ([#21](https://github.com/devrelaicom/tome/pull/21))
- Phase 4 codebase refresh + retro ([#18](https://github.com/devrelaicom/tome/pull/18))
- codebase refresh + finalise Phase 3 retro ([#15](https://github.com/devrelaicom/tome/pull/15))

### Fixed

- convert-funnel fixes — remote-plugin fetch, detection tie-break, hooks pass-through + papercuts ([#206](https://github.com/devrelaicom/tome/pull/206))
- *(meta)* P9 phase-wide review + docs (Polish) ([#204](https://github.com/devrelaicom/tome/pull/204))
- *(authoring)* P8 phase-wide review — symlink-safe lint, SSOT dedupe, docs, +tests ([#197](https://github.com/devrelaicom/tome/pull/197))
- *(authoring)* US3 closeout — lint catalog source validation + never-halt (P8) ([#195](https://github.com/devrelaicom/tome/pull/195))
- *(authoring)* US2 closeout — emit-sink write containment + reviewer fixes (P8) ([#192](https://github.com/devrelaicom/tome/pull/192))
- *(us1-closeout)* doctor --fix re-download bug, flake fix, coverage + bounded read ([#183](https://github.com/devrelaicom/tome/pull/183))
- *(mcp)* resolve scoped catalog from the DB in get_skill + search_skills (publish-blocker 3/3) ([#171](https://github.com/devrelaicom/tome/pull/171))
- *(plugin,reindex,query)* resolve scoped catalog discovery from the DB (publish-blocker 2/3) ([#170](https://github.com/devrelaicom/tome/pull/170))
- *(plugin)* resolve plugin dir from catalog DB, not the never-written config.toml (publish-blocker, 1/3) ([#168](https://github.com/devrelaicom/tome/pull/168))
- *(models)* bound aux model-file downloads + correct SECURITY.md verification scope (P9 MAJOR-2) ([#165](https://github.com/devrelaicom/tome/pull/165))
- *(harness)* agents cleanup-removal symlink refusal returns its dedicated code 45 (CON-1) ([#164](https://github.com/devrelaicom/tome/pull/164))
- *(phase-7)* off-spec inputs fail closed; config parse → exit 5; duplicate (kind,name) warned + truthfully counted (FR-013/014/015) ([#157](https://github.com/devrelaicom/tome/pull/157))
- *(catalog)* re-derive remove --force cascade inside the lock (F-REMOVE-TOCTOU) ([#155](https://github.com/devrelaicom/tome/pull/155))
- *(harness)* write inline rules body when any sharer needs it so OpenCode receives Tome's rules (F-RULES-OPENCODE) ([#154](https://github.com/devrelaicom/tome/pull/154))
- *(plugin,catalog)* bound every third-party read by its per-class cap (F-PLUGIN-MANIFEST-DOS class) ([#152](https://github.com/devrelaicom/tome/pull/152))
- *(workspace)* emit settings.toml via toml_edit + reject control chars in catalog names (F-WS-TOML-NEWLINE) ([#151](https://github.com/devrelaicom/tome/pull/151))
- *(mcp)* assign prompt names against a single global taken-set (F-MCP-PROMPT-COLLISION) ([#150](https://github.com/devrelaicom/tome/pull/150))
- *(query)* over-fetch+widen so filtered KNN returns min(top_k, matches) (F-KNN) ([#145](https://github.com/devrelaicom/tome/pull/145))
- *(catalog)* key cache dir + refcount by scrubbed URL so SSH sources round-trip (F-CACHE-KEY) ([#149](https://github.com/devrelaicom/tome/pull/149))
- *(models)* re-pin embedder to CPU-compatible INT8 ONNX artefact (F-MODEL-ONNX-CPU) ([#148](https://github.com/devrelaicom/tome/pull/148))
- *(doctor)* open index read-only, degrade not abort on schema mismatch (F-DOCTOR-RW) ([#147](https://github.com/devrelaicom/tome/pull/147))
- *(models)* download all required model files (tokenizer), not just the primary .onnx (F-MODEL-FILES) ([#146](https://github.com/devrelaicom/tome/pull/146))
- *(phase-5/us1d)* reviewer pass closeout — 1 blocker + 8 majors + docs refresh ([#115](https://github.com/devrelaicom/tome/pull/115))
- *(phase-4)* apply Polish PR-E security hardening (S-M1 + S-M2 + S-M6 + S-M7 + T-M8 + T416 + T419) ([#107](https://github.com/devrelaicom/tome/pull/107))
- *(phase-4)* apply Phase 4 Polish blockers C-B1 + C-B2 + C-B3 (PR-B) ([#104](https://github.com/devrelaicom/tome/pull/104))
- *(doctor)* US5 reviewer-flagged fixups (1 blocker + 10 majors) ([#101](https://github.com/devrelaicom/tome/pull/101))
- *(summarise)* US4 reviewer-flagged fixups (4 blockers + 9 majors) ([#97](https://github.com/devrelaicom/tome/pull/97))
- *(harness,settings)* US3 reviewer-flagged fixups ([#92](https://github.com/devrelaicom/tome/pull/92))
- *(workspace,catalog)* US2 reviewer-flagged fixups ([#86](https://github.com/devrelaicom/tome/pull/86))
- *(workspace,harness)* US1 reviewer-flagged fixups ([#80](https://github.com/devrelaicom/tome/pull/80))
- *(security)* mcp.log 0600, symlink rejection, registry validation, init refusal ([#56](https://github.com/devrelaicom/tome/pull/56))
- *(doctor)* orphan clones, workspace registry status, schema fix, signature ([#55](https://github.com/devrelaicom/tome/pull/55))
- *(mcp)* signal handling, log scrubbing, log taxonomy, specific-over-generic ([#54](https://github.com/devrelaicom/tome/pull/54))
- *(workspace)* enforce §Validation 1b/1c in resolver; add doctor drift tests ([#53](https://github.com/devrelaicom/tome/pull/53))
- *(mcp/log)* emit contract-pinned field names (ts/level/target/msg) ([#52](https://github.com/devrelaicom/tome/pull/52))
- *(security)* scrub catalog URL on add, chmod config 0600, ignore harness state ([#36](https://github.com/devrelaicom/tome/pull/36))
- *(catalog)* report real per-plugin skills_dropped in cascade ([#34](https://github.com/devrelaicom/tome/pull/34))

### Other

- Phase 6 Polish + v0.6.0 release ([#137](https://github.com/devrelaicom/tome/pull/137))
- Phase 5 Polish + v0.5.0 release ([#128](https://github.com/devrelaicom/tome/pull/128))
- doctor Phase 4 subsystems + Subsystem enum promotion + detected-uninstalled ([#99](https://github.com/devrelaicom/tome/pull/99))
- production LlamaSummariser + Qwen2.5 + tome models extension ([#94](https://github.com/devrelaicom/tome/pull/94))
- cross-harness module tests for all 5 production harnesses ([#90](https://github.com/devrelaicom/tome/pull/90))
- composition resolver verification + comprehensive tests ([#88](https://github.com/devrelaicom/tome/pull/88))
- workspace init + list + info Phase 4 fields ([#82](https://github.com/devrelaicom/tome/pull/82))
- tome workspace use <name> — core binding flow ([#74](https://github.com/devrelaicom/tome/pull/74))
- pre-allocate 8 new TomeError variants (codes 13–19, 24) ([#61](https://github.com/devrelaicom/tome/pull/61))
- Phase 4 setup + F1: deps + constitution v1.3.0 amendment ([#59](https://github.com/devrelaicom/tome/pull/59))
- Phase 3 Polish PR-A: review findings + disposition ([#51](https://github.com/devrelaicom/tome/pull/51))
- Phase 3 / US5: forward schema migrations ([#50](https://github.com/devrelaicom/tome/pull/50))
- Phase 3 / US4: tome doctor + --fix repairs ([#49](https://github.com/devrelaicom/tome/pull/49))
- Phase 3 / US3: per-command scope honouring + reference-counted catalog cleanup ([#48](https://github.com/devrelaicom/tome/pull/48))
- Phase 3 / US2: tome workspace info + init ([#47](https://github.com/devrelaicom/tome/pull/47))
- Phase 3 / US1: tome mcp — stdio MCP server with search_skills + get_skill ([#46](https://github.com/devrelaicom/tome/pull/46))
- Phase 3 Foundational F7+F8: schema-migration framework + MCP scaffolding ([#45](https://github.com/devrelaicom/tome/pull/45))
- Phase 3 Foundational (F1-F6): Scope + per-scope Paths + resolution + read-only DB + query::run_with_deps ([#44](https://github.com/devrelaicom/tome/pull/44))
- Phase 3 Setup: plan artefacts + rmcp/tokio dependencies ([#43](https://github.com/devrelaicom/tome/pull/43))
- Phase 6 US4 slice 2: integration tests for models download / list / remove ([#23](https://github.com/devrelaicom/tome/pull/23))
- Phase 6 US4 slice 1: tome models download / list / remove ([#22](https://github.com/devrelaicom/tome/pull/22))
- Phase 5 US3 slice 2: integration tests for disable, cheap re-enable, repeated state ([#20](https://github.com/devrelaicom/tome/pull/20))
- Phase 5 US3 slice 1: tome plugin disable + cheap re-enable verification ([#19](https://github.com/devrelaicom/tome/pull/19))
- Phase 3 US1 slice 3: integration tests for plugin enable/list/show/query + atomicity ([#14](https://github.com/devrelaicom/tome/pull/14))
- Phase 3 US1 slice 1b: tome plugin enable / list / show CLI surface ([#12](https://github.com/devrelaicom/tome/pull/12))
- Phase 3 US1 slice 1a: lifecycle::enable / disable + pinned MODEL_REGISTRY ([#11](https://github.com/devrelaicom/tome/pull/11))
- Phase 2 foundational (7/N): model-download integration test + docs + retro ([#10](https://github.com/devrelaicom/tome/pull/10))
- Phase 2 foundational (6/N): extend credential scrubber to model download ([#9](https://github.com/devrelaicom/tome/pull/9))
- Phase 2 foundational (5/N): embedding pipeline core ([#8](https://github.com/devrelaicom/tome/pull/8))
- Phase 2 foundational (4b/N): index features — lock, meta, integrity, skills, query ([#7](https://github.com/devrelaicom/tome/pull/7))
- Phase 2 foundational (4a/N): index bootstrap pipeline ([#6](https://github.com/devrelaicom/tome/pull/6))
- Phase 2 foundational (3/N): plugin metadata parsers ([#5](https://github.com/devrelaicom/tome/pull/5))
- Phase 2 foundational (2/N): presentation primitives + git-hooks migration ([#4](https://github.com/devrelaicom/tome/pull/4))
- Phase 2 foundational (1/N): error surface + paths ([#3](https://github.com/devrelaicom/tome/pull/3))
- Phase 2: setup — deps, vendored sqlite-vec, build infra ([#2](https://github.com/devrelaicom/tome/pull/2))
- Phase 1: Project Foundations and Catalog Management ([#1](https://github.com/devrelaicom/tome/pull/1))

### Chore

- *(phase-7)* [**breaking**] rename crate tome→tome-mcp, keep [[bin]] name = tome (FR-017) ([#159](https://github.com/devrelaicom/tome/pull/159))

## [0.6.0](https://github.com/devrelaicom/tome/releases/tag/v0.6.0) - 2026-06-12

### Added

- *(telemetry)* P10 US5 — verifiable transparency (TELEMETRY.md + doctor) ([#215](https://github.com/devrelaicom/tome/pull/215))
- *(telemetry)* P10 US4 — named adoption signal for allowlisted catalogs only ([#214](https://github.com/devrelaicom/tome/pull/214))
- *(telemetry)* P10 US3 — best-effort delivery off the foreground path ([#213](https://github.com/devrelaicom/tome/pull/213))
- *(telemetry)* P10 US2 — anonymous stream captured at zero foreground cost ([#212](https://github.com/devrelaicom/tome/pull/212))
- *(telemetry)* P10 US1 (MVP) — consent, identity & off switch ([#211](https://github.com/devrelaicom/tome/pull/211))
- *(telemetry)* typed event API + buckets + config/enabled gate + clock/transport seams ([#210](https://github.com/devrelaicom/tome/pull/210))
- *(telemetry)* module skeleton, ErrorCategory refactor, exit codes 90-92, getrandom/rustix ([#209](https://github.com/devrelaicom/tome/pull/209))
- *(meta)* US4 — doctor meta-skill drift report + --fix re-install ([#203](https://github.com/devrelaicom/tome/pull/203))
- *(meta)* US3 — MCP meta tool + reserved prompt + host-harness stamping ([#201](https://github.com/devrelaicom/tome/pull/201))
- *(meta)* author convert-marketplace guided skill + unsupported-component rubric ([#200](https://github.com/devrelaicom/tome/pull/200))
- *(meta)* tome meta {list,add,remove} CLI over the shared install path ([#199](https://github.com/devrelaicom/tome/pull/199))
- *(meta)* Phase 9 foundational — embed pipeline + shared compute + harness skill-emit trait ([#198](https://github.com/devrelaicom/tome/pull/198))
- *(authoring)* create + built-in templates (P8 US4-1) ([#196](https://github.com/devrelaicom/tome/pull/196))
- *(authoring)* lint --autofix + lint command surface (P8 US3) ([#194](https://github.com/devrelaicom/tome/pull/194))
- *(authoring)* lint rules + native-artifact parser (P8 US3) ([#193](https://github.com/devrelaicom/tome/pull/193))
- *(authoring)* convert --into injection (P8 US2) ([#191](https://github.com/devrelaicom/tome/pull/191))
- *(authoring)* remote SOURCE fetch with guaranteed cleanup (P8 US2) ([#190](https://github.com/devrelaicom/tome/pull/190))
- *(authoring)* CC marketplace → catalog convert + detect fix (P8 US2) ([#189](https://github.com/devrelaicom/tome/pull/189))
- *(authoring)* Codex project importer (Tier 2 synthesis) (P8 US2) ([#188](https://github.com/devrelaicom/tome/pull/188))
- *(authoring)* native SKILL.md convert + tome skill convert (P8 US2) ([#187](https://github.com/devrelaicom/tome/pull/187))
- *(authoring)* convert pipeline + `tome plugin convert` (P8 US2) ([#186](https://github.com/devrelaicom/tome/pull/186))
- *(authoring)* Claude Code → IR plugin importer (P8 US2) ([#185](https://github.com/devrelaicom/tome/pull/185))
- *(authoring)* untrusted-read guard + source-format detection (P8 US2) ([#184](https://github.com/devrelaicom/tome/pull/184))
- *(doctor)* migrate legacy model manifest + report unconverted plugins (US1) ([#182](https://github.com/devrelaicom/tome/pull/182))
- *(substitution)* add ${TOME_PROJECT_DIR} builtin (US1) ([#181](https://github.com/devrelaicom/tome/pull/181))
- *(plugin)* read native tome-plugin.toml; PluginNotConverted on legacy (US1 cutover) ([#179](https://github.com/devrelaicom/tome/pull/179))
- *(authoring)* harness-ism rewriter and lint runner framework (P8 foundational B) ([#178](https://github.com/devrelaicom/tome/pull/178))
- *(authoring)* TomePluginManifest, artifact IR, and the emitter ([#177](https://github.com/devrelaicom/tome/pull/177))
- *(harness)* symlink-safe write primitive across all sinks (FR-007; intermediate-component guard) ([#153](https://github.com/devrelaicom/tome/pull/153))
- *(phase-6)* US5 — privilege governance + doctor extensions ([#136](https://github.com/devrelaicom/tome/pull/136))
- *(phase-6)* US4 — agent personas via MCP prompts ([#135](https://github.com/devrelaicom/tome/pull/135))
- *(phase-6)* US3 — guardrails fallback + Claude Code rules-file correction ([#134](https://github.com/devrelaicom/tome/pull/134))
- *(phase-6)* US2 — real Claude Code hooks ([#133](https://github.com/devrelaicom/tome/pull/133))
- *(phase-6)* US1 — native agents across four harnesses ([#132](https://github.com/devrelaicom/tome/pull/132))
- *(phase-6)* Foundational — error codes 43-46, EntryKind::Agent, HarnessModule trait ([#131](https://github.com/devrelaicom/tome/pull/131))
- *(phase-5/us5c)* reviewer pass + Phase 7 closeout ([#127](https://github.com/devrelaicom/tome/pull/127))
- *(phase-5/us5b)* plugin show + doctor — Phase 5 surfaces ([#125](https://github.com/devrelaicom/tome/pull/125))
- *(phase-5/us4c+d)* search_skills extensions + reviewer pass (HIGH DoS fix + closeout) ([#123](https://github.com/devrelaicom/tome/pull/123))
- *(index)* when_to_use contributes to embedding_text (US4.b verification) ([#122](https://github.com/devrelaicom/tome/pull/122))
- *(phase-5/us4a)* get_skill_info middle-tier MCP tool + resource enumeration ([#121](https://github.com/devrelaicom/tome/pull/121))
- *(phase-5/us3c+d)* substitution end-to-end + reviewer pass (0 BLOCKERS; substitution layer COMPLETE) ([#120](https://github.com/devrelaicom/tome/pull/120))
- *(phase-5/us3a+b)* argument substitution Stage 3 + ARGUMENTS append-fallback Stage 4 ([#119](https://github.com/devrelaicom/tome/pull/119))
- *(phase-5/us2c+d)* substitution end-to-end + reviewer pass (2 BLOCKERS fixed; data exfiltration vector closed) ([#118](https://github.com/devrelaicom/tome/pull/118))
- *(phase-5/us2b)* substitution env passthrough + data-dir + rename relocation ([#117](https://github.com/devrelaicom/tome/pull/117))
- *(phase-5/us2a)* substitution built-ins stage + clock injection + path sanitisation ([#116](https://github.com/devrelaicom/tome/pull/116))
- *(phase-5/us1c)* prompts/get + substitution wiring + shared path resolver ([#114](https://github.com/devrelaicom/tome/pull/114))
- *(phase-5/us1b)* MCP prompts capability — prompts/list + name derivation + collisions ([#113](https://github.com/devrelaicom/tome/pull/113))
- *(phase-5/us1a)* schema v3 + frontmatter widening + commands walk + EntryKind ([#112](https://github.com/devrelaicom/tome/pull/112))
- *(phase-5)* foundational F1+F2+F3 — error variants, regex, substitution skeleton ([#111](https://github.com/devrelaicom/tome/pull/111))
- *(phase-4)* apply Polish PR-C selected majors (C-M1 + C-M9 + C-M12 + R-M3 + R-M4 + R-M5 + R-M7 + R-M8 + R-M12) ([#105](https://github.com/devrelaicom/tome/pull/105))
- *(doctor)* --fix handlers for Phase 4 subsystems + --force override + orphan cleanup ([#100](https://github.com/devrelaicom/tome/pull/100))
- *(summarise)* trigger wiring + MCP cached-short readout + forward-progress (FR-380/381/382/385/425) ([#95](https://github.com/devrelaicom/tome/pull/95))
- *(harness)* tome harness command surface (bare/list/use/remove/info/sync) ([#91](https://github.com/devrelaicom/tome/pull/91))
- *(settings)* composition validation rules (workspace-ref, bad-exclusion, unsupported) ([#89](https://github.com/devrelaicom/tome/pull/89))
- *(workspace)* sync CLI command (per-workspace + all-workspaces) ([#85](https://github.com/devrelaicom/tome/pull/85))
- *(workspace)* remove with 5-step cascade + refcount-clean catalog caches ([#84](https://github.com/devrelaicom/tome/pull/84))
- *(workspace)* rename + regen-summary + sync helper for bound projects ([#83](https://github.com/devrelaicom/tome/pull/83))
- *(harness)* claude-code production harness + end-to-end bind test ([#78](https://github.com/devrelaicom/tome/pull/78))
- *(harness)* sync algorithm orchestrator + StubHarness end-to-end ([#77](https://github.com/devrelaicom/tome/pull/77))
- *(harness)* mcp_config primitives (JSON + TOML) + idempotence ([#76](https://github.com/devrelaicom/tome/pull/76))
- *(harness)* rules_file primitives + StubHarness fixture ([#75](https://github.com/devrelaicom/tome/pull/75))
- *(migrations)* register phase_4_v1_to_v2; bootstrap emits v2 directly ([#67](https://github.com/devrelaicom/tome/pull/67))
- *(settings)* layered settings parser + composition resolver skeleton ([#66](https://github.com/devrelaicom/tome/pull/66))
- *(harness)* harness skeleton (HarnessModule trait, 5 stub impls, rules_file + mcp_config stubs) ([#65](https://github.com/devrelaicom/tome/pull/65))
- *(summarise)* summariser skeleton (Summariser trait, LlamaBackend singleton, StubSummariser, prompts) ([#64](https://github.com/devrelaicom/tome/pull/64))
- *(catalog)* refuse remove on enabled plugins; cascade with --force ([#32](https://github.com/devrelaicom/tome/pull/32))
- *(status,version)* add tome status; extend --version ([#29](https://github.com/devrelaicom/tome/pull/29))
- *(reindex)* add tome reindex subcommand ([#27](https://github.com/devrelaicom/tome/pull/27))
- *(catalog)* reindex enabled plugins on update; auto-disable orphans ([#26](https://github.com/devrelaicom/tome/pull/26))
- *(reindex)* add reindex_plugin_atomic + lifecycle wrappers ([#25](https://github.com/devrelaicom/tome/pull/25))
- *(plugin)* add interactive catalog/plugin browse flow ([#16](https://github.com/devrelaicom/tome/pull/16))
- *(query)* add tome query with reranker + strict mode ([#13](https://github.com/devrelaicom/tome/pull/13))

### Changed

- *(models)* model manifest json -> toml (US1 cutover) ([#180](https://github.com/devrelaicom/tome/pull/180))
- *(phase-7)* remove dead reference_count; sweep stale doc-comments; strip internal citations from --help (FR-016) ([#158](https://github.com/devrelaicom/tome/pull/158))
- *(phase-7)* move reconcile_hooks; sync.rs is now a thin orchestrator (no behaviour change) ([#144](https://github.com/devrelaicom/tome/pull/144))
- *(phase-7)* move reconcile_guardrails into harness/reconcile/guardrails.rs (no behaviour change) ([#143](https://github.com/devrelaicom/tome/pull/143))
- *(phase-7)* move reconcile_agents into harness/reconcile/agents.rs (no behaviour change) ([#142](https://github.com/devrelaicom/tome/pull/142))
- *(catalog)* rewire onto workspace_catalogs junction; derive metadata from filesystem ([#70](https://github.com/devrelaicom/tome/pull/70))
- *(plugin,index)* thread resolved workspace name through SQL queries ([#69](https://github.com/devrelaicom/tome/pull/69))
- *(workspace)* WorkspaceName newtype + Scope reshape (workspace names, not paths) ([#68](https://github.com/devrelaicom/tome/pull/68))
- *(util)* promote atomic-populated-directory helper to src/util/atomic_dir.rs ([#62](https://github.com/devrelaicom/tome/pull/62))
- *(paths)* collapse XDG-separated paths under <home>/.tome/; drop workspace inventory; read-only DB open across read paths ([#60](https://github.com/devrelaicom/tome/pull/60))
- tidy small code-review findings ([#37](https://github.com/devrelaicom/tome/pull/37))

### Dependencies

- *(deps)* audit serde_json/preserve_order + toml_edit scope ([#63](https://github.com/devrelaicom/tome/pull/63))

### Documentation

- *(readme)* restructure for consumer onboarding + fix stale examples ([#217](https://github.com/devrelaicom/tome/pull/217))
- *(phase-7)* P9 closeout — CHANGELOG v0.6.0 + CLAUDE.md + retro; fix 2 stale test comments ([#166](https://github.com/devrelaicom/tome/pull/166))
- *(review)* phase-7 phase-wide findings + disposition ([#163](https://github.com/devrelaicom/tome/pull/163))
- *(phase-7)* README front door + SECURITY.md + trust-model doc (FR-021/022/010) ([#162](https://github.com/devrelaicom/tome/pull/162))
- *(phase-7)* crate discovery metadata + docs.rs config + CHANGELOG/[Unreleased] reorder (FR-025) ([#160](https://github.com/devrelaicom/tome/pull/160))
- *(phase-7)* amend constitution → v1.4.0 (authorise release tooling) ([#140](https://github.com/devrelaicom/tome/pull/140))
- *(phase-7)* land planning artifacts + beta-readiness audits ([#138](https://github.com/devrelaicom/tome/pull/138))
- *(phase-6)* planning artifacts — spec, plan, research, contracts, tasks ([#130](https://github.com/devrelaicom/tome/pull/130))
- *(codebase,retro,claude-md)* close Phase 4 Polish (PR-G) ([#109](https://github.com/devrelaicom/tome/pull/109))
- *(review)* Phase 4 Polish phase-wide reviewer findings + disposition (PR-A) ([#103](https://github.com/devrelaicom/tome/pull/103))
- *(codebase,retro,claude-md)* refresh after Phase 4 / US5 ([#102](https://github.com/devrelaicom/tome/pull/102))
- *(codebase,retro,claude-md)* refresh after Phase 4 / US4 ([#98](https://github.com/devrelaicom/tome/pull/98))
- *(codebase,retro,claude-md)* refresh after Phase 4 / US3 ([#93](https://github.com/devrelaicom/tome/pull/93))
- *(codebase,retro,claude-md)* refresh after Phase 4 / US2 ([#87](https://github.com/devrelaicom/tome/pull/87))
- *(codebase,retro,claude-md)* refresh after Phase 4 / US1 ([#81](https://github.com/devrelaicom/tome/pull/81))
- *(codebase,retro,claude-md)* refresh after Phase 4 / F1–F11 Foundational ([#73](https://github.com/devrelaicom/tome/pull/73))
- README + CHANGELOG + contract reconciliation + v0.3.0 closeout ([#58](https://github.com/devrelaicom/tome/pull/58))
- Phase 10 retro and task closeout ([#41](https://github.com/devrelaicom/tome/pull/41))
- Phase 2 README, CHANGELOG, CLAUDE.md updates ([#40](https://github.com/devrelaicom/tome/pull/40))
- *(spec)* reconcile Phase 2 contracts with shipped behaviour ([#35](https://github.com/devrelaicom/tome/pull/35))
- Phase 9 codebase refresh + retro ([#33](https://github.com/devrelaicom/tome/pull/33))
- Phase 8 codebase refresh + retro ([#31](https://github.com/devrelaicom/tome/pull/31))
- Phase 7 codebase refresh + retro ([#28](https://github.com/devrelaicom/tome/pull/28))
- Phase 6 codebase refresh + retro ([#24](https://github.com/devrelaicom/tome/pull/24))
- Phase 5 codebase refresh + retro ([#21](https://github.com/devrelaicom/tome/pull/21))
- Phase 4 codebase refresh + retro ([#18](https://github.com/devrelaicom/tome/pull/18))
- codebase refresh + finalise Phase 3 retro ([#15](https://github.com/devrelaicom/tome/pull/15))

### Fixed

- convert-funnel fixes — remote-plugin fetch, detection tie-break, hooks pass-through + papercuts ([#206](https://github.com/devrelaicom/tome/pull/206))
- *(meta)* P9 phase-wide review + docs (Polish) ([#204](https://github.com/devrelaicom/tome/pull/204))
- *(authoring)* P8 phase-wide review — symlink-safe lint, SSOT dedupe, docs, +tests ([#197](https://github.com/devrelaicom/tome/pull/197))
- *(authoring)* US3 closeout — lint catalog source validation + never-halt (P8) ([#195](https://github.com/devrelaicom/tome/pull/195))
- *(authoring)* US2 closeout — emit-sink write containment + reviewer fixes (P8) ([#192](https://github.com/devrelaicom/tome/pull/192))
- *(us1-closeout)* doctor --fix re-download bug, flake fix, coverage + bounded read ([#183](https://github.com/devrelaicom/tome/pull/183))
- *(mcp)* resolve scoped catalog from the DB in get_skill + search_skills (publish-blocker 3/3) ([#171](https://github.com/devrelaicom/tome/pull/171))
- *(plugin,reindex,query)* resolve scoped catalog discovery from the DB (publish-blocker 2/3) ([#170](https://github.com/devrelaicom/tome/pull/170))
- *(plugin)* resolve plugin dir from catalog DB, not the never-written config.toml (publish-blocker, 1/3) ([#168](https://github.com/devrelaicom/tome/pull/168))
- *(models)* bound aux model-file downloads + correct SECURITY.md verification scope (P9 MAJOR-2) ([#165](https://github.com/devrelaicom/tome/pull/165))
- *(harness)* agents cleanup-removal symlink refusal returns its dedicated code 45 (CON-1) ([#164](https://github.com/devrelaicom/tome/pull/164))
- *(phase-7)* off-spec inputs fail closed; config parse → exit 5; duplicate (kind,name) warned + truthfully counted (FR-013/014/015) ([#157](https://github.com/devrelaicom/tome/pull/157))
- *(catalog)* re-derive remove --force cascade inside the lock (F-REMOVE-TOCTOU) ([#155](https://github.com/devrelaicom/tome/pull/155))
- *(harness)* write inline rules body when any sharer needs it so OpenCode receives Tome's rules (F-RULES-OPENCODE) ([#154](https://github.com/devrelaicom/tome/pull/154))
- *(plugin,catalog)* bound every third-party read by its per-class cap (F-PLUGIN-MANIFEST-DOS class) ([#152](https://github.com/devrelaicom/tome/pull/152))
- *(workspace)* emit settings.toml via toml_edit + reject control chars in catalog names (F-WS-TOML-NEWLINE) ([#151](https://github.com/devrelaicom/tome/pull/151))
- *(mcp)* assign prompt names against a single global taken-set (F-MCP-PROMPT-COLLISION) ([#150](https://github.com/devrelaicom/tome/pull/150))
- *(query)* over-fetch+widen so filtered KNN returns min(top_k, matches) (F-KNN) ([#145](https://github.com/devrelaicom/tome/pull/145))
- *(catalog)* key cache dir + refcount by scrubbed URL so SSH sources round-trip (F-CACHE-KEY) ([#149](https://github.com/devrelaicom/tome/pull/149))
- *(models)* re-pin embedder to CPU-compatible INT8 ONNX artefact (F-MODEL-ONNX-CPU) ([#148](https://github.com/devrelaicom/tome/pull/148))
- *(doctor)* open index read-only, degrade not abort on schema mismatch (F-DOCTOR-RW) ([#147](https://github.com/devrelaicom/tome/pull/147))
- *(models)* download all required model files (tokenizer), not just the primary .onnx (F-MODEL-FILES) ([#146](https://github.com/devrelaicom/tome/pull/146))
- *(phase-5/us1d)* reviewer pass closeout — 1 blocker + 8 majors + docs refresh ([#115](https://github.com/devrelaicom/tome/pull/115))
- *(phase-4)* apply Polish PR-E security hardening (S-M1 + S-M2 + S-M6 + S-M7 + T-M8 + T416 + T419) ([#107](https://github.com/devrelaicom/tome/pull/107))
- *(phase-4)* apply Phase 4 Polish blockers C-B1 + C-B2 + C-B3 (PR-B) ([#104](https://github.com/devrelaicom/tome/pull/104))
- *(doctor)* US5 reviewer-flagged fixups (1 blocker + 10 majors) ([#101](https://github.com/devrelaicom/tome/pull/101))
- *(summarise)* US4 reviewer-flagged fixups (4 blockers + 9 majors) ([#97](https://github.com/devrelaicom/tome/pull/97))
- *(harness,settings)* US3 reviewer-flagged fixups ([#92](https://github.com/devrelaicom/tome/pull/92))
- *(workspace,catalog)* US2 reviewer-flagged fixups ([#86](https://github.com/devrelaicom/tome/pull/86))
- *(workspace,harness)* US1 reviewer-flagged fixups ([#80](https://github.com/devrelaicom/tome/pull/80))
- *(security)* mcp.log 0600, symlink rejection, registry validation, init refusal ([#56](https://github.com/devrelaicom/tome/pull/56))
- *(doctor)* orphan clones, workspace registry status, schema fix, signature ([#55](https://github.com/devrelaicom/tome/pull/55))
- *(mcp)* signal handling, log scrubbing, log taxonomy, specific-over-generic ([#54](https://github.com/devrelaicom/tome/pull/54))
- *(workspace)* enforce §Validation 1b/1c in resolver; add doctor drift tests ([#53](https://github.com/devrelaicom/tome/pull/53))
- *(mcp/log)* emit contract-pinned field names (ts/level/target/msg) ([#52](https://github.com/devrelaicom/tome/pull/52))
- *(security)* scrub catalog URL on add, chmod config 0600, ignore harness state ([#36](https://github.com/devrelaicom/tome/pull/36))
- *(catalog)* report real per-plugin skills_dropped in cascade ([#34](https://github.com/devrelaicom/tome/pull/34))

### Other

- Phase 6 Polish + v0.6.0 release ([#137](https://github.com/devrelaicom/tome/pull/137))
- Phase 5 Polish + v0.5.0 release ([#128](https://github.com/devrelaicom/tome/pull/128))
- doctor Phase 4 subsystems + Subsystem enum promotion + detected-uninstalled ([#99](https://github.com/devrelaicom/tome/pull/99))
- production LlamaSummariser + Qwen2.5 + tome models extension ([#94](https://github.com/devrelaicom/tome/pull/94))
- cross-harness module tests for all 5 production harnesses ([#90](https://github.com/devrelaicom/tome/pull/90))
- composition resolver verification + comprehensive tests ([#88](https://github.com/devrelaicom/tome/pull/88))
- workspace init + list + info Phase 4 fields ([#82](https://github.com/devrelaicom/tome/pull/82))
- tome workspace use <name> — core binding flow ([#74](https://github.com/devrelaicom/tome/pull/74))
- pre-allocate 8 new TomeError variants (codes 13–19, 24) ([#61](https://github.com/devrelaicom/tome/pull/61))
- Phase 4 setup + F1: deps + constitution v1.3.0 amendment ([#59](https://github.com/devrelaicom/tome/pull/59))
- Phase 3 Polish PR-A: review findings + disposition ([#51](https://github.com/devrelaicom/tome/pull/51))
- Phase 3 / US5: forward schema migrations ([#50](https://github.com/devrelaicom/tome/pull/50))
- Phase 3 / US4: tome doctor + --fix repairs ([#49](https://github.com/devrelaicom/tome/pull/49))
- Phase 3 / US3: per-command scope honouring + reference-counted catalog cleanup ([#48](https://github.com/devrelaicom/tome/pull/48))
- Phase 3 / US2: tome workspace info + init ([#47](https://github.com/devrelaicom/tome/pull/47))
- Phase 3 / US1: tome mcp — stdio MCP server with search_skills + get_skill ([#46](https://github.com/devrelaicom/tome/pull/46))
- Phase 3 Foundational F7+F8: schema-migration framework + MCP scaffolding ([#45](https://github.com/devrelaicom/tome/pull/45))
- Phase 3 Foundational (F1-F6): Scope + per-scope Paths + resolution + read-only DB + query::run_with_deps ([#44](https://github.com/devrelaicom/tome/pull/44))
- Phase 3 Setup: plan artefacts + rmcp/tokio dependencies ([#43](https://github.com/devrelaicom/tome/pull/43))
- Phase 6 US4 slice 2: integration tests for models download / list / remove ([#23](https://github.com/devrelaicom/tome/pull/23))
- Phase 6 US4 slice 1: tome models download / list / remove ([#22](https://github.com/devrelaicom/tome/pull/22))
- Phase 5 US3 slice 2: integration tests for disable, cheap re-enable, repeated state ([#20](https://github.com/devrelaicom/tome/pull/20))
- Phase 5 US3 slice 1: tome plugin disable + cheap re-enable verification ([#19](https://github.com/devrelaicom/tome/pull/19))
- Phase 3 US1 slice 3: integration tests for plugin enable/list/show/query + atomicity ([#14](https://github.com/devrelaicom/tome/pull/14))
- Phase 3 US1 slice 1b: tome plugin enable / list / show CLI surface ([#12](https://github.com/devrelaicom/tome/pull/12))
- Phase 3 US1 slice 1a: lifecycle::enable / disable + pinned MODEL_REGISTRY ([#11](https://github.com/devrelaicom/tome/pull/11))
- Phase 2 foundational (7/N): model-download integration test + docs + retro ([#10](https://github.com/devrelaicom/tome/pull/10))
- Phase 2 foundational (6/N): extend credential scrubber to model download ([#9](https://github.com/devrelaicom/tome/pull/9))
- Phase 2 foundational (5/N): embedding pipeline core ([#8](https://github.com/devrelaicom/tome/pull/8))
- Phase 2 foundational (4b/N): index features — lock, meta, integrity, skills, query ([#7](https://github.com/devrelaicom/tome/pull/7))
- Phase 2 foundational (4a/N): index bootstrap pipeline ([#6](https://github.com/devrelaicom/tome/pull/6))
- Phase 2 foundational (3/N): plugin metadata parsers ([#5](https://github.com/devrelaicom/tome/pull/5))
- Phase 2 foundational (2/N): presentation primitives + git-hooks migration ([#4](https://github.com/devrelaicom/tome/pull/4))
- Phase 2 foundational (1/N): error surface + paths ([#3](https://github.com/devrelaicom/tome/pull/3))
- Phase 2: setup — deps, vendored sqlite-vec, build infra ([#2](https://github.com/devrelaicom/tome/pull/2))
- Phase 1: Project Foundations and Catalog Management ([#1](https://github.com/devrelaicom/tome/pull/1))

### Chore

- *(phase-7)* [**breaking**] rename crate tome→tome-mcp, keep [[bin]] name = tome (FR-017) ([#159](https://github.com/devrelaicom/tome/pull/159))

## [0.6.0](https://github.com/devrelaicom/tome/releases/tag/v0.6.0) - 2026-06-09

### Added

- *(meta)* US4 — doctor meta-skill drift report + --fix re-install ([#203](https://github.com/devrelaicom/tome/pull/203))
- *(meta)* US3 — MCP meta tool + reserved prompt + host-harness stamping ([#201](https://github.com/devrelaicom/tome/pull/201))
- *(meta)* author convert-marketplace guided skill + unsupported-component rubric ([#200](https://github.com/devrelaicom/tome/pull/200))
- *(meta)* tome meta {list,add,remove} CLI over the shared install path ([#199](https://github.com/devrelaicom/tome/pull/199))
- *(meta)* Phase 9 foundational — embed pipeline + shared compute + harness skill-emit trait ([#198](https://github.com/devrelaicom/tome/pull/198))
- *(authoring)* create + built-in templates (P8 US4-1) ([#196](https://github.com/devrelaicom/tome/pull/196))
- *(authoring)* lint --autofix + lint command surface (P8 US3) ([#194](https://github.com/devrelaicom/tome/pull/194))
- *(authoring)* lint rules + native-artifact parser (P8 US3) ([#193](https://github.com/devrelaicom/tome/pull/193))
- *(authoring)* convert --into injection (P8 US2) ([#191](https://github.com/devrelaicom/tome/pull/191))
- *(authoring)* remote SOURCE fetch with guaranteed cleanup (P8 US2) ([#190](https://github.com/devrelaicom/tome/pull/190))
- *(authoring)* CC marketplace → catalog convert + detect fix (P8 US2) ([#189](https://github.com/devrelaicom/tome/pull/189))
- *(authoring)* Codex project importer (Tier 2 synthesis) (P8 US2) ([#188](https://github.com/devrelaicom/tome/pull/188))
- *(authoring)* native SKILL.md convert + tome skill convert (P8 US2) ([#187](https://github.com/devrelaicom/tome/pull/187))
- *(authoring)* convert pipeline + `tome plugin convert` (P8 US2) ([#186](https://github.com/devrelaicom/tome/pull/186))
- *(authoring)* Claude Code → IR plugin importer (P8 US2) ([#185](https://github.com/devrelaicom/tome/pull/185))
- *(authoring)* untrusted-read guard + source-format detection (P8 US2) ([#184](https://github.com/devrelaicom/tome/pull/184))
- *(doctor)* migrate legacy model manifest + report unconverted plugins (US1) ([#182](https://github.com/devrelaicom/tome/pull/182))
- *(substitution)* add ${TOME_PROJECT_DIR} builtin (US1) ([#181](https://github.com/devrelaicom/tome/pull/181))
- *(plugin)* read native tome-plugin.toml; PluginNotConverted on legacy (US1 cutover) ([#179](https://github.com/devrelaicom/tome/pull/179))
- *(authoring)* harness-ism rewriter and lint runner framework (P8 foundational B) ([#178](https://github.com/devrelaicom/tome/pull/178))
- *(authoring)* TomePluginManifest, artifact IR, and the emitter ([#177](https://github.com/devrelaicom/tome/pull/177))
- *(harness)* symlink-safe write primitive across all sinks (FR-007; intermediate-component guard) ([#153](https://github.com/devrelaicom/tome/pull/153))
- *(phase-6)* US5 — privilege governance + doctor extensions ([#136](https://github.com/devrelaicom/tome/pull/136))
- *(phase-6)* US4 — agent personas via MCP prompts ([#135](https://github.com/devrelaicom/tome/pull/135))
- *(phase-6)* US3 — guardrails fallback + Claude Code rules-file correction ([#134](https://github.com/devrelaicom/tome/pull/134))
- *(phase-6)* US2 — real Claude Code hooks ([#133](https://github.com/devrelaicom/tome/pull/133))
- *(phase-6)* US1 — native agents across four harnesses ([#132](https://github.com/devrelaicom/tome/pull/132))
- *(phase-6)* Foundational — error codes 43-46, EntryKind::Agent, HarnessModule trait ([#131](https://github.com/devrelaicom/tome/pull/131))
- *(phase-5/us5c)* reviewer pass + Phase 7 closeout ([#127](https://github.com/devrelaicom/tome/pull/127))
- *(phase-5/us5b)* plugin show + doctor — Phase 5 surfaces ([#125](https://github.com/devrelaicom/tome/pull/125))
- *(phase-5/us4c+d)* search_skills extensions + reviewer pass (HIGH DoS fix + closeout) ([#123](https://github.com/devrelaicom/tome/pull/123))
- *(index)* when_to_use contributes to embedding_text (US4.b verification) ([#122](https://github.com/devrelaicom/tome/pull/122))
- *(phase-5/us4a)* get_skill_info middle-tier MCP tool + resource enumeration ([#121](https://github.com/devrelaicom/tome/pull/121))
- *(phase-5/us3c+d)* substitution end-to-end + reviewer pass (0 BLOCKERS; substitution layer COMPLETE) ([#120](https://github.com/devrelaicom/tome/pull/120))
- *(phase-5/us3a+b)* argument substitution Stage 3 + ARGUMENTS append-fallback Stage 4 ([#119](https://github.com/devrelaicom/tome/pull/119))
- *(phase-5/us2c+d)* substitution end-to-end + reviewer pass (2 BLOCKERS fixed; data exfiltration vector closed) ([#118](https://github.com/devrelaicom/tome/pull/118))
- *(phase-5/us2b)* substitution env passthrough + data-dir + rename relocation ([#117](https://github.com/devrelaicom/tome/pull/117))
- *(phase-5/us2a)* substitution built-ins stage + clock injection + path sanitisation ([#116](https://github.com/devrelaicom/tome/pull/116))
- *(phase-5/us1c)* prompts/get + substitution wiring + shared path resolver ([#114](https://github.com/devrelaicom/tome/pull/114))
- *(phase-5/us1b)* MCP prompts capability — prompts/list + name derivation + collisions ([#113](https://github.com/devrelaicom/tome/pull/113))
- *(phase-5/us1a)* schema v3 + frontmatter widening + commands walk + EntryKind ([#112](https://github.com/devrelaicom/tome/pull/112))
- *(phase-5)* foundational F1+F2+F3 — error variants, regex, substitution skeleton ([#111](https://github.com/devrelaicom/tome/pull/111))
- *(phase-4)* apply Polish PR-C selected majors (C-M1 + C-M9 + C-M12 + R-M3 + R-M4 + R-M5 + R-M7 + R-M8 + R-M12) ([#105](https://github.com/devrelaicom/tome/pull/105))
- *(doctor)* --fix handlers for Phase 4 subsystems + --force override + orphan cleanup ([#100](https://github.com/devrelaicom/tome/pull/100))
- *(summarise)* trigger wiring + MCP cached-short readout + forward-progress (FR-380/381/382/385/425) ([#95](https://github.com/devrelaicom/tome/pull/95))
- *(harness)* tome harness command surface (bare/list/use/remove/info/sync) ([#91](https://github.com/devrelaicom/tome/pull/91))
- *(settings)* composition validation rules (workspace-ref, bad-exclusion, unsupported) ([#89](https://github.com/devrelaicom/tome/pull/89))
- *(workspace)* sync CLI command (per-workspace + all-workspaces) ([#85](https://github.com/devrelaicom/tome/pull/85))
- *(workspace)* remove with 5-step cascade + refcount-clean catalog caches ([#84](https://github.com/devrelaicom/tome/pull/84))
- *(workspace)* rename + regen-summary + sync helper for bound projects ([#83](https://github.com/devrelaicom/tome/pull/83))
- *(harness)* claude-code production harness + end-to-end bind test ([#78](https://github.com/devrelaicom/tome/pull/78))
- *(harness)* sync algorithm orchestrator + StubHarness end-to-end ([#77](https://github.com/devrelaicom/tome/pull/77))
- *(harness)* mcp_config primitives (JSON + TOML) + idempotence ([#76](https://github.com/devrelaicom/tome/pull/76))
- *(harness)* rules_file primitives + StubHarness fixture ([#75](https://github.com/devrelaicom/tome/pull/75))
- *(migrations)* register phase_4_v1_to_v2; bootstrap emits v2 directly ([#67](https://github.com/devrelaicom/tome/pull/67))
- *(settings)* layered settings parser + composition resolver skeleton ([#66](https://github.com/devrelaicom/tome/pull/66))
- *(harness)* harness skeleton (HarnessModule trait, 5 stub impls, rules_file + mcp_config stubs) ([#65](https://github.com/devrelaicom/tome/pull/65))
- *(summarise)* summariser skeleton (Summariser trait, LlamaBackend singleton, StubSummariser, prompts) ([#64](https://github.com/devrelaicom/tome/pull/64))
- *(catalog)* refuse remove on enabled plugins; cascade with --force ([#32](https://github.com/devrelaicom/tome/pull/32))
- *(status,version)* add tome status; extend --version ([#29](https://github.com/devrelaicom/tome/pull/29))
- *(reindex)* add tome reindex subcommand ([#27](https://github.com/devrelaicom/tome/pull/27))
- *(catalog)* reindex enabled plugins on update; auto-disable orphans ([#26](https://github.com/devrelaicom/tome/pull/26))
- *(reindex)* add reindex_plugin_atomic + lifecycle wrappers ([#25](https://github.com/devrelaicom/tome/pull/25))
- *(plugin)* add interactive catalog/plugin browse flow ([#16](https://github.com/devrelaicom/tome/pull/16))
- *(query)* add tome query with reranker + strict mode ([#13](https://github.com/devrelaicom/tome/pull/13))

### Changed

- *(models)* model manifest json -> toml (US1 cutover) ([#180](https://github.com/devrelaicom/tome/pull/180))
- *(phase-7)* remove dead reference_count; sweep stale doc-comments; strip internal citations from --help (FR-016) ([#158](https://github.com/devrelaicom/tome/pull/158))
- *(phase-7)* move reconcile_hooks; sync.rs is now a thin orchestrator (no behaviour change) ([#144](https://github.com/devrelaicom/tome/pull/144))
- *(phase-7)* move reconcile_guardrails into harness/reconcile/guardrails.rs (no behaviour change) ([#143](https://github.com/devrelaicom/tome/pull/143))
- *(phase-7)* move reconcile_agents into harness/reconcile/agents.rs (no behaviour change) ([#142](https://github.com/devrelaicom/tome/pull/142))
- *(catalog)* rewire onto workspace_catalogs junction; derive metadata from filesystem ([#70](https://github.com/devrelaicom/tome/pull/70))
- *(plugin,index)* thread resolved workspace name through SQL queries ([#69](https://github.com/devrelaicom/tome/pull/69))
- *(workspace)* WorkspaceName newtype + Scope reshape (workspace names, not paths) ([#68](https://github.com/devrelaicom/tome/pull/68))
- *(util)* promote atomic-populated-directory helper to src/util/atomic_dir.rs ([#62](https://github.com/devrelaicom/tome/pull/62))
- *(paths)* collapse XDG-separated paths under <home>/.tome/; drop workspace inventory; read-only DB open across read paths ([#60](https://github.com/devrelaicom/tome/pull/60))
- tidy small code-review findings ([#37](https://github.com/devrelaicom/tome/pull/37))

### Dependencies

- *(deps)* audit serde_json/preserve_order + toml_edit scope ([#63](https://github.com/devrelaicom/tome/pull/63))

### Documentation

- *(phase-7)* P9 closeout — CHANGELOG v0.6.0 + CLAUDE.md + retro; fix 2 stale test comments ([#166](https://github.com/devrelaicom/tome/pull/166))
- *(review)* phase-7 phase-wide findings + disposition ([#163](https://github.com/devrelaicom/tome/pull/163))
- *(phase-7)* README front door + SECURITY.md + trust-model doc (FR-021/022/010) ([#162](https://github.com/devrelaicom/tome/pull/162))
- *(phase-7)* crate discovery metadata + docs.rs config + CHANGELOG/[Unreleased] reorder (FR-025) ([#160](https://github.com/devrelaicom/tome/pull/160))
- *(phase-7)* amend constitution → v1.4.0 (authorise release tooling) ([#140](https://github.com/devrelaicom/tome/pull/140))
- *(phase-7)* land planning artifacts + beta-readiness audits ([#138](https://github.com/devrelaicom/tome/pull/138))
- *(phase-6)* planning artifacts — spec, plan, research, contracts, tasks ([#130](https://github.com/devrelaicom/tome/pull/130))
- *(codebase,retro,claude-md)* close Phase 4 Polish (PR-G) ([#109](https://github.com/devrelaicom/tome/pull/109))
- *(review)* Phase 4 Polish phase-wide reviewer findings + disposition (PR-A) ([#103](https://github.com/devrelaicom/tome/pull/103))
- *(codebase,retro,claude-md)* refresh after Phase 4 / US5 ([#102](https://github.com/devrelaicom/tome/pull/102))
- *(codebase,retro,claude-md)* refresh after Phase 4 / US4 ([#98](https://github.com/devrelaicom/tome/pull/98))
- *(codebase,retro,claude-md)* refresh after Phase 4 / US3 ([#93](https://github.com/devrelaicom/tome/pull/93))
- *(codebase,retro,claude-md)* refresh after Phase 4 / US2 ([#87](https://github.com/devrelaicom/tome/pull/87))
- *(codebase,retro,claude-md)* refresh after Phase 4 / US1 ([#81](https://github.com/devrelaicom/tome/pull/81))
- *(codebase,retro,claude-md)* refresh after Phase 4 / F1–F11 Foundational ([#73](https://github.com/devrelaicom/tome/pull/73))
- README + CHANGELOG + contract reconciliation + v0.3.0 closeout ([#58](https://github.com/devrelaicom/tome/pull/58))
- Phase 10 retro and task closeout ([#41](https://github.com/devrelaicom/tome/pull/41))
- Phase 2 README, CHANGELOG, CLAUDE.md updates ([#40](https://github.com/devrelaicom/tome/pull/40))
- *(spec)* reconcile Phase 2 contracts with shipped behaviour ([#35](https://github.com/devrelaicom/tome/pull/35))
- Phase 9 codebase refresh + retro ([#33](https://github.com/devrelaicom/tome/pull/33))
- Phase 8 codebase refresh + retro ([#31](https://github.com/devrelaicom/tome/pull/31))
- Phase 7 codebase refresh + retro ([#28](https://github.com/devrelaicom/tome/pull/28))
- Phase 6 codebase refresh + retro ([#24](https://github.com/devrelaicom/tome/pull/24))
- Phase 5 codebase refresh + retro ([#21](https://github.com/devrelaicom/tome/pull/21))
- Phase 4 codebase refresh + retro ([#18](https://github.com/devrelaicom/tome/pull/18))
- codebase refresh + finalise Phase 3 retro ([#15](https://github.com/devrelaicom/tome/pull/15))

### Fixed

- *(meta)* P9 phase-wide review + docs (Polish) ([#204](https://github.com/devrelaicom/tome/pull/204))
- *(authoring)* P8 phase-wide review — symlink-safe lint, SSOT dedupe, docs, +tests ([#197](https://github.com/devrelaicom/tome/pull/197))
- *(authoring)* US3 closeout — lint catalog source validation + never-halt (P8) ([#195](https://github.com/devrelaicom/tome/pull/195))
- *(authoring)* US2 closeout — emit-sink write containment + reviewer fixes (P8) ([#192](https://github.com/devrelaicom/tome/pull/192))
- *(us1-closeout)* doctor --fix re-download bug, flake fix, coverage + bounded read ([#183](https://github.com/devrelaicom/tome/pull/183))
- *(mcp)* resolve scoped catalog from the DB in get_skill + search_skills (publish-blocker 3/3) ([#171](https://github.com/devrelaicom/tome/pull/171))
- *(plugin,reindex,query)* resolve scoped catalog discovery from the DB (publish-blocker 2/3) ([#170](https://github.com/devrelaicom/tome/pull/170))
- *(plugin)* resolve plugin dir from catalog DB, not the never-written config.toml (publish-blocker, 1/3) ([#168](https://github.com/devrelaicom/tome/pull/168))
- *(models)* bound aux model-file downloads + correct SECURITY.md verification scope (P9 MAJOR-2) ([#165](https://github.com/devrelaicom/tome/pull/165))
- *(harness)* agents cleanup-removal symlink refusal returns its dedicated code 45 (CON-1) ([#164](https://github.com/devrelaicom/tome/pull/164))
- *(phase-7)* off-spec inputs fail closed; config parse → exit 5; duplicate (kind,name) warned + truthfully counted (FR-013/014/015) ([#157](https://github.com/devrelaicom/tome/pull/157))
- *(catalog)* re-derive remove --force cascade inside the lock (F-REMOVE-TOCTOU) ([#155](https://github.com/devrelaicom/tome/pull/155))
- *(harness)* write inline rules body when any sharer needs it so OpenCode receives Tome's rules (F-RULES-OPENCODE) ([#154](https://github.com/devrelaicom/tome/pull/154))
- *(plugin,catalog)* bound every third-party read by its per-class cap (F-PLUGIN-MANIFEST-DOS class) ([#152](https://github.com/devrelaicom/tome/pull/152))
- *(workspace)* emit settings.toml via toml_edit + reject control chars in catalog names (F-WS-TOML-NEWLINE) ([#151](https://github.com/devrelaicom/tome/pull/151))
- *(mcp)* assign prompt names against a single global taken-set (F-MCP-PROMPT-COLLISION) ([#150](https://github.com/devrelaicom/tome/pull/150))
- *(query)* over-fetch+widen so filtered KNN returns min(top_k, matches) (F-KNN) ([#145](https://github.com/devrelaicom/tome/pull/145))
- *(catalog)* key cache dir + refcount by scrubbed URL so SSH sources round-trip (F-CACHE-KEY) ([#149](https://github.com/devrelaicom/tome/pull/149))
- *(models)* re-pin embedder to CPU-compatible INT8 ONNX artefact (F-MODEL-ONNX-CPU) ([#148](https://github.com/devrelaicom/tome/pull/148))
- *(doctor)* open index read-only, degrade not abort on schema mismatch (F-DOCTOR-RW) ([#147](https://github.com/devrelaicom/tome/pull/147))
- *(models)* download all required model files (tokenizer), not just the primary .onnx (F-MODEL-FILES) ([#146](https://github.com/devrelaicom/tome/pull/146))
- *(phase-5/us1d)* reviewer pass closeout — 1 blocker + 8 majors + docs refresh ([#115](https://github.com/devrelaicom/tome/pull/115))
- *(phase-4)* apply Polish PR-E security hardening (S-M1 + S-M2 + S-M6 + S-M7 + T-M8 + T416 + T419) ([#107](https://github.com/devrelaicom/tome/pull/107))
- *(phase-4)* apply Phase 4 Polish blockers C-B1 + C-B2 + C-B3 (PR-B) ([#104](https://github.com/devrelaicom/tome/pull/104))
- *(doctor)* US5 reviewer-flagged fixups (1 blocker + 10 majors) ([#101](https://github.com/devrelaicom/tome/pull/101))
- *(summarise)* US4 reviewer-flagged fixups (4 blockers + 9 majors) ([#97](https://github.com/devrelaicom/tome/pull/97))
- *(harness,settings)* US3 reviewer-flagged fixups ([#92](https://github.com/devrelaicom/tome/pull/92))
- *(workspace,catalog)* US2 reviewer-flagged fixups ([#86](https://github.com/devrelaicom/tome/pull/86))
- *(workspace,harness)* US1 reviewer-flagged fixups ([#80](https://github.com/devrelaicom/tome/pull/80))
- *(security)* mcp.log 0600, symlink rejection, registry validation, init refusal ([#56](https://github.com/devrelaicom/tome/pull/56))
- *(doctor)* orphan clones, workspace registry status, schema fix, signature ([#55](https://github.com/devrelaicom/tome/pull/55))
- *(mcp)* signal handling, log scrubbing, log taxonomy, specific-over-generic ([#54](https://github.com/devrelaicom/tome/pull/54))
- *(workspace)* enforce §Validation 1b/1c in resolver; add doctor drift tests ([#53](https://github.com/devrelaicom/tome/pull/53))
- *(mcp/log)* emit contract-pinned field names (ts/level/target/msg) ([#52](https://github.com/devrelaicom/tome/pull/52))
- *(security)* scrub catalog URL on add, chmod config 0600, ignore harness state ([#36](https://github.com/devrelaicom/tome/pull/36))
- *(catalog)* report real per-plugin skills_dropped in cascade ([#34](https://github.com/devrelaicom/tome/pull/34))

### Other

- Phase 6 Polish + v0.6.0 release ([#137](https://github.com/devrelaicom/tome/pull/137))
- Phase 5 Polish + v0.5.0 release ([#128](https://github.com/devrelaicom/tome/pull/128))
- doctor Phase 4 subsystems + Subsystem enum promotion + detected-uninstalled ([#99](https://github.com/devrelaicom/tome/pull/99))
- production LlamaSummariser + Qwen2.5 + tome models extension ([#94](https://github.com/devrelaicom/tome/pull/94))
- cross-harness module tests for all 5 production harnesses ([#90](https://github.com/devrelaicom/tome/pull/90))
- composition resolver verification + comprehensive tests ([#88](https://github.com/devrelaicom/tome/pull/88))
- workspace init + list + info Phase 4 fields ([#82](https://github.com/devrelaicom/tome/pull/82))
- tome workspace use <name> — core binding flow ([#74](https://github.com/devrelaicom/tome/pull/74))
- pre-allocate 8 new TomeError variants (codes 13–19, 24) ([#61](https://github.com/devrelaicom/tome/pull/61))
- Phase 4 setup + F1: deps + constitution v1.3.0 amendment ([#59](https://github.com/devrelaicom/tome/pull/59))
- Phase 3 Polish PR-A: review findings + disposition ([#51](https://github.com/devrelaicom/tome/pull/51))
- Phase 3 / US5: forward schema migrations ([#50](https://github.com/devrelaicom/tome/pull/50))
- Phase 3 / US4: tome doctor + --fix repairs ([#49](https://github.com/devrelaicom/tome/pull/49))
- Phase 3 / US3: per-command scope honouring + reference-counted catalog cleanup ([#48](https://github.com/devrelaicom/tome/pull/48))
- Phase 3 / US2: tome workspace info + init ([#47](https://github.com/devrelaicom/tome/pull/47))
- Phase 3 / US1: tome mcp — stdio MCP server with search_skills + get_skill ([#46](https://github.com/devrelaicom/tome/pull/46))
- Phase 3 Foundational F7+F8: schema-migration framework + MCP scaffolding ([#45](https://github.com/devrelaicom/tome/pull/45))
- Phase 3 Foundational (F1-F6): Scope + per-scope Paths + resolution + read-only DB + query::run_with_deps ([#44](https://github.com/devrelaicom/tome/pull/44))
- Phase 3 Setup: plan artefacts + rmcp/tokio dependencies ([#43](https://github.com/devrelaicom/tome/pull/43))
- Phase 6 US4 slice 2: integration tests for models download / list / remove ([#23](https://github.com/devrelaicom/tome/pull/23))
- Phase 6 US4 slice 1: tome models download / list / remove ([#22](https://github.com/devrelaicom/tome/pull/22))
- Phase 5 US3 slice 2: integration tests for disable, cheap re-enable, repeated state ([#20](https://github.com/devrelaicom/tome/pull/20))
- Phase 5 US3 slice 1: tome plugin disable + cheap re-enable verification ([#19](https://github.com/devrelaicom/tome/pull/19))
- Phase 3 US1 slice 3: integration tests for plugin enable/list/show/query + atomicity ([#14](https://github.com/devrelaicom/tome/pull/14))
- Phase 3 US1 slice 1b: tome plugin enable / list / show CLI surface ([#12](https://github.com/devrelaicom/tome/pull/12))
- Phase 3 US1 slice 1a: lifecycle::enable / disable + pinned MODEL_REGISTRY ([#11](https://github.com/devrelaicom/tome/pull/11))
- Phase 2 foundational (7/N): model-download integration test + docs + retro ([#10](https://github.com/devrelaicom/tome/pull/10))
- Phase 2 foundational (6/N): extend credential scrubber to model download ([#9](https://github.com/devrelaicom/tome/pull/9))
- Phase 2 foundational (5/N): embedding pipeline core ([#8](https://github.com/devrelaicom/tome/pull/8))
- Phase 2 foundational (4b/N): index features — lock, meta, integrity, skills, query ([#7](https://github.com/devrelaicom/tome/pull/7))
- Phase 2 foundational (4a/N): index bootstrap pipeline ([#6](https://github.com/devrelaicom/tome/pull/6))
- Phase 2 foundational (3/N): plugin metadata parsers ([#5](https://github.com/devrelaicom/tome/pull/5))
- Phase 2 foundational (2/N): presentation primitives + git-hooks migration ([#4](https://github.com/devrelaicom/tome/pull/4))
- Phase 2 foundational (1/N): error surface + paths ([#3](https://github.com/devrelaicom/tome/pull/3))
- Phase 2: setup — deps, vendored sqlite-vec, build infra ([#2](https://github.com/devrelaicom/tome/pull/2))
- Phase 1: Project Foundations and Catalog Management ([#1](https://github.com/devrelaicom/tome/pull/1))

### Chore

- *(phase-7)* [**breaking**] rename crate tome→tome-mcp, keep [[bin]] name = tome (FR-017) ([#159](https://github.com/devrelaicom/tome/pull/159))

## [0.6.0](https://github.com/devrelaicom/tome/releases/tag/v0.6.0) - 2026-06-04

### Added

- *(harness)* symlink-safe write primitive across all sinks (FR-007; intermediate-component guard) ([#153](https://github.com/devrelaicom/tome/pull/153))
- *(phase-6)* US5 — privilege governance + doctor extensions ([#136](https://github.com/devrelaicom/tome/pull/136))
- *(phase-6)* US4 — agent personas via MCP prompts ([#135](https://github.com/devrelaicom/tome/pull/135))
- *(phase-6)* US3 — guardrails fallback + Claude Code rules-file correction ([#134](https://github.com/devrelaicom/tome/pull/134))
- *(phase-6)* US2 — real Claude Code hooks ([#133](https://github.com/devrelaicom/tome/pull/133))
- *(phase-6)* US1 — native agents across four harnesses ([#132](https://github.com/devrelaicom/tome/pull/132))
- *(phase-6)* Foundational — error codes 43-46, EntryKind::Agent, HarnessModule trait ([#131](https://github.com/devrelaicom/tome/pull/131))
- *(phase-5/us5c)* reviewer pass + Phase 7 closeout ([#127](https://github.com/devrelaicom/tome/pull/127))
- *(phase-5/us5b)* plugin show + doctor — Phase 5 surfaces ([#125](https://github.com/devrelaicom/tome/pull/125))
- *(phase-5/us4c+d)* search_skills extensions + reviewer pass (HIGH DoS fix + closeout) ([#123](https://github.com/devrelaicom/tome/pull/123))
- *(index)* when_to_use contributes to embedding_text (US4.b verification) ([#122](https://github.com/devrelaicom/tome/pull/122))
- *(phase-5/us4a)* get_skill_info middle-tier MCP tool + resource enumeration ([#121](https://github.com/devrelaicom/tome/pull/121))
- *(phase-5/us3c+d)* substitution end-to-end + reviewer pass (0 BLOCKERS; substitution layer COMPLETE) ([#120](https://github.com/devrelaicom/tome/pull/120))
- *(phase-5/us3a+b)* argument substitution Stage 3 + ARGUMENTS append-fallback Stage 4 ([#119](https://github.com/devrelaicom/tome/pull/119))
- *(phase-5/us2c+d)* substitution end-to-end + reviewer pass (2 BLOCKERS fixed; data exfiltration vector closed) ([#118](https://github.com/devrelaicom/tome/pull/118))
- *(phase-5/us2b)* substitution env passthrough + data-dir + rename relocation ([#117](https://github.com/devrelaicom/tome/pull/117))
- *(phase-5/us2a)* substitution built-ins stage + clock injection + path sanitisation ([#116](https://github.com/devrelaicom/tome/pull/116))
- *(phase-5/us1c)* prompts/get + substitution wiring + shared path resolver ([#114](https://github.com/devrelaicom/tome/pull/114))
- *(phase-5/us1b)* MCP prompts capability — prompts/list + name derivation + collisions ([#113](https://github.com/devrelaicom/tome/pull/113))
- *(phase-5/us1a)* schema v3 + frontmatter widening + commands walk + EntryKind ([#112](https://github.com/devrelaicom/tome/pull/112))
- *(phase-5)* foundational F1+F2+F3 — error variants, regex, substitution skeleton ([#111](https://github.com/devrelaicom/tome/pull/111))
- *(phase-4)* apply Polish PR-C selected majors (C-M1 + C-M9 + C-M12 + R-M3 + R-M4 + R-M5 + R-M7 + R-M8 + R-M12) ([#105](https://github.com/devrelaicom/tome/pull/105))
- *(doctor)* --fix handlers for Phase 4 subsystems + --force override + orphan cleanup ([#100](https://github.com/devrelaicom/tome/pull/100))
- *(summarise)* trigger wiring + MCP cached-short readout + forward-progress (FR-380/381/382/385/425) ([#95](https://github.com/devrelaicom/tome/pull/95))
- *(harness)* tome harness command surface (bare/list/use/remove/info/sync) ([#91](https://github.com/devrelaicom/tome/pull/91))
- *(settings)* composition validation rules (workspace-ref, bad-exclusion, unsupported) ([#89](https://github.com/devrelaicom/tome/pull/89))
- *(workspace)* sync CLI command (per-workspace + all-workspaces) ([#85](https://github.com/devrelaicom/tome/pull/85))
- *(workspace)* remove with 5-step cascade + refcount-clean catalog caches ([#84](https://github.com/devrelaicom/tome/pull/84))
- *(workspace)* rename + regen-summary + sync helper for bound projects ([#83](https://github.com/devrelaicom/tome/pull/83))
- *(harness)* claude-code production harness + end-to-end bind test ([#78](https://github.com/devrelaicom/tome/pull/78))
- *(harness)* sync algorithm orchestrator + StubHarness end-to-end ([#77](https://github.com/devrelaicom/tome/pull/77))
- *(harness)* mcp_config primitives (JSON + TOML) + idempotence ([#76](https://github.com/devrelaicom/tome/pull/76))
- *(harness)* rules_file primitives + StubHarness fixture ([#75](https://github.com/devrelaicom/tome/pull/75))
- *(migrations)* register phase_4_v1_to_v2; bootstrap emits v2 directly ([#67](https://github.com/devrelaicom/tome/pull/67))
- *(settings)* layered settings parser + composition resolver skeleton ([#66](https://github.com/devrelaicom/tome/pull/66))
- *(harness)* harness skeleton (HarnessModule trait, 5 stub impls, rules_file + mcp_config stubs) ([#65](https://github.com/devrelaicom/tome/pull/65))
- *(summarise)* summariser skeleton (Summariser trait, LlamaBackend singleton, StubSummariser, prompts) ([#64](https://github.com/devrelaicom/tome/pull/64))
- *(catalog)* refuse remove on enabled plugins; cascade with --force ([#32](https://github.com/devrelaicom/tome/pull/32))
- *(status,version)* add tome status; extend --version ([#29](https://github.com/devrelaicom/tome/pull/29))
- *(reindex)* add tome reindex subcommand ([#27](https://github.com/devrelaicom/tome/pull/27))
- *(catalog)* reindex enabled plugins on update; auto-disable orphans ([#26](https://github.com/devrelaicom/tome/pull/26))
- *(reindex)* add reindex_plugin_atomic + lifecycle wrappers ([#25](https://github.com/devrelaicom/tome/pull/25))
- *(plugin)* add interactive catalog/plugin browse flow ([#16](https://github.com/devrelaicom/tome/pull/16))
- *(query)* add tome query with reranker + strict mode ([#13](https://github.com/devrelaicom/tome/pull/13))

### Changed

- *(phase-7)* remove dead reference_count; sweep stale doc-comments; strip internal citations from --help (FR-016) ([#158](https://github.com/devrelaicom/tome/pull/158))
- *(phase-7)* move reconcile_hooks; sync.rs is now a thin orchestrator (no behaviour change) ([#144](https://github.com/devrelaicom/tome/pull/144))
- *(phase-7)* move reconcile_guardrails into harness/reconcile/guardrails.rs (no behaviour change) ([#143](https://github.com/devrelaicom/tome/pull/143))
- *(phase-7)* move reconcile_agents into harness/reconcile/agents.rs (no behaviour change) ([#142](https://github.com/devrelaicom/tome/pull/142))
- *(catalog)* rewire onto workspace_catalogs junction; derive metadata from filesystem ([#70](https://github.com/devrelaicom/tome/pull/70))
- *(plugin,index)* thread resolved workspace name through SQL queries ([#69](https://github.com/devrelaicom/tome/pull/69))
- *(workspace)* WorkspaceName newtype + Scope reshape (workspace names, not paths) ([#68](https://github.com/devrelaicom/tome/pull/68))
- *(util)* promote atomic-populated-directory helper to src/util/atomic_dir.rs ([#62](https://github.com/devrelaicom/tome/pull/62))
- *(paths)* collapse XDG-separated paths under <home>/.tome/; drop workspace inventory; read-only DB open across read paths ([#60](https://github.com/devrelaicom/tome/pull/60))
- tidy small code-review findings ([#37](https://github.com/devrelaicom/tome/pull/37))

### Dependencies

- *(deps)* audit serde_json/preserve_order + toml_edit scope ([#63](https://github.com/devrelaicom/tome/pull/63))

### Documentation

- *(phase-7)* P9 closeout — CHANGELOG v0.6.0 + CLAUDE.md + retro; fix 2 stale test comments ([#166](https://github.com/devrelaicom/tome/pull/166))
- *(review)* phase-7 phase-wide findings + disposition ([#163](https://github.com/devrelaicom/tome/pull/163))
- *(phase-7)* README front door + SECURITY.md + trust-model doc (FR-021/022/010) ([#162](https://github.com/devrelaicom/tome/pull/162))
- *(phase-7)* crate discovery metadata + docs.rs config + CHANGELOG/[Unreleased] reorder (FR-025) ([#160](https://github.com/devrelaicom/tome/pull/160))
- *(phase-7)* amend constitution → v1.4.0 (authorise release tooling) ([#140](https://github.com/devrelaicom/tome/pull/140))
- *(phase-7)* land planning artifacts + beta-readiness audits ([#138](https://github.com/devrelaicom/tome/pull/138))
- *(phase-6)* planning artifacts — spec, plan, research, contracts, tasks ([#130](https://github.com/devrelaicom/tome/pull/130))
- *(codebase,retro,claude-md)* close Phase 4 Polish (PR-G) ([#109](https://github.com/devrelaicom/tome/pull/109))
- *(review)* Phase 4 Polish phase-wide reviewer findings + disposition (PR-A) ([#103](https://github.com/devrelaicom/tome/pull/103))
- *(codebase,retro,claude-md)* refresh after Phase 4 / US5 ([#102](https://github.com/devrelaicom/tome/pull/102))
- *(codebase,retro,claude-md)* refresh after Phase 4 / US4 ([#98](https://github.com/devrelaicom/tome/pull/98))
- *(codebase,retro,claude-md)* refresh after Phase 4 / US3 ([#93](https://github.com/devrelaicom/tome/pull/93))
- *(codebase,retro,claude-md)* refresh after Phase 4 / US2 ([#87](https://github.com/devrelaicom/tome/pull/87))
- *(codebase,retro,claude-md)* refresh after Phase 4 / US1 ([#81](https://github.com/devrelaicom/tome/pull/81))
- *(codebase,retro,claude-md)* refresh after Phase 4 / F1–F11 Foundational ([#73](https://github.com/devrelaicom/tome/pull/73))
- README + CHANGELOG + contract reconciliation + v0.3.0 closeout ([#58](https://github.com/devrelaicom/tome/pull/58))
- Phase 10 retro and task closeout ([#41](https://github.com/devrelaicom/tome/pull/41))
- Phase 2 README, CHANGELOG, CLAUDE.md updates ([#40](https://github.com/devrelaicom/tome/pull/40))
- *(spec)* reconcile Phase 2 contracts with shipped behaviour ([#35](https://github.com/devrelaicom/tome/pull/35))
- Phase 9 codebase refresh + retro ([#33](https://github.com/devrelaicom/tome/pull/33))
- Phase 8 codebase refresh + retro ([#31](https://github.com/devrelaicom/tome/pull/31))
- Phase 7 codebase refresh + retro ([#28](https://github.com/devrelaicom/tome/pull/28))
- Phase 6 codebase refresh + retro ([#24](https://github.com/devrelaicom/tome/pull/24))
- Phase 5 codebase refresh + retro ([#21](https://github.com/devrelaicom/tome/pull/21))
- Phase 4 codebase refresh + retro ([#18](https://github.com/devrelaicom/tome/pull/18))
- codebase refresh + finalise Phase 3 retro ([#15](https://github.com/devrelaicom/tome/pull/15))

### Fixed

- *(mcp)* resolve scoped catalog from the DB in get_skill + search_skills (publish-blocker 3/3) ([#171](https://github.com/devrelaicom/tome/pull/171))
- *(plugin,reindex,query)* resolve scoped catalog discovery from the DB (publish-blocker 2/3) ([#170](https://github.com/devrelaicom/tome/pull/170))
- *(plugin)* resolve plugin dir from catalog DB, not the never-written config.toml (publish-blocker, 1/3) ([#168](https://github.com/devrelaicom/tome/pull/168))
- *(models)* bound aux model-file downloads + correct SECURITY.md verification scope (P9 MAJOR-2) ([#165](https://github.com/devrelaicom/tome/pull/165))
- *(harness)* agents cleanup-removal symlink refusal returns its dedicated code 45 (CON-1) ([#164](https://github.com/devrelaicom/tome/pull/164))
- *(phase-7)* off-spec inputs fail closed; config parse → exit 5; duplicate (kind,name) warned + truthfully counted (FR-013/014/015) ([#157](https://github.com/devrelaicom/tome/pull/157))
- *(catalog)* re-derive remove --force cascade inside the lock (F-REMOVE-TOCTOU) ([#155](https://github.com/devrelaicom/tome/pull/155))
- *(harness)* write inline rules body when any sharer needs it so OpenCode receives Tome's rules (F-RULES-OPENCODE) ([#154](https://github.com/devrelaicom/tome/pull/154))
- *(plugin,catalog)* bound every third-party read by its per-class cap (F-PLUGIN-MANIFEST-DOS class) ([#152](https://github.com/devrelaicom/tome/pull/152))
- *(workspace)* emit settings.toml via toml_edit + reject control chars in catalog names (F-WS-TOML-NEWLINE) ([#151](https://github.com/devrelaicom/tome/pull/151))
- *(mcp)* assign prompt names against a single global taken-set (F-MCP-PROMPT-COLLISION) ([#150](https://github.com/devrelaicom/tome/pull/150))
- *(query)* over-fetch+widen so filtered KNN returns min(top_k, matches) (F-KNN) ([#145](https://github.com/devrelaicom/tome/pull/145))
- *(catalog)* key cache dir + refcount by scrubbed URL so SSH sources round-trip (F-CACHE-KEY) ([#149](https://github.com/devrelaicom/tome/pull/149))
- *(models)* re-pin embedder to CPU-compatible INT8 ONNX artefact (F-MODEL-ONNX-CPU) ([#148](https://github.com/devrelaicom/tome/pull/148))
- *(doctor)* open index read-only, degrade not abort on schema mismatch (F-DOCTOR-RW) ([#147](https://github.com/devrelaicom/tome/pull/147))
- *(models)* download all required model files (tokenizer), not just the primary .onnx (F-MODEL-FILES) ([#146](https://github.com/devrelaicom/tome/pull/146))
- *(phase-5/us1d)* reviewer pass closeout — 1 blocker + 8 majors + docs refresh ([#115](https://github.com/devrelaicom/tome/pull/115))
- *(phase-4)* apply Polish PR-E security hardening (S-M1 + S-M2 + S-M6 + S-M7 + T-M8 + T416 + T419) ([#107](https://github.com/devrelaicom/tome/pull/107))
- *(phase-4)* apply Phase 4 Polish blockers C-B1 + C-B2 + C-B3 (PR-B) ([#104](https://github.com/devrelaicom/tome/pull/104))
- *(doctor)* US5 reviewer-flagged fixups (1 blocker + 10 majors) ([#101](https://github.com/devrelaicom/tome/pull/101))
- *(summarise)* US4 reviewer-flagged fixups (4 blockers + 9 majors) ([#97](https://github.com/devrelaicom/tome/pull/97))
- *(harness,settings)* US3 reviewer-flagged fixups ([#92](https://github.com/devrelaicom/tome/pull/92))
- *(workspace,catalog)* US2 reviewer-flagged fixups ([#86](https://github.com/devrelaicom/tome/pull/86))
- *(workspace,harness)* US1 reviewer-flagged fixups ([#80](https://github.com/devrelaicom/tome/pull/80))
- *(security)* mcp.log 0600, symlink rejection, registry validation, init refusal ([#56](https://github.com/devrelaicom/tome/pull/56))
- *(doctor)* orphan clones, workspace registry status, schema fix, signature ([#55](https://github.com/devrelaicom/tome/pull/55))
- *(mcp)* signal handling, log scrubbing, log taxonomy, specific-over-generic ([#54](https://github.com/devrelaicom/tome/pull/54))
- *(workspace)* enforce §Validation 1b/1c in resolver; add doctor drift tests ([#53](https://github.com/devrelaicom/tome/pull/53))
- *(mcp/log)* emit contract-pinned field names (ts/level/target/msg) ([#52](https://github.com/devrelaicom/tome/pull/52))
- *(security)* scrub catalog URL on add, chmod config 0600, ignore harness state ([#36](https://github.com/devrelaicom/tome/pull/36))
- *(catalog)* report real per-plugin skills_dropped in cascade ([#34](https://github.com/devrelaicom/tome/pull/34))

### Other

- Phase 6 Polish + v0.6.0 release ([#137](https://github.com/devrelaicom/tome/pull/137))
- Phase 5 Polish + v0.5.0 release ([#128](https://github.com/devrelaicom/tome/pull/128))
- doctor Phase 4 subsystems + Subsystem enum promotion + detected-uninstalled ([#99](https://github.com/devrelaicom/tome/pull/99))
- production LlamaSummariser + Qwen2.5 + tome models extension ([#94](https://github.com/devrelaicom/tome/pull/94))
- cross-harness module tests for all 5 production harnesses ([#90](https://github.com/devrelaicom/tome/pull/90))
- composition resolver verification + comprehensive tests ([#88](https://github.com/devrelaicom/tome/pull/88))
- workspace init + list + info Phase 4 fields ([#82](https://github.com/devrelaicom/tome/pull/82))
- tome workspace use <name> — core binding flow ([#74](https://github.com/devrelaicom/tome/pull/74))
- pre-allocate 8 new TomeError variants (codes 13–19, 24) ([#61](https://github.com/devrelaicom/tome/pull/61))
- Phase 4 setup + F1: deps + constitution v1.3.0 amendment ([#59](https://github.com/devrelaicom/tome/pull/59))
- Phase 3 Polish PR-A: review findings + disposition ([#51](https://github.com/devrelaicom/tome/pull/51))
- Phase 3 / US5: forward schema migrations ([#50](https://github.com/devrelaicom/tome/pull/50))
- Phase 3 / US4: tome doctor + --fix repairs ([#49](https://github.com/devrelaicom/tome/pull/49))
- Phase 3 / US3: per-command scope honouring + reference-counted catalog cleanup ([#48](https://github.com/devrelaicom/tome/pull/48))
- Phase 3 / US2: tome workspace info + init ([#47](https://github.com/devrelaicom/tome/pull/47))
- Phase 3 / US1: tome mcp — stdio MCP server with search_skills + get_skill ([#46](https://github.com/devrelaicom/tome/pull/46))
- Phase 3 Foundational F7+F8: schema-migration framework + MCP scaffolding ([#45](https://github.com/devrelaicom/tome/pull/45))
- Phase 3 Foundational (F1-F6): Scope + per-scope Paths + resolution + read-only DB + query::run_with_deps ([#44](https://github.com/devrelaicom/tome/pull/44))
- Phase 3 Setup: plan artefacts + rmcp/tokio dependencies ([#43](https://github.com/devrelaicom/tome/pull/43))
- Phase 6 US4 slice 2: integration tests for models download / list / remove ([#23](https://github.com/devrelaicom/tome/pull/23))
- Phase 6 US4 slice 1: tome models download / list / remove ([#22](https://github.com/devrelaicom/tome/pull/22))
- Phase 5 US3 slice 2: integration tests for disable, cheap re-enable, repeated state ([#20](https://github.com/devrelaicom/tome/pull/20))
- Phase 5 US3 slice 1: tome plugin disable + cheap re-enable verification ([#19](https://github.com/devrelaicom/tome/pull/19))
- Phase 3 US1 slice 3: integration tests for plugin enable/list/show/query + atomicity ([#14](https://github.com/devrelaicom/tome/pull/14))
- Phase 3 US1 slice 1b: tome plugin enable / list / show CLI surface ([#12](https://github.com/devrelaicom/tome/pull/12))
- Phase 3 US1 slice 1a: lifecycle::enable / disable + pinned MODEL_REGISTRY ([#11](https://github.com/devrelaicom/tome/pull/11))
- Phase 2 foundational (7/N): model-download integration test + docs + retro ([#10](https://github.com/devrelaicom/tome/pull/10))
- Phase 2 foundational (6/N): extend credential scrubber to model download ([#9](https://github.com/devrelaicom/tome/pull/9))
- Phase 2 foundational (5/N): embedding pipeline core ([#8](https://github.com/devrelaicom/tome/pull/8))
- Phase 2 foundational (4b/N): index features — lock, meta, integrity, skills, query ([#7](https://github.com/devrelaicom/tome/pull/7))
- Phase 2 foundational (4a/N): index bootstrap pipeline ([#6](https://github.com/devrelaicom/tome/pull/6))
- Phase 2 foundational (3/N): plugin metadata parsers ([#5](https://github.com/devrelaicom/tome/pull/5))
- Phase 2 foundational (2/N): presentation primitives + git-hooks migration ([#4](https://github.com/devrelaicom/tome/pull/4))
- Phase 2 foundational (1/N): error surface + paths ([#3](https://github.com/devrelaicom/tome/pull/3))
- Phase 2: setup — deps, vendored sqlite-vec, build infra ([#2](https://github.com/devrelaicom/tome/pull/2))
- Phase 1: Project Foundations and Catalog Management ([#1](https://github.com/devrelaicom/tome/pull/1))

### Chore

- *(phase-7)* [**breaking**] rename crate tome→tome-mcp, keep [[bin]] name = tome (FR-017) ([#159](https://github.com/devrelaicom/tome/pull/159))

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
