# Tome — Phase 2 PRD

## Overview

Phase 2 makes Tome actually useful from the command line. Phase 1 gave us
project foundations and catalog management; Phase 2 lets the user select which
plugins from those catalogs they want active and indexes the SKILL.md
frontmatter into a local semantic search database that can be queried from the
CLI.

Still no MCP server, still no workspaces, still no cross-harness file
installation. This phase is purely about Tome's own state: which plugins the
user has opted into, and a queryable skill index over them.

## Conceptual model

Two distinct concepts that Phase 1 conflated and Phase 2 separates:

- **Installed.** A plugin is *installed* when its files exist on disk locally.
  That happens automatically as a consequence of `tome catalog add` — adding a
  catalog clones every plugin it ships. There is no separate install command in
  Phase 2.
- **Enabled.** A plugin is *enabled* when the user has opted to use it. Enabling
  a plugin indexes its skills into the search DB. Disabling marks them inactive
  and excludes them from query results. The user-facing verbs are
  `enable` / `disable`.

Plugins are identified everywhere by `<catalog>/<plugin>` — e.g.
`midnight-expert/compact-expert`. Catalogs are namespaces, and the same plugin
name can appear in multiple catalogs without colliding.

## Goals

1. Bundle a local skill index (SQLite + sqlite-vec, embedder, reranker) into
   the Tome binary, with explicit model management commands.
2. Provide an interactive `tome plugin` flow for browsing catalogs and
   enabling/disabling plugins, plus non-interactive equivalents for scripting.
3. Generate skill embeddings from SKILL.md frontmatter on enable; remove them
   on disable.
4. Provide a `tome query` command that returns ranked skill matches as a
   formatted table or JSON.
5. CLI presentation that earns the "top-notch" descriptor — colours,
   spinners, progress bars, and rendered tables throughout.

## Non-goals (Phase 2)

- MCP server (`tome mcp`)
- Workspaces (project-scoped catalogs and indexes); global only for now
- Cross-harness file installation (writing into `~/.claude/`, `~/.codex/`,
  etc.) — the index is Tome-internal
- Indexing non-skill plugin components (commands, agents, hooks). They are
  *counted* in the plugin view, but not embedded.
- Hybrid search (BM25 + semantic); pure semantic with reranking only
- Multiple embedding backends; bge-small-en-v1.5 + bge-reranker-base via
  fastembed-rs, period
- Plugin authoring tools (`tome plugin new`, scaffolding, validation, lint)
- Updates to plugin source files driven by Tome — Tome only reads

## CLI presentation

Phase 1 set the rule that every command supports `--json` for structured
output. Phase 2 adds rich human-facing presentation for the default case.

### Libraries

- **`indicatif`** — progress bars and spinners
- **`comfy-table`** — table rendering with borders, alignment, colour
- **`owo-colors`** — terminal colours (preferred over `colored`: faster,
  actively maintained, native `NO_COLOR` support)
- **`inquire`** — interactive prompts (select, multiselect, confirm)
- **`console`** — pulled in transitively by indicatif; used for terminal
  feature detection

### Where presentation appears

- **Progress bars** (`indicatif`):
  - Model download (with bytes / total / speed)
  - Embedding generation (skills processed / total)
  - Reindex operations
- **Spinners** (`indicatif`):
  - Git clone and pull operations
  - Model loading on first use
  - Database initialisation
- **Tables** (`comfy-table`):
  - `tome catalog list` (retrofit from Phase 1)
  - `tome plugin list`
  - `tome plugin show` (component breakdown)
  - `tome models list`
  - `tome query` results
- **Colour conventions** (`owo-colors`):
  - Green for success and enabled state
  - Yellow for warnings and stale state
  - Red for errors and disabled state
  - Cyan for hints and next-step suggestions
  - Bold for headers and primary identifiers
  - Dim for paths and secondary metadata

### `--json` and `--no-color`

Both bypass the rich presentation. `--json` emits structured output suitable
for `jq`. `NO_COLOR` env var or `--no-color` flag disables colour while
keeping tables and progress. Non-TTY stdout disables colour and progress
automatically.

## Plugin model

### What a plugin is

A plugin is a directory inside a catalog Git repo containing a
`.claude-plugin/plugin.json` manifest. We read Claude Code's plugin format
unchanged — no Tome-specific manifest in Phase 2. Existing Claude Code plugins
work as-is.

The plugin directory may contain:

- `skills/<skill-name>/SKILL.md` (and accompanying files)
- `commands/<command-name>.md`
- `agents/<agent-name>.md`
- `hooks/hooks.json` (plus any scripts)
- `.mcp.json` (MCP server declarations)

Only `SKILL.md` files are indexed in Phase 2. The other components are counted
in the plugin view but otherwise untouched.

### SKILL.md frontmatter handling

We read YAML frontmatter from the top of each SKILL.md. Only two fields matter
for Phase 2:

- `name` — used as the canonical skill name in the index.
- `description` — embedded for semantic search.

Fallback policy when either is missing or empty:

| Missing field | Fallback | Logged? |
|---|---|---|
| `name` | Skill directory name | yes, as warning |
| `description` | First 500 characters of SKILL.md body (post-frontmatter) | yes, as warning |
| Both | Above fallbacks applied independently | yes, two warnings |

Other frontmatter fields (`when_to_use`, `allowed-tools`, etc.) are ignored in
Phase 2.

### Plugin enable / disable semantics

- **Enable**: parses the plugin's SKILL.md files, generates embeddings, inserts
  rows into the skills table with `enabled = 1`.
- **Disable**: flips `enabled = 0` for all of that plugin's skill rows.
  Embeddings remain in the DB so re-enabling is cheap.
- **Query**: only returns rows with `enabled = 1`.

If a skill's source content is unchanged between disable and re-enable (matched
by content hash), no re-embedding is performed — we just flip the flag.

### Skill identity

Skills are identified by `(catalog, plugin, name)`. Two catalogs that ship
plugins with overlapping skill names produce separate rows; both can appear in
query results. The user disambiguates by inspecting catalog/plugin columns.

## Embedding pipeline

### Models

- **Embedder:** `bge-small-en-v1.5` INT8 ONNX (~45 MB), via `fastembed-rs`.
  384-dimensional output.
- **Reranker:** `bge-reranker-base` INT8 ONNX (~280 MB), via `fastembed-rs`.

Rationale lives in the embedding strategy report and the vector store research
document — not duplicated here.

### Model storage

Path: `${XDG_DATA_HOME:-~/.local/share}/tome/models/`

Models live under the data dir, not cache, so that an OS cache sweep doesn't
silently delete them. Each model is a subdirectory containing the ONNX files
plus a `manifest.json` recording name, version, source URL, and SHA-256.

### Embedding text composition

For each indexed skill, the text fed to the embedder is exactly:

```
{name}

{description}
```

Two lines, blank line between. No additional context, no full SKILL.md body.
The research showed short focused text outperforms verbose context for skill
retrieval.

### Reranker policy

The reranker is **on by default**. The CLI flag `--no-rerank` disables it on
`tome query`, primarily for debugging and benchmarking. Production use cases
should leave it on.

### When embeddings get generated

- **Plugin enable** (interactive or via `tome plugin enable`): full embed for
  any skills without a matching content hash.
- **Catalog update** (`tome catalog update`): for each enabled plugin in the
  updated catalog, diff content hashes; re-embed only changed skills.
- **`tome reindex`**: explicit, scoped to all / one catalog / one plugin.
  Force-rebuilds embeddings regardless of cached hashes.
- **Embedding model change** detected via DB metadata: `tome query` errors
  with a message pointing the user at `tome reindex`. We never silently mix
  embeddings across model versions.

## Index storage

### Bundling

`rusqlite` with the `bundled` feature statically links SQLite into the Tome
binary. The `sqlite-vec` extension is vendored as a single C file and compiled
in via a build script — no runtime dependency on a system SQLite or extension
loading.

### Database location

Path: `${XDG_DATA_HOME:-~/.local/share}/tome/index.db`

Single global database. WAL mode enabled at open time. No workspaces in Phase
2, so no per-project DBs.

### Schema

Approximate schema (final shape lands in code):

```sql
CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
-- keys: schema_version, embedding_model, embedding_model_version,
--       reranker_model, reranker_model_version

CREATE TABLE skills (
  id              INTEGER PRIMARY KEY,
  catalog         TEXT NOT NULL,
  plugin          TEXT NOT NULL,
  name            TEXT NOT NULL,
  description     TEXT NOT NULL,
  plugin_version  TEXT NOT NULL,
  path            TEXT NOT NULL,
  content_hash    TEXT NOT NULL,
  enabled         INTEGER NOT NULL DEFAULT 1,
  indexed_at      INTEGER NOT NULL,
  UNIQUE (catalog, plugin, name)
);

CREATE VIRTUAL TABLE skill_embeddings USING vec0(
  skill_id   INTEGER PRIMARY KEY,
  embedding  FLOAT[384]
);
```

### Schema and model versioning

`meta.schema_version` is set at DB initialisation. Future Phase 2 patches that
change the schema include migration code that bumps this. If Tome sees a
schema version it doesn't recognise, it refuses to run with a clear error.

`meta.embedding_model` and `embedding_model_version` track which model produced
the stored vectors. Changing either invalidates the index — `tome query` will
refuse and instruct the user to run `tome reindex --force`.

## CLI surface

### `tome plugin`

```
tome plugin                                       # interactive browse mode
tome plugin list [--catalog <name>]
tome plugin show <catalog>/<plugin>
tome plugin enable <catalog>/<plugin>
tome plugin disable <catalog>/<plugin> [--force]
```

Behaviour:

- **interactive mode** (no subcommand): the flow described in §Interactive
  enable/disable.
- **`list`**: table of every plugin across configured catalogs. Columns:
  catalog, plugin, version, status (enabled/disabled), skills, last updated.
- **`show <catalog>/<plugin>`**: rich plugin view. Same content as the
  interactive view, rendered for stdout.
- **`enable <catalog>/<plugin>`**: parses skills, generates embeddings,
  inserts/updates rows. Progress bar during embedding.
- **`disable <catalog>/<plugin>`**: prompts for confirmation, flips skills to
  `enabled = 0`. `--force` skips the prompt; required in non-TTY environments.

### `tome models`

```
tome models download
tome models list
tome models remove <name>
```

Behaviour:

- **`download`**: downloads the embedder and reranker into the models data dir,
  verifies SHA-256, with `indicatif` progress bars. Idempotent — already-present
  models are skipped unless `--force` is passed (re-downloads).
- **`list`**: table of installed models. Columns: name, version, size, path,
  status (ok / corrupt / missing).
- **`remove <name>`**: deletes a model. Prompts for confirmation; `--force`
  bypasses.

### `tome reindex`

```
tome reindex                                       # all enabled plugins
tome reindex <catalog>                             # all enabled plugins in one catalog
tome reindex <catalog>/<plugin>                    # single plugin
tome reindex --force                               # ignore content hashes
```

Re-runs the embedding pipeline. Default behaviour skips unchanged skills (by
content hash). `--force` re-embeds everything.

### `tome query`

```
tome query <text> [--top-k N] [--catalog X] [--plugin Y] [--no-rerank] [--json]
```

Behaviour:

- Required positional: query string.
- `--top-k N` (default 10): how many results to return after reranking.
- `--catalog X`, `--plugin Y`: filter results before reranking.
- `--no-rerank`: skip the reranker stage; pure embedding distance.
- `--json`: structured output per Phase 1 conventions.

Default output is a `comfy-table` with columns:

| catalog | plugin | skill | version | score | path |

Score is post-rerank if reranking ran, otherwise the embedding similarity.

### `tome catalog` (carry-over from Phase 1)

The interactions with enabled plugins:

- **`catalog update`**: after pulling Git changes, diff content hashes for
  every enabled plugin's skills. Re-embed changed skills automatically. Show a
  summary table of what changed.
- **`catalog remove`**: refuses to remove a catalog with any enabled plugins.
  Error message lists them and points at `tome plugin disable`. `--force`
  cascades: disables every plugin in the catalog and drops their skill rows
  before removing.

## Interactive enable/disable flow

Entered by running `tome plugin` with no subcommand.

```
1. Catalog selector (inquire Select)
   → list of configured catalogs, with plugin count and "enabled/total"
   → "Quit" option at the bottom

2. Plugin browser (inquire Select)
   → list of plugins in selected catalog
   → each row shows: name, version, status (✓ enabled / ✗ disabled)
   → "Back" option at the bottom

3. Plugin view (rendered with owo-colors + comfy-table)
   → metadata: name, version, last updated, author, status
   → component breakdown table (see §Plugin view)
   → action prompt (inquire Select):
     - "Enable plugin" (if disabled) / "Disable plugin" (if enabled)
     - "Back"

4. Action confirmation
   → Enable: progress bar for embedding generation
   → Disable: inquire Confirm prompt, then status update
   → returns to plugin browser (step 2) with updated status
```

The flow loops until the user quits. Every level has a "Back" / "Quit"
escape. Non-TTY callers must use the non-interactive subcommands; running
`tome plugin` without a subcommand in a non-TTY context errors.

## Plugin view

Rendered both in interactive mode and by `tome plugin show <catalog>/<plugin>`.

Header:

```
Plugin:       midnight-expert/compact-expert
Version:      1.2.0
Status:       ✓ enabled (last indexed 2 hours ago)
Last updated: 3 days ago — Alice <alice@midnight.network>
Description:  An expert on writing Compact smart contracts on Midnight.
```

Component breakdown (`comfy-table`):

| Component | Count |
|---|---|
| Skills | 12 |
| Agents | 2 |
| Commands | 5 |
| Hooks | 1 |
| MCP servers | 0 |

Status colours: green for enabled, dim red for disabled. Last-updated stale
checks: green if synced within the last 7 days, yellow if within 30, red if
older.

Data sources:

- **Version, author, description**: `.claude-plugin/plugin.json`
- **Last updated**: `git log -1 --format=%ci -- <plugin-path>` against the
  catalog repo
- **Component counts**: directory walks under `skills/`, `agents/`,
  `commands/`, `hooks/`, plus `.mcp.json` presence

## Index lifecycle reference

| Operation | Effect on skills DB |
|---|---|
| `catalog add` | None — plugins installed but not enabled |
| `catalog update` (no plugin changes) | None |
| `catalog update` (changes to enabled plugin) | Re-embed changed skills only |
| `catalog update` (plugin removed upstream) | Disable plugin, drop its rows, log loudly |
| `catalog remove` | Refused if any plugin enabled, unless `--force` |
| `catalog remove --force` | Disable all plugins in catalog, drop rows, remove |
| `plugin enable` | Embed and insert (or flip flag if content unchanged) |
| `plugin disable` | Flip `enabled = 0` on all skill rows for the plugin |
| `reindex` | Re-embed enabled plugins (changed skills only by default) |
| `reindex --force` | Re-embed all enabled plugins regardless of hash |
| Embedding model version mismatch | Query refuses; user runs `reindex --force` |

## First-run UX

Models are not bundled — they're downloaded on demand. First-time triggers:

- User runs `tome plugin enable …` and models are missing
- User runs `tome query …` and models are missing

Behaviour:

- **TTY context**: prompt `Models not found (~325 MB). Download now? [Y/n]`
  via `inquire`. If yes, run the download with progress bars and proceed.
- **Non-TTY context**: error with a clear message pointing at
  `tome models download` and exit with a dedicated non-zero code.

`tome models download` is also explicitly callable any time.

## Exit codes (additions to Phase 1)

| Code | Meaning |
|---|---|
| 7 | Plugin not found |
| 8 | Plugin already in requested state (e.g. enable on enabled) |
| 9 | Models missing or corrupt |
| 10 | Embedding generation failure |
| 11 | Query returned no results above threshold (only with `--strict` flag, default returns empty table with exit 0) |
| 12 | Index schema or model version mismatch |

## Success criteria

Phase 2 is done when:

- `tome models download` fetches both models with progress bars, verifies
  checksums, and stores them under the data dir.
- A fixture catalog containing two plugins with a total of ~10 SKILL.md files
  can be added, browsed interactively, and a plugin enabled — the embedding
  pipeline runs to completion within 10 seconds on a recent laptop.
- `tome query "how do I write a compact circuit"` against an enabled
  midnight-expert plugin returns the relevant skill in the top 3 results.
- `tome plugin show` displays accurate counts of skills, agents, commands, and
  hooks for a fixture plugin containing all four.
- `tome catalog update` correctly diffs and re-embeds only the skills whose
  source changed.
- `tome catalog remove` on a catalog with enabled plugins refuses with a
  helpful error; `--force` cascades cleanly.
- `--json` produces machine-parseable output for `plugin list`, `plugin show`,
  `models list`, and `query`.
- Disabling stdout TTY (piping to a file) suppresses colours and progress bars
  without breaking output.
- All Phase 1 success criteria still hold; CI matrix still green; lints clean.

## Resolved decisions

| Question | Decision |
|---|---|
| `install` vs `enable` | install = on disk (consequence of `catalog add`); enable = active intent |
| Plugin identity syntax | `<catalog>/<plugin>` |
| Plugin manifest format | Claude Code's `.claude-plugin/plugin.json` unchanged |
| SKILL.md frontmatter fields used | `name` and `description` only; fallback to dir name + first 500 chars of body |
| Cross-catalog skill name collisions | Separate rows, both appear in query |
| Embedding text | `"{name}\n\n{description}"` — nothing else |
| Embedder | `bge-small-en-v1.5` INT8 via fastembed-rs |
| Reranker | `bge-reranker-base` INT8, on by default, `--no-rerank` to disable |
| Vector store | SQLite + sqlite-vec, statically linked, bundled |
| DB location | `~/.local/share/tome/index.db` (single global) |
| Model location | `~/.local/share/tome/models/` (data dir, not cache) |
| Disable semantics | Flip `enabled = 0`, keep rows; cheap re-enable |
| `catalog remove` with enabled plugins | Refuse without `--force`; `--force` cascades |
| Interactive prompt library | `inquire` |
| Colour library | `owo-colors` |
| Progress / spinners | `indicatif` |
| Tables | `comfy-table` |
| First-run model download | Prompt in TTY, error in non-TTY |
| Embedding model version drift | Query refuses; user runs `reindex --force` |

## Phase 3 preview

Out of scope here, but signposted:

- MCP server (`tome mcp`) exposing `search_skills` and `get_skill` tools
  backed by the Phase 2 index
- Workspace awareness (per-project catalogs and indexes)
- Cross-harness installation — copying / symlinking enabled plugins into
  `~/.claude/plugins/`, `~/.codex/skills/`, and other harness-native locations
- Hooks, commands, and agents translation across harnesses
