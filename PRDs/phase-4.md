# Tome — Phase 4 PRD

## Overview

Phase 4 does two things in one phase:

1. **Refactors the existing codebase** to a centralised architecture. The
   per-workspace storage model from Phase 3 didn't hold up under scrutiny —
   workspaces work better as named, centrally-stored objects that projects
   bind to. This phase moves the storage, rewrites the DB schema, and
   rebuilds the workspace concept on the new foundation.
2. **Adds cross-harness agent integration on top.** Auto-wires the Tome MCP
   server and a skill-discovery preamble into every supported harness's
   configuration, so plugins enabled in Tome become usable from any of them
   without the user touching a config file.

This is a chunky phase. Cross-harness installation (copying SKILL.md files
into harness plugin directories) does NOT happen — that approach was always
wrong, and is explicitly rejected by the architecture this phase puts in
place. SKILL.md files stay in the catalog cache forever; the integration is
purely configuration.

Hooks, commands, and agents translation are deferred to Phase 5+.

## Architectural corrections (supersedes Phase 1–3)

The following design decisions from prior PRDs are superseded by this one.
Prior PRD files are not edited — this section is the authoritative diff.

| Prior design | New design |
|---|---|
| Storage scattered across XDG dirs (`~/.config/tome/`, `~/.local/share/tome/`, `~/.local/state/tome/`) | Everything under `~/.tome/` |
| Per-workspace SQLite DB at `<project>/.tome/index.db` | Single central DB at `~/.tome/index.db` |
| Per-workspace skill enablement via `enabled` column on `skills` row | Workspace-agnostic `skills` rows; enablement lives in `workspace_skills` junction |
| Workspaces discovered by walking up CWD for `.tome/` containing state | Workspaces are named, centrally stored at `~/.tome/workspaces/<name>/`; projects bind to one via `.tome/config.toml` |
| `tome workspace init` creates a self-contained workspace | `tome workspace init <name>` creates the workspace in the central registry; `tome workspace use <name>` binds the current project to it |
| Workspace state self-contained in project's `.tome/` | Workspace state at `~/.tome/workspaces/<name>/settings.toml` + `RULES.md`; project's `.tome/` contains a binding pointer and a *copy* of `RULES.md` |
| Workspaces reference state via symlinks (originally considered) | File copy with explicit `tome workspace sync` (cross-platform; avoids Windows symlink elevation requirement) |
| `--workspace global` top-level flag | Global is just a workspace named `global` — same code path as user workspaces, no special-case flag needed |

The MCP server's tool surface (`search_skills`, `get_skill`) and stdio
transport are unchanged from Phase 3. Catalog management commands from
Phase 1 are unchanged in behaviour but their on-disk paths move.

## Goals

1. Refactor every path, schema, and code path touched by prior phases to
   the centralised architecture above.
2. Ship named-workspace lifecycle: `init`, `list`, `use`, `info`, `rename`,
   `remove`, `sync`.
3. Generate two cached natural-language summaries per workspace (short for
   MCP tool description, long for agent rules file) via a bundled local
   summarisation model.
4. Ship layered, composable harness configuration: project / workspace /
   global `settings.toml`, with composition syntax for inheritance and
   exclusion.
5. Ship cross-harness integration: auto-wire the Tome MCP server and
   skill-discovery preamble into Claude Code, Codex CLI, Gemini CLI,
   Cursor, and OpenCode.
6. Ship `tome harness` command surface: `use`, `remove`, `list`, `info`,
   `sync`.
7. Extend `tome doctor` to cover workspace bindings, rules-file integration
   state, MCP config entries, and bound-project consistency.

## Non-goals (Phase 4)

- Hooks, commands, or agents translation — Phase 5+
- HTTP / SSE transport for the MCP server — stdio only
- More harnesses than the five above — additive in Phase 5+
- Escape hatches for the bundled summarisation model — Qwen2.5-0.5B only,
  no config knobs in v1
- Cross-machine workspace sync — the architecture allows it (workspace
  dirs can be Dropbox'd / Git'd by the user) but Tome ships no tooling for
  it
- Backward compatibility / migration tooling for users on prior phases —
  pre-release, breaking changes acceptable, users wipe
  `~/.local/share/tome/` by hand if they upgrade
- WSL1 or WSL2-on-Windows-filesystem (`/mnt/c/...`) — supported
  environments are macOS, Linux, and WSL2 on the WSL filesystem only

---

# Part A — Refactor

## Path consolidation

Everything under `~/.tome/`. Final layout:

```
~/.tome/
├── config.toml          # global Tome config (small)
├── settings.toml        # global harness settings (NEW in Phase 4)
├── index.db             # central SQLite DB
├── catalogs/            # shared catalog clones, refcounted
│   └── <url-hash>/
├── models/              # embedder, reranker, summariser
│   ├── bge-small-en-v1.5/
│   ├── bge-reranker-base/
│   └── qwen2.5-0.5b-instruct/
├── logs/
│   └── mcp.log
└── workspaces/
    ├── global/          # privileged default workspace
    │   ├── settings.toml
    │   └── RULES.md
    └── <name>/          # user-created workspaces
        ├── settings.toml
        └── RULES.md
```

**Implementation work**:

- Drop the `directories` crate; resolve `~/.tome/` via `home::home_dir()`
  plus a single `paths` module that exposes typed accessors
  (`paths::db()`, `paths::catalogs()`, `paths::workspace_dir(name)`, etc.).
- Rewrite every callsite that referenced an XDG path. Audit via
  `rg "config_dir|data_dir|state_dir|cache_dir"` after the move.
- `tome doctor` already reports paths — update it to show the new layout.

No symlinks, no fallback to old locations. Pre-release users upgrading
wipe their old data manually; the PRD's non-goals make this explicit.

## Central database schema

Single DB at `~/.tome/index.db`. WAL mode at open time (unchanged from
Phase 3).

```sql
-- Schema metadata
CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
-- keys: schema_version, embedding_model, embedding_model_version,
--       reranker_model, reranker_model_version,
--       summariser_model, summariser_model_version

-- Workspaces (always at least one: 'global')
CREATE TABLE workspaces (
  id           INTEGER PRIMARY KEY,
  name         TEXT UNIQUE NOT NULL,
  created_at   INTEGER NOT NULL,
  last_used_at INTEGER NOT NULL
);

-- Canonical skill rows; one per (catalog, plugin, name).
-- Workspace-agnostic: enablement lives in workspace_skills.
CREATE TABLE skills (
  id             INTEGER PRIMARY KEY,
  catalog        TEXT NOT NULL,
  plugin         TEXT NOT NULL,
  name           TEXT NOT NULL,
  description    TEXT NOT NULL,
  plugin_version TEXT NOT NULL,
  path           TEXT NOT NULL,
  content_hash   TEXT NOT NULL,
  indexed_at     INTEGER NOT NULL,
  UNIQUE (catalog, plugin, name)
);

CREATE VIRTUAL TABLE skill_embeddings USING vec0(
  skill_id  INTEGER PRIMARY KEY,
  embedding FLOAT[384]
);

-- Per-workspace enablement.
CREATE TABLE workspace_skills (
  workspace_id INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  skill_id     INTEGER NOT NULL REFERENCES skills(id)     ON DELETE CASCADE,
  enabled_at   INTEGER NOT NULL,
  PRIMARY KEY (workspace_id, skill_id)
);

-- Per-workspace catalog enrolment (mirrors workspace's settings.toml).
CREATE TABLE workspace_catalogs (
  workspace_id INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  catalog_name TEXT NOT NULL,
  ref          TEXT NOT NULL,
  PRIMARY KEY (workspace_id, catalog_name)
);

-- Projects bound to workspaces.
CREATE TABLE workspace_projects (
  workspace_id INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  project_path TEXT NOT NULL,
  bound_at     INTEGER NOT NULL,
  PRIMARY KEY (workspace_id, project_path)
);
```

The `global` workspace row is created automatically on first DB
initialisation.

**Implementation work**:

- Drop the old per-workspace DB code path entirely.
- Rewrite every skills query to JOIN `workspace_skills` when scoping reads
  to a workspace.
- Migrate the existing `skills.enabled` column → `workspace_skills` rows.
  (Pre-release breaking change; no automatic migration shipped.)
- Schema migration plumbing from Phase 3 stays — first applied migration
  is this one.

## Workspace model rewrite

Workspaces are now named first-class objects at
`~/.tome/workspaces/<name>/`.

**Workspace state on disk**:

```
~/.tome/workspaces/<name>/
├── settings.toml    # workspace-scoped config (catalogs, harnesses, summaries)
└── RULES.md         # generated agent rules content; copied to bound projects
```

`settings.toml` schema:

```toml
name = "<workspace-name>"

# Cached summaries, regenerated on plugin enable/disable/update.
[summaries]
short        = "..."    # for MCP tool description
long         = "..."    # for RULES.md content
generated_at = "..."

# Workspace's enrolled catalogs (mirrored to workspace_catalogs table).
[[catalogs]]
name = "midnight-expert"
url  = "https://github.com/devrelaicom/midnight-expert"
ref  = "main"

# Workspace's harness configuration (see Part B for composition syntax).
harnesses = ["claude-code", "codex"]
```

**Project binding state**:

```
<project>/.tome/
├── config.toml      # { workspace = "<name>" }
└── RULES.md         # copy of ~/.tome/workspaces/<name>/RULES.md
```

`<project>/.tome/config.toml` may also contain a project-local
`harnesses = [...]` list (see Part B for composition).

**Discovery** (run on every command):

1. Resolve project: walk up from CWD for `.tome/config.toml`. If found,
   read `workspace = "<name>"` to identify the bound workspace.
2. If no `.tome/config.toml` is found above CWD, use the `global`
   workspace.
3. Override: `--workspace <name>` flag and `TOME_WORKSPACE` env var, in
   that priority order, both override discovery.

Note: `--workspace global` from Phase 3 is no longer special-cased —
`global` is just a workspace name. The flag works the same way for
`global` as for any other name.

**Implementation work**:

- Rewrite workspace discovery code from Phase 3's "walk for `.tome/` and
  open its DB" pattern to "walk for `.tome/config.toml`, read workspace
  name, query central DB."
- New `Workspace` type encapsulates the workspace's name, central
  directory, bound projects, and settings.
- Workspace creation now writes to `~/.tome/workspaces/<name>/` and
  inserts a row in `workspaces` table.
- All commands that operate on workspace state route through the central
  DB and central workspace directory.

## Catalog refcounting

Phase 1's catalog cache layout (`~/.tome/catalogs/<url-hash>/`) is
unchanged. What changes is refcount tracking:

- A catalog clone exists if at least one workspace enrols it (any row in
  `workspace_catalogs` referencing its name).
- `tome catalog add` from within a workspace context inserts a
  `workspace_catalogs` row for the active workspace; clones the cache only
  if no other workspace has the same URL.
- `tome catalog remove` from within a workspace context deletes the
  `workspace_catalogs` row for the active workspace. If no other workspace
  references the URL, delete the cache directory too.
- Refuse `catalog remove` (without `--force`) when the workspace has any
  plugins enabled from this catalog — same rule as Phase 2.

**Implementation work**:

- Rewrite catalog cache cleanup to consult `workspace_catalogs` row count
  before deleting the cache directory.
- `tome catalog list` shows the resolved workspace's catalogs only,
  queried via `workspace_catalogs`.

## Plugin enable/disable rewrite

Phase 2's "skill row per workspace with `enabled` column" becomes "skill
row per `(catalog, plugin, name)` (workspace-agnostic), `workspace_skills`
junction tracks enablement."

**Enable flow**:

1. Parse plugin's SKILL.md files; compute content_hash per skill.
2. For each skill: insert or update the `skills` row by
   `(catalog, plugin, name)`. If `content_hash` is unchanged from the
   existing row, skip re-embedding.
3. For each skill: insert a `workspace_skills` row for the active
   workspace. If already present, no-op.
4. Trigger summary regeneration for the workspace.
5. Trigger rules-file and MCP-config sync for any bound project (only if
   Tome is operating in a bound project; otherwise no integration work).

**Disable flow**:

1. For each skill in the plugin: delete the `workspace_skills` row for
   the active workspace.
2. Skill row itself stays — another workspace may still enable it.
3. Trigger summary regeneration.
4. Trigger integration sync (the new summary content needs to propagate).

**Reindex**: re-embeds skills whose content_hash differs (or all, with
`--force`). Triggered manually or by `tome catalog update` when an
enabled plugin's source changed.

**Implementation work**:

- Rewrite enable/disable code paths to operate on `workspace_skills`
  rather than `skills.enabled`.
- Add summary-regeneration hooks into both code paths (see Part B).
- Add integration-sync hooks (see Part B).

---

# Part B — New work

## Workspace lifecycle commands

```
tome workspace init <name> [--inherit-global]
tome workspace list
tome workspace info [<name>]
tome workspace use <name>
tome workspace rename <old> <new>
tome workspace remove <name> [--force]
tome workspace sync [<name>]
tome workspace regen-summary <name>
```

### `init <name>`

Creates `~/.tome/workspaces/<name>/` with empty `settings.toml` and an
empty `RULES.md`. Inserts a row in `workspaces`. Errors if `<name>`
already exists.

`--inherit-global` copies the global workspace's catalog list (only — not
the enabled plugins, not the harness list) into the new workspace's
`settings.toml`.

### `list`

Tabular output of all workspaces: name, catalog count, enabled plugin
count, indexed skill count, bound project count, last used. `--json` per
CLI conventions.

### `info [<name>]`

If `<name>` omitted: shows the currently resolved workspace (per
discovery). With `<name>`: shows that specific workspace.

Output includes: name, central directory, catalogs, enabled plugins,
indexed skill count, bound projects, cached summary lengths.

### `use <name>`

Run from within a project directory. Binds the current project to the
named workspace:

1. Errors if `<name>` doesn't exist (suggests `tome workspace init`).
2. Creates `<CWD>/.tome/` if missing.
3. Writes `<CWD>/.tome/config.toml` with `workspace = "<name>"`.
4. Copies `~/.tome/workspaces/<name>/RULES.md` to `<CWD>/.tome/RULES.md`.
5. Inserts `workspace_projects` row.
6. Runs `tome harness sync` automatically (so the new binding takes
   effect immediately for the configured harnesses).

Re-running `use` with the same name is a no-op. Running with a different
name rebinds: updates `.tome/config.toml`, recopies `RULES.md`, removes
the old `workspace_projects` row, inserts the new one, re-runs harness
sync.

### `rename <old> <new>`

Renames the workspace. Updates:

- `~/.tome/workspaces/<old>/` → `~/.tome/workspaces/<new>/`
- `workspaces.name` column
- Every bound project's `.tome/config.toml` (via `workspace_projects`
  paths)

Errors if `<new>` already exists or if any bound project's directory is
missing on disk.

### `remove <name>`

Deletes the workspace. Refuses without `--force` if any project is bound
to it (helpful error names the projects). With `--force`:

1. Tears down integration in every bound project
   (`tome harness sync`-style rollback).
2. Removes `<project>/.tome/` from every bound project.
3. Deletes the workspace's `workspace_skills`, `workspace_catalogs`,
   `workspace_projects` rows.
4. Deletes the `workspaces` row.
5. Removes `~/.tome/workspaces/<name>/`.
6. Refcount-cleans catalog caches no longer referenced.

The `global` workspace cannot be removed.

### `sync [<name>]`

Copies `~/.tome/workspaces/<name>/RULES.md` to every bound project's
`.tome/RULES.md`. Without `<name>`: syncs every workspace.

Idempotent. Used when the workspace's RULES.md has been regenerated
(Tome calls this automatically) or when the user manually edited it (and
wants the change pushed out).

Does NOT regenerate the summary — that's `regen-summary`. `sync` only
copies files.

### `regen-summary <name>`

Forces regeneration of the workspace's cached short and long summaries by
running the summarisation model against current plugin state. Writes back
to `settings.toml` and re-syncs RULES.md to bound projects. Useful for
debugging or after manual catalog/plugin tinkering.

## Bundled summarisation model

A third local model joins the embedder and reranker:

- **Model**: Qwen2.5-0.5B-Instruct, GGUF, INT4 quantisation.
- **Runtime**: `llama-cpp-2` Rust bindings.
- **On-disk**: `~/.tome/models/qwen2.5-0.5b-instruct/model.gguf`.
- **Footprint**: ~400 MB.
- **Lifecycle in CLI**: loaded only when summarisation is triggered;
  unloaded after.
- **Download**: managed by `tome models download` alongside the embedder
  and reranker.

No escape hatches in v1 — no config knobs to swap models, no support for
external API endpoints. If the bundled model proves inadequate in
dogfooding, that's Phase 5+ work.

### When summarisation runs

- Plugin enabled.
- Plugin disabled.
- Plugin reindexed (if any skill's content_hash changed).
- Catalog updated (if any enabled plugin's source changed).
- Explicit `tome workspace regen-summary <name>`.

After regeneration, the new RULES.md is written to
`~/.tome/workspaces/<name>/RULES.md`, then `tome workspace sync <name>`
runs to propagate to bound projects.

### What gets summarised

Input to the model: the descriptions of every enabled plugin in the
workspace, plus the names and descriptions of every indexed skill within
those plugins.

Two prompts run sequentially:

1. **Short summary** (target ~400–800 chars): "List the topics covered by
   these skills as a comma-separated phrase. No prose, no intro." Used
   inline in the MCP tool description.
2. **Long summary** (target ~1,500–2,500 chars): "Write a short rules
   section that tells an AI coding agent which topics this workspace has
   skills for, and instructs it to call the `search_skills` MCP tool when
   working on tasks involving those topics." Used as the body of
   `RULES.md`.

Both are cached in the workspace's `settings.toml` under `[summaries]`.
Regeneration overwrites.

### Tool description length — research item

The exact upper bound on what's safe to put in an MCP tool description
varies by host LLM and is not formally specified by the MCP protocol.
Phase 4 implementation should:

- Empirically test the short summary against each of the five supported
  harnesses.
- Target ~800 chars for the dynamic summary portion of the tool
  description, with ~200 chars of fixed scaffold ("Search the user's
  installed skill library for skills relevant to the current task…")
  around it.
- Log a warning if the cached short summary exceeds the threshold.

This is a sub-task within Phase 4, not a blocking research item before
implementation starts.

## Layered settings.toml and composition syntax

Three potential settings files, in priority order:

1. `<project>/.tome/config.toml` (project-scope harness list lives here,
   alongside `workspace = "..."`)
2. `~/.tome/workspaces/<name>/settings.toml`
3. `~/.tome/settings.toml`

Resolution order for the effective harness list of a project:

1. Read project settings. If it has a `harnesses` key, expand and
   resolve; that's the effective list.
2. Else read the bound workspace's settings. If it has a `harnesses`
   key, expand and resolve.
3. Else read global settings. If it has a `harnesses` key, expand and
   resolve.
4. Else: no harnesses; integration skipped.

A settings file *without* a `harnesses` key falls through. A settings
file *with* an empty `harnesses = []` resolves to no harnesses —
explicit opt-out of integration at that scope.

### Composition syntax

Entries in a `harnesses` array can be:

- `"<name>"` — explicit add (e.g., `"claude-code"`)
- `"!<name>"` — explicit remove (subtract from the union)
- `"[workspaces.<name>]"` — include all harnesses from another workspace
- `"[workspace]"` (singular, no name) — include all harnesses from the
  currently-bound workspace. Only valid in project-scope settings.
- `"[global]"` — include all harnesses from global settings

Resolution: union all references and adds, then subtract everything
prefixed with `!`. Order in the array doesn't matter.

**Cycle detection**: resolution walks the reference graph DFS, tracking
visited nodes. A cycle produces a hard error naming the path. `[global]`
is terminal (it references nothing further).

**Restrictions**:

- `[workspace]` (singular) only valid in project settings — workspaces
  aren't bound to anything.
- Composition references in workspace and global settings are allowed
  (referencing other workspaces, or `[global]`).
- A workspace can be referenced even if no project is bound to it; the
  reference resolves to the workspace's `harnesses` list as written.

Example project settings:

```toml
workspace = "my-project"

harnesses = [
  "[workspace]",          # everything from the bound workspace
  "[workspaces.shared]",  # plus everything from "shared" workspace
  "cursor",               # plus cursor explicitly
  "!cline",               # minus cline if present in any source
]
```

## Harness modules

Each supported harness is a code module exposing:

- `name`: stable identifier (`claude-code`, `codex`, `gemini`, `cursor`,
  `opencode`)
- `detect()`: returns whether the harness is installed locally (checks
  conventional paths like `~/.claude/`, `~/.codex/`, etc.)
- `rules_file_target(project_path)`: returns the path of the rules file
  Tome should write to, following the precedence: existing rules file
  the harness reads → AGENTS.md → harness-specific default. For
  multi-file rules harnesses (Cursor), returns a path to a Tome-specific
  file in the rules directory.
- `rules_file_strategy()`: returns `BlockInExistingFile` or
  `StandaloneFile`, determining whether Tome writes a delimited block or
  a separate file.
- `mcp_config_path(project_path)`: returns the MCP config file path for
  this harness (project-scoped if the harness supports per-project MCP
  config, global fallback otherwise).
- `mcp_config_format()`: `JsonObject { ... }`, `TomlTable { ... }`, etc.
- `mcp_config_key()`: name under which Tome's entry lives. Standardised
  on `"tome"` for naming consistency across harnesses (the rules-file
  text references this name).

### Five harness modules to ship

| Harness | Strategy | Rules file (default) | MCP config (default) |
|---|---|---|---|
| `claude-code` | `BlockInExistingFile` | AGENTS.md > CLAUDE.md | `.claude/settings.json` |
| `codex` | `BlockInExistingFile` | AGENTS.md | `~/.codex/config.toml` (global) |
| `gemini` | `BlockInExistingFile` | AGENTS.md > GEMINI.md | `~/.gemini/settings.json` (global) |
| `cursor` | `StandaloneFile` | `.cursor/rules/TOME_SKILLS.md` | `.cursor/mcp.json` |
| `opencode` | `BlockInExistingFile` | AGENTS.md | TBD |

**Implementation note**: the rules-file paths, MCP config formats, and
per-harness scope behaviour listed above are the best information
available at PRD time but require verification at implementation start —
this ecosystem moves fast and module specifics will need to be checked
against each harness's current docs. The PRD specifies the integration
*pattern*; per-harness specifics are filled in during implementation.

## Rules-file integration

Two strategies depending on the harness module's
`rules_file_strategy()`:

### Strategy 1: `BlockInExistingFile`

Tome maintains a delimited block in a single rules file:

```markdown
... user's existing content above ...

<!-- tome:begin -->
@.tome/RULES.md
<!-- tome:end -->

... user's existing content below ...
```

The block contents are an `@`-include directive pointing to the
project's `.tome/RULES.md` (which is a copy of the workspace's
RULES.md). When the workspace RULES.md regenerates and syncs to the
project, the agent sees the updated content through the include — no
edit to the rules file itself needed.

Verification of `@`-include support per harness happens at
implementation time. For any harness that doesn't support `@`-includes,
the fallback is inline content (the full RULES.md text between the
delimiters). The harness module declares which mode it uses.

### Strategy 2: `StandaloneFile`

Tome writes a complete rules file with no delimiters:

- File path: per the harness module (e.g.,
  `.cursor/rules/TOME_SKILLS.md`).
- Contents: the full RULES.md text inline (no `@`-includes in this
  strategy).
- File is entirely Tome-owned; rolling back deletes the file.

Used for harnesses where multiple rules files coexist as separate
documents (Cursor's `.cursor/rules/*.mdc` pattern).

### Shared rules file caveat

When multiple harnesses target the same rules file (e.g., several
harnesses all using AGENTS.md), Tome writes one block — the file's
integration state is per-file, not per-harness. Sync's removal logic
respects this: the block is removed only when *no* harness in the
effective list points to that file. Details under "Sync algorithm"
below.

## MCP config integration

Per harness, Tome writes one entry to the harness's MCP config file. The
entry is keyed `"tome"` (consistent across harnesses for naming
continuity in rules-file text).

### Entry contents

The entry Tome writes for a project bound to workspace `<name>`:

```json
{
  "command": "tome",
  "args": ["mcp", "--workspace", "<name>"]
}
```

The TOML equivalent is structurally identical (the harness module knows
the format).

### Marker / match logic

Tome doesn't track integration state in a sidecar file or DB. It reads
the MCP config and infers state by content.

For a `tome` entry in the existing config:

1. If `command == "tome"` and `args[0] == "mcp"`: Tome-owned entry.
   Update to current state freely (e.g., rewrite the workspace arg if
   rebinding). No `--force` needed.
2. If `command` or `args[0]` differ: user-owned entry sharing the name.
   Refuse to overwrite without `--force`. Error message quotes the
   existing entry and suggests resolution.

`env` field is ignored in comparison — if the user adds env vars to a
Tome-owned entry, those persist through updates.

## `tome harness` command surface

```
tome harness                              # list supported harnesses
tome harness list [<workspace>]           # list configured harnesses in scope
tome harness use <name> [--scope project|workspace|global] [--force]
tome harness remove <name> [--scope project|workspace|global]
tome harness info <name>
tome harness sync
```

### `tome harness` (bare)

Lists all harnesses Tome supports (the five listed above). Tabular
columns: name, detected on this system (yes/no), rules-file target it
would use for the current project, MCP config path it would write.

### `harness use <name>`

- `--scope project` (default): adds `<name>` to
  `<CWD>/.tome/config.toml`'s `harnesses` array. Creates the array if
  absent.
- `--scope workspace`: adds to the resolved workspace's `settings.toml`.
- `--scope global`: adds to `~/.tome/settings.toml`.

After the settings file is updated, the command recomputes the
effective harness list for the current project. If the addition changed
that list, run the integration logic for the new harness(es) in the
current project (write rules-file block, write MCP config entry).

If the addition didn't change the effective list (e.g., scope is
`global` but the project has its own list that doesn't reference
global), the command prints a notice: "Added to global settings, but
this project's effective list is unchanged. Run `tome harness sync` in
projects that should pick this up."

`--force` overwrites a clashing `tome` entry in an MCP config (see
marker match logic above).

### `harness remove <name>`

- `--scope project` (default): removes from
  `<CWD>/.tome/config.toml`.
- `--scope workspace`: removes from the resolved workspace's
  `settings.toml`.
- `--scope global`: removes from `~/.tome/settings.toml`.

After the settings file is updated, recompute the effective harness
list. If the removal changed it, roll back the integration in the
current project (remove rules-file block if no longer needed, remove
MCP config entry).

If the removal didn't change the effective list, just edit the settings
file and notify the user that other affected projects need
`tome harness sync`.

### `harness list [<workspace>]`

Without args: shows the effective harness list for the current project,
with each harness annotated by source (project / workspace / global /
multiple).

With `<workspace>`: shows that workspace's directly-configured
harnesses (without walking up to global). Useful for inspecting a
specific workspace's intent.

### `harness info <name>`

Per-harness detail for the current project:

- Name, description.
- Detected on system: yes/no, paths.
- Rules-file target Tome would write to in this project.
- MCP config target.
- Currently integrated? (Inferred from filesystem; see sync algorithm.)
- Which of the three settings files reference this harness, and how.

### `harness sync`

The reconciler. Run from a project directory. Computes the effective
harness list and reconciles the project's actual integration state to
match.

## Sync algorithm

Run from a project directory:

1. Compute the effective harness list `L` via the layered settings
   lookup and composition resolution.
2. For each harness `h` in `L`, ask its module: which rules file would
   it write to (`rules_target[h]`)? Which MCP config? With what
   strategy?
3. Build `target_rules_files` = `{ rules_target[h] for h in L }`.
4. **Rules-file reconciliation**:
   - For each file path in `target_rules_files`: ensure the file exists
     with a current tome block (BlockInExistingFile strategy) or
     current content (StandaloneFile strategy).
   - For each rules-file path Tome could possibly have written to
     (AGENTS.md, CLAUDE.md, GEMINI.md,
     `.cursor/rules/TOME_SKILLS.md`, etc.): if it has a tome block or
     is a Tome-owned standalone file, *and* it's not in
     `target_rules_files`, remove the block or delete the file.
5. **MCP config reconciliation**:
   - For each `h` in `L`: ensure `h`'s MCP config has the `tome` entry
     matching the project's current binding state. Update if Tome-owned
     but stale (e.g., workspace arg changed). Error if user-owned and
     not `--force`.
   - For each supported harness *not* in `L`: ensure `h`'s MCP config
     does NOT contain the `tome` entry. Remove if present and
     Tome-owned.

The state inference is filesystem-derived (no sidecar state). A
partially-integrated harness (rules file written but MCP missing, or
vice versa) is reconciled by the sync.

Sync prints a summary of what changed: added integrations, removed
integrations, leave-alones.

## Doctor extensions

`tome doctor` from Phase 3 extends to cover:

- Resolved workspace and resolution method (per Phase 3).
- Bound project consistency: does `.tome/config.toml` exist where the DB
  says it should? Does `.tome/RULES.md` match the workspace's RULES.md?
- Effective harness list for the current project.
- Per supported harness: integrated yes/no, partial integration warnings
  (rules-file block exists but no MCP entry, or vice versa).
- Models: embedder, reranker, summariser presence and checksum.
- `--fix` repairs: re-copy stale RULES.md, redownload corrupt models,
  re-clone broken catalog caches. Destructive operations (removing tome
  blocks, deleting workspaces) are never automatic.

---

## CLI surface — additions and changes

New commands:

```
tome workspace init <name> [--inherit-global]
tome workspace list
tome workspace info [<name>]
tome workspace use <name>
tome workspace rename <old> <new>
tome workspace remove <name> [--force]
tome workspace sync [<name>]
tome workspace regen-summary <name>

tome harness                              # bare = list supported harnesses
tome harness list [<workspace>]
tome harness use <name> [--scope project|workspace|global] [--force]
tome harness remove <name> [--scope project|workspace|global]
tome harness info <name>
tome harness sync
```

Changed commands (behaviour shifts; surface unchanged):

```
tome catalog add | remove | update | list | show  # operates on workspace_catalogs
tome plugin enable | disable                       # operates on workspace_skills
tome query                                         # joins through workspace_skills
tome reindex                                       # scoped to effective workspace
tome mcp                                           # workspace via .tome/config.toml
tome doctor                                        # extensions per above
```

Removed (no longer relevant):

- `--global` top-level flag (replaced by named `global` workspace)
- Per-workspace DB initialisation logic (centralised)

## Exit codes (additions)

| Code | Meaning |
|---|---|
| 13 | Workspace not found |
| 14 | Workspace already exists |
| 15 | Workspace name invalid (reserved word, illegal chars, etc.) |
| 16 | Workspace has bound projects (remove without `--force`) |
| 17 | Composition cycle detected |
| 18 | Harness not supported |
| 19 | Harness clash (existing `tome` MCP entry doesn't match Tome-owned shape) |
| 20 | Summarisation model failure |

## Success criteria

Phase 4 is done when:

- All paths under `~/.tome/`. No code references `~/.config/tome/`,
  `~/.local/share/tome/`, or `~/.local/state/tome/`.
- Single central DB. No per-workspace DBs anywhere. All workspace-scoped
  queries route through `workspace_skills` / `workspace_catalogs`.
- `tome workspace init my-project` creates the central workspace dir
  and inserts a row.
- `tome workspace use my-project` from a project directory creates
  `.tome/config.toml`, copies `RULES.md`, registers the binding, and
  runs `tome harness sync` automatically.
- Two bound projects share the same central RULES.md; updating one
  workspace's enabled plugins regenerates RULES.md and propagates to
  both projects on sync.
- Qwen2.5-0.5B summarises a fixture workspace's enabled plugins into
  both a short and a long summary, cached in `settings.toml`. Both
  summaries regenerate when the plugin set changes.
- Layered settings resolution works: a project with no harness key
  inherits from its workspace; a project with `[workspace]` composition
  inherits explicitly; `!name` exclusion removes a harness; cycle
  detection errors loudly.
- Auto-integration: `tome harness use claude-code` from a fixture
  project writes the rules-file block and the MCP config entry.
- Multi-harness integration: enabling Codex and Gemini, both targeting
  AGENTS.md, writes a single shared block. Removing Codex (only) leaves
  the block intact for Gemini.
- Conflicts: an existing non-Tome `tome` MCP entry causes an exit-19
  error with clear message; `--force` overrides.
- Cursor's multi-file integration: `tome harness use cursor` writes
  `.cursor/rules/TOME_SKILLS.md`; removal deletes the file.
- `tome harness sync` is idempotent — running twice in a row produces
  no changes the second time.
- `tome doctor` accurately reports workspace bindings, integration
  state, and inconsistencies. `--fix` repairs the supported cases.
- All Phase 1, 2, and 3 success criteria still hold (against the
  refactored architecture).
- CI matrix green on macOS, Linux, and WSL2-on-WSL-filesystem.

## Resolved decisions

| Question | Decision |
|---|---|
| Storage layout | Everything consolidated under `~/.tome/` |
| Database | Single central `~/.tome/index.db` with workspace junction tables |
| Workspace storage | Named, centralised at `~/.tome/workspaces/<name>/` |
| Project binding mechanism | File copy of `RULES.md` (not symlinks) |
| Symlink fallback | Not needed; file copy works cross-platform |
| Supported environments | macOS, Linux, WSL2 on WSL filesystem |
| Schema migration tooling | Plumbing exists; pre-release breaking changes need no migration |
| Summary model | Qwen2.5-0.5B-Instruct, GGUF INT4, llama-cpp-2 runtime |
| Summary escape hatches | None in v1 |
| Two summaries | Short for MCP tool description, long for RULES.md |
| Summary regeneration triggers | Plugin enable/disable, plugin reindex with content change, catalog update affecting enabled plugins, explicit `regen-summary` |
| Settings layering | Project → workspace → global, stop at first with `harnesses` key |
| Composition syntax | `[workspaces.<name>]`, `[workspace]`, `[global]`, `<name>`, `!<name>` |
| Cycle detection | DFS during resolution; hard error on cycle |
| Composition at workspace/global levels | Allowed |
| Supported harnesses | Claude Code, Codex CLI, Gemini CLI, Cursor, OpenCode |
| Rules-file precedence | Existing rules file > AGENTS.md > harness-specific |
| Rules-file strategy | `BlockInExistingFile` (delimited) or `StandaloneFile` (multi-file rules harnesses) |
| Shared rules files | One block per file regardless of how many harnesses share; per-file removal logic |
| MCP entry key | `"tome"` (consistent across harnesses) |
| MCP marker logic | `command == "tome" && args[0] == "mcp"` → Tome-owned; else user-owned |
| MCP conflict handling | Error (exit 19); `--force` to overwrite |
| Integration state tracking | None; inferred from filesystem at sync time |
| `tome workspace use` post-action | Auto-runs `tome harness sync` |
| `tome workspace remove` with bound projects | Refuse without `--force`; `--force` cascades |
| `global` workspace | Cannot be removed |

## Phase 5 preview

Out of scope here, but signposted:

- Commands translation: harness-aware translation of `commands/*.md`
  files from plugins into each harness's command surface (or a
  search-based alternative if commands have the same enumeration problem
  as skills).
- Agents (subagents) translation: design challenge, since most
  non-Claude harnesses have no equivalent concept.
- Hooks translation: gnarly, harness-by-harness semantics, likely its
  own phase.
- Additional harnesses: Cline, Goose, Aider, etc.
- HTTP / SSE transport for the MCP server.
- Plugin authoring tools: `tome plugin new`, validators, lint, catalog
  publishing helpers.
- Cross-machine workspace sync tooling (Dropbox / Git-backed
  workspaces).
