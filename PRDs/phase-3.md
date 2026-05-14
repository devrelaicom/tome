# Tome — Phase 3 PRD

## Overview

Phase 3 turns Tome from a useful CLI into something agents can actually consume.
Phase 2 produced a queryable skill index; Phase 3 exposes that index over MCP
so any compliant harness can call `search_skills` and `get_skill` as tools.
Alongside the MCP server, Phase 3 introduces workspaces so the same machine
can hold per-project catalog state and indexes without everything bleeding
into one global pile.

Cross-harness file installation, hooks/commands/agents translation, and HTTP
transport for MCP are all explicitly deferred to Phase 4+.

## Goals

1. Ship `tome mcp` — a stdio MCP server backed by the Phase 2 skill index,
   exposing `search_skills` and `get_skill` tools.
2. Ship workspaces — per-project catalog state and skill index, auto-detected
   from CWD with global fallback.
3. Ship `tome doctor` — diagnostic command covering models, DB, catalog
   caches, harness detection, and workspace context.

## Non-goals (Phase 3)

- Cross-harness file installation (writing into `~/.claude/`, `~/.codex/`,
  etc.) — Phase 4
- Hooks, commands, or agents translation — Phase 4+
- HTTP / SSE transport for the MCP server — stdio only
- Multi-tenant or shared MCP servers
- Authentication, remote access, or non-local clients
- MCP tool annotations beyond plain descriptions (no caching hints, no
  permission scopes)
- Global → workspace migration tooling — workspaces are an explicit opt-in,
  no automatic conversion
- Validating that harness MCP configs reference Tome correctly — `tome doctor`
  reports Tome's own state, not the harness's

## MCP server

### SDK and transport

- **SDK**: [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk), the
  official Rust MCP SDK.
- **Transport**: stdio only. Universal across harnesses; HTTP can be added
  later without breaking the stdio contract.

### Process model

- Spawned by the harness as a long-lived child process — one MCP server
  instance per agent session.
- Connects to a single workspace (or global) for its lifetime. Switching
  workspaces means restarting the server.
- Reads the same SQLite DB as the CLI. SQLite WAL mode handles concurrent
  access from both cleanly.

### Tool surface

Two tools, both lazy, neither enumerates skills in its description:

#### `search_skills`

**Input**:
- `query` (string, required): natural-language description of what the agent
  needs.
- `top_k` (integer, optional, default 10): how many results to return after
  reranking.
- `catalog` (string, optional): filter to one catalog.
- `plugin` (string, optional, requires `catalog`): filter to one plugin.

**Output**: a JSON array of objects with these fields per match:
- `catalog`, `plugin`, `name` — identifiers
- `description` — the indexed description
- `plugin_version`
- `path` — absolute path to the SKILL.md file
- `score` — reranker score (or embedding similarity if `--no-rerank` mode is
  active on the server side)

Filters apply pre-rerank so the reranker only sees the filtered subset.

#### `get_skill`

**Input**:
- `catalog` (string, required)
- `plugin` (string, required)
- `name` (string, required)

**Output**: an object with:
- `content` — the full SKILL.md body (frontmatter stripped, body verbatim)
- `path` — absolute path to the SKILL.md
- `resources` — array of absolute paths to every other file in the skill's
  directory (no filtering, no inlining; the agent's own file tools fetch
  these if they want them)

If `(catalog, plugin, name)` doesn't resolve to an enabled skill, the tool
returns a structured error.

### Tool descriptions

The exact text of each tool's description is left as an implementation detail
to be iterated on, but the PRD locks the constraints:

- Must invite proactive use — agents should call `search_skills` before
  approaching substantial tasks.
- Must NOT enumerate available skills, catalogs, or plugins by name. Doing so
  defeats the entire purpose of the search tool.
- Must clarify that `get_skill` is the follow-up to `search_skills` results.
- Must keep total token cost of both descriptions well under any individual
  harness's per-tool budget.

### Model lifecycle in the server

- **Embedder** (~45 MB): eager-load at startup. Small enough that the few
  hundred milliseconds of load time is acceptable up front and the first
  query stays fast.
- **Reranker** (~280 MB): lazy-load on first `search_skills` call. Bigger,
  pays off only once per process lifetime, and not all sessions will use
  search.
- Both models verified by checksum at load time. Corrupt model → server
  fails to start (TTY shows error on stderr, sets non-zero exit).

### Startup validation

On startup, before accepting any MCP requests, the server checks:

1. DB exists and is readable.
2. `meta.schema_version` matches what this Tome version expects.
3. `meta.embedding_model` matches the embedder currently installed.
4. Required model files exist with correct checksums.

Any failure → log to stderr and the file log, exit with a dedicated code.
The agent harness sees the server fail to start; the user runs `tome doctor`
to see what's wrong.

### Logging

stdout is the protocol channel and cannot be used for logs. Logging strategy:

- **File**: `${XDG_STATE_HOME:-~/.local/state}/tome/mcp.log`, rotated by size
  (cap around 10 MB, keep the previous one).
- **stderr**: startup errors and fatal failures only; otherwise silent so the
  harness's console isn't polluted.
- **Filtering**: `RUST_LOG` env var works as usual.

## Workspaces

### Definition

A workspace is a directory that contains a `.tome/` subdirectory. The
`.tome/` directory holds:

```
.tome/
├── config.toml          # workspace catalog list and settings
└── index.db             # workspace skill index
```

The global state remains where Phase 2 put it
(`~/.local/share/tome/index.db`, etc.). Workspaces don't replace it; they
sit alongside.

### Discovery

When any command runs (including `tome mcp`), workspace resolution follows
this priority order:

1. `--workspace <path>` CLI flag (or `--global` to force global).
2. `TOME_WORKSPACE` environment variable.
3. Walk up from CWD looking for `.tome/`. First hit wins. Stop at filesystem
   root.
4. Fall back to global state.

The resolved workspace is logged (at debug level) and reported by
`tome workspace info`.

### Catalog cache sharing

Per-workspace state is just the catalog *list*, the enablement state, and
the index. The actual Git clones live globally:

- Catalog cache (cloned files): `~/.local/share/tome/catalogs/<url-hash>/` —
  one copy on disk per unique catalog URL, regardless of how many workspaces
  reference it.
- Workspace config records which catalogs that workspace knows about and
  pins the ref.

This means `tome catalog add` in a workspace either reuses an existing
clone (if some other workspace or global already cached the URL) or clones
fresh into the shared cache. Reference-counted; cleaned up only when no
workspace or global config references the URL.

### Per-workspace DB

- Each workspace has its own `index.db`.
- Skills enabled in workspace A are not enabled in workspace B or globally.
- Embedder/reranker models are shared globally — no per-workspace model
  copies.
- Schema and embedding-model version metadata live in each DB and are
  checked on open.

### `tome workspace init`

```
tome workspace init [<path>] [--inherit-global] [--force]
```

- Creates `<path>/.tome/` (or `./.tome/` if no path given).
- Errors if `.tome/` already exists, unless `--force`.
- Default: empty catalog list, empty index.
- `--inherit-global`: copy the global catalog list into the new workspace
  config. Does NOT copy enablement state — the user has to enable plugins
  per workspace explicitly.

Namespacing rationale: `tome workspace init` keeps the verb available for
future `tome catalog init`, `tome plugin init`, etc.

### `tome workspace info`

Reports current workspace context:

- Detected workspace path (or "global").
- Resolution method (CLI flag / env var / CWD walk / fallback).
- Catalog count and total plugin count.
- Enabled plugin count and indexed skill count.
- DB schema version and embedding model.

Default output is a `comfy-table`. `--json` for structured output.

### CLI behaviour with workspaces

Every existing command auto-detects workspace and operates on it. Concretely:

- `tome catalog add` adds to the workspace catalog list, not the global one.
- `tome plugin enable` enables in the workspace, not globally.
- `tome query` queries the workspace's index.
- `tome reindex` reindexes the workspace's enabled plugins.

To explicitly operate on global state from inside a workspace:

```
tome --global catalog add ...
tome --global plugin enable ...
```

The `--global` flag is a global flag accepted at the top level on every
command.

## `tome doctor`

A read-only (by default) diagnostic command.

```
tome doctor [--fix]
```

Checks performed:

- **Workspace context**: which workspace resolved, how, and from where.
- **Models**: presence and checksum of embedder and reranker. Reports size,
  path, status.
- **Database**: opens the resolved DB, reports schema version, embedding
  model recorded, total skill count, enabled count.
- **Catalog caches**: for every catalog in the resolved config, checks the
  cache dir exists, is a valid Git repo, has a manifest.
- **Harness detection**: existence of `~/.claude/`, `~/.codex/`,
  `~/.cursor/`, `~/.gemini/`, etc. Reports what's installed locally. (Phase
  4 will lean on this for installation targets.)

Output is a rendered report with `owo-colors` status glyphs:

- ✓ green for healthy
- ⚠ yellow for non-fatal issues (e.g. stale catalog cache, model older than
  recommended)
- ✗ red for broken (e.g. missing model, corrupt DB)

When issues are present, a "suggested fixes" section at the bottom lists
the specific commands to run.

`--fix` performs the obvious automatic repairs:

- Re-download missing or corrupt models.
- Re-clone broken catalog caches from their recorded URL.
- Run forward-compatible DB migrations (if any apply).

Destructive fixes (anything that would touch user data, like dropping a DB)
are never automatic, even with `--fix`. The doctor reports them; the user
runs the command.

`--json` produces machine-parseable output of the same data.

## CLI surface additions

```
tome mcp [--workspace <path> | --global]
tome workspace init [<path>] [--inherit-global] [--force]
tome workspace info
tome doctor [--fix]
```

Plus the global `--workspace <path>` / `--global` flag accepted on every
existing command.

## Schema migration plumbing

No schema changes are required between Phase 2 and Phase 3 (workspaces use
the same per-DB schema; each workspace just has its own DB file). But the
machinery to run migrations is added now so Phase 4+ schema bumps land
cleanly:

- DB open checks `meta.schema_version`.
- If older than current Tome expects: run registered migrations in order,
  inside a transaction, update version on success.
- If newer than current: refuse to open with a clear error pointing at
  upgrading Tome.

Migrations are registered in code as `(from_version, to_version, sql_fn)`
tuples and run by an in-process migrator. No external migration tooling.

## Success criteria

Phase 3 is done when:

- `tome mcp` registered in a Claude Code MCP config produces working
  `search_skills` and `get_skill` calls from inside a Claude Code session.
- `tome mcp` registered in at least one non-Claude-Code harness (Codex or
  Cursor — both for credit) works the same way.
- The MCP server starts in under 1 second on a recent laptop (embedder
  eager-loaded, reranker lazy).
- `search_skills` end-to-end latency p50 < 300 ms, p99 < 600 ms for an
  index of ~100 skills with reranker active.
- `tome workspace init` produces a working workspace; subsequent commands
  in that directory operate on workspace state rather than global.
- `--inherit-global` correctly seeds the workspace catalog list without
  copying enablement state.
- `tome --global` flag works from inside a workspace.
- `tome doctor` accurately reports state and `--fix` correctly repairs the
  three supported repair cases.
- Every Phase 1 and Phase 2 success criterion still holds; CI matrix still
  green; lints clean.

## Resolved decisions

| Question | Decision |
|---|---|
| Phase 3 scope | MCP server + workspaces only; cross-harness installation deferred |
| MCP SDK | `rmcp` (official Rust SDK) |
| MCP transport | stdio only |
| MCP tools | `search_skills` and `get_skill` |
| `get_skill` resource handling | return absolute paths to other files in skill dir; no inlining |
| Workspace inheritance from global | requires explicit `--inherit-global` flag on init |
| Workspace migration tooling | none; clean opt-in only |
| MCP tool descriptions | exact text is an implementation detail to iterate on |
| Workspace command namespacing | `tome workspace init` (keeps `init` available for other things) |
| Workspace detection priority | flag > env var > CWD walk > global fallback |
| Catalog cache scope | shared globally by URL hash; reference-counted across workspaces |
| Per-workspace state | own catalog list, own enablement, own DB; shared models |
| Embedder loading | eager at MCP startup |
| Reranker loading | lazy on first `search_skills` call |
| MCP logging | file at `~/.local/state/tome/mcp.log`, stderr for fatal only |
| `tome doctor` scope | Tome's own state; harness MCP config validation deferred |

## Phase 4 preview

Out of scope here, but the natural next steps:

- Cross-harness file installation: writing/symlinking enabled skills into
  the native plugin directories of detected harnesses (Claude Code first,
  then a chosen second harness).
- Commands and agents translation across harnesses.
- HTTP/SSE transport for the MCP server.
- Harness MCP config validation in `tome doctor`.
- Hooks translation (the gnarly one; possibly its own phase).
