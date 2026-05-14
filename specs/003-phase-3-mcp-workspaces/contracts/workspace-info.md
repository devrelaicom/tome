# `tome workspace info` — Command Contract

```
tome workspace info [--json]
```

Reports the resolved workspace context for the current invocation. Read-only, never mutates. Honours the same global `--workspace` / `--global` flags as every other command.

## Behaviour

1. Resolve scope using [workspace-resolution.md](./workspace-resolution.md).
2. Load the resolved config (`Paths::config_file(&scope)`).
3. Open the resolved index DB read-only (if it exists; bootstrap-not-yet is not an error here).
4. Compute counts: catalogs, plugins total, plugins enabled, skills indexed.
5. Read `meta.schema_version` and `meta.embedder_name` / `meta.embedder_version` if the DB exists.
6. Emit human or JSON.

## Output (human)

Workspace case:

```
Workspace:       /home/user/projects/acme-app
  resolved via:  CWD walk
  catalogs:      3
  plugins:       12 total, 5 enabled
  skills:        47 indexed
  schema:        v1
  embedder:      bge-small-en-v1.5 1.5
```

Global case:

```
Workspace:       (global)
  resolved via:  global fallback
  catalogs:      8
  plugins:       45 total, 12 enabled
  skills:        156 indexed
  schema:        v1
  embedder:      bge-small-en-v1.5 1.5
```

When the index DB is not yet bootstrapped:

```
Workspace:       /home/user/projects/acme-app
  resolved via:  CWD walk
  catalogs:      0
  plugins:       0 total, 0 enabled
  skills:        not yet bootstrapped (no enabled plugins)
  schema:        —
  embedder:      —
```

"not yet bootstrapped" is an informational state, not an error.

## Output (`--json`)

```json
{
  "scope": "workspace",
  "path": "/home/user/projects/acme-app",
  "source": "cwd_walk",
  "catalogs": 3,
  "plugins_total": 12,
  "plugins_enabled": 5,
  "skills_indexed": 47,
  "schema_version": 1,
  "embedder": { "name": "bge-small-en-v1.5", "version": "1.5" }
}
```

Global case: `"scope": "global"`, `"path": null`, `"source": "global_fallback"` (or whichever priority source actually picked it).

Bootstrap-not-yet: `"schema_version": null`, `"embedder": null`.

`source` values: `"flag" | "global_flag" | "env" | "cwd_walk" | "global_fallback"` (matches `ScopeSource` in [data-model.md](../data-model.md)).

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Always, on successful report (even when the DB is not bootstrapped). |
| 2 | Usage error (e.g., `--workspace` with `--global`). |
| 7 | I/O error reading config or DB. |
| 35 | Index integrity check failure (rare; surfaces if the DB exists but PRAGMA integrity fails). |
| 70 | Workspace malformed (config.toml unreadable). |
| 71 | Workspace not found (explicit `--workspace <path>` named a path without `.tome/`). |
| 72 | Workspace conflict (`--workspace` + `--global`). |

## Relationship to `tome doctor`

`tome workspace info` is the narrow read-only report on the current scope. `tome doctor` is the broad diagnostic that includes this same information **plus** model state, drift, catalog cache integrity, and harness detection. Doctor's `workspace` subsection embeds this exact `WorkspaceInfo` record verbatim — when you see `--json` output from doctor, the `workspace` field has the shape documented here.

## Examples

```sh
$ cd ~/projects/acme-app
$ tome workspace info
Workspace:       /home/user/projects/acme-app
  resolved via:  CWD walk
  catalogs:      3
  plugins:       12 total, 5 enabled
  skills:        47 indexed
  schema:        v1
  embedder:      bge-small-en-v1.5 1.5

$ tome --global workspace info
Workspace:       (global)
  resolved via:  --global flag
  catalogs:      8
  plugins:       45 total, 12 enabled
  …

$ tome workspace info --json | jq '.plugins_enabled'
5
```
