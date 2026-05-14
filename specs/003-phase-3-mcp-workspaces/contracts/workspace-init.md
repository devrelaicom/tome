# `tome workspace init` — Command Contract

```
tome workspace init [<path>] [--inherit-global] [--force] [--json]
```

Creates a `.tome/` workspace marker at `<path>` (or `./` if omitted). The workspace starts with no enabled plugins; with `--inherit-global`, the global catalog list is copied into the new workspace's `config.toml` (catalog list only — enablement state is never copied).

## Arguments

| Argument | Semantics |
|---|---|
| `<path>` (positional, optional) | Workspace root. Defaults to `current_dir()`. Must be an existing directory; init does NOT create it. |
| `--inherit-global` | Copy the global `[catalogs]` table into the new workspace's `config.toml`. |
| `--force` | Replace an existing `.tome/` (DELETE then recreate). Without `--force`, init refuses on a pre-existing marker. |
| `--json` | NDJSON output instead of human. |

## Behaviour

1. Resolve `<path>` to an absolute, canonicalised directory. Error 7 (`Io`) if the path doesn't exist.
2. Check `path/.tome/`:
   - Doesn't exist → proceed.
   - Exists, `--force` not set → exit 4 (`CatalogAlreadyExists`-class; reused for workspace-already-initialised — the Phase 3 task plan promotes this to a dedicated variant only if a specific failure mode emerges).
   - Exists, `--force` set → remove `.tome/` recursively (atomic via `tempfile::TempDir::persist` rollback if rename fails), then proceed.
3. Create `path/.tome/` (mode 0700 on Unix; default on Windows).
4. Write `path/.tome/config.toml`:
   - Without `--inherit-global`: empty config (no `[catalogs]` map).
   - With `--inherit-global`: copy the global config's `[catalogs]` block verbatim. Catalog URLs and pinned refs carry over; no enablement state is involved at this layer (enablement is a property of the index DB, not the config).
5. The index database file `path/.tome/index.db` is **not** created at init time. The first command that needs to write the index (e.g., `tome plugin enable`) bootstraps it via the Phase 2 path.
6. Emit a success record.

## Output (human)

```
Initialized workspace at /home/user/projects/acme-app
  catalogs: 3 (inherited from global)
  config:   /home/user/projects/acme-app/.tome/config.toml
  index:    not yet bootstrapped (will be created on first enable)
```

Without `--inherit-global`:

```
Initialized workspace at /home/user/projects/acme-app
  catalogs: 0
  config:   /home/user/projects/acme-app/.tome/config.toml
  index:    not yet bootstrapped (will be created on first enable)

Next: run `tome --workspace /home/user/projects/acme-app catalog add <source>` to add a catalog,
      or rerun init with --inherit-global to seed catalogs from the global config.
```

The output uses absolute paths even when `<path>` was relative or omitted, so the user sees exactly what was created.

## Output (`--json`)

```json
{"workspace":"/home/user/projects/acme-app","catalogs":3,"inherited":true,"config_path":"/home/user/projects/acme-app/.tome/config.toml","index_bootstrapped":false}
```

Single object, NDJSON-compatible (one line, terminated by `\n`).

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Workspace created. |
| 2 | Usage error (e.g., `<path>` provided but doesn't exist). |
| 4 | Workspace already exists and `--force` not provided. |
| 7 | I/O error during creation (permission denied, disk full, etc.). |
| 8 | SIGINT during the operation; partial state cleaned via the atomic-rename pattern. |

## Atomicity

`.tome/` creation is atomic from the user's perspective: write into a sibling temp directory, fsync, then rename to `.tome/`. A SIGINT or crash during init leaves either no `.tome/` (rollback) or a complete `.tome/` (commit) — never a partial.

With `--force`, the existing `.tome/` is renamed aside before the new one lands, then removed best-effort. A crash between rename and removal leaves an orphan `.tome.old/` next to the new `.tome/`; doctor reports this as cleanup candidate.

## Side effects on the workspace registry

If the opt-in `${XDG_STATE_HOME}/tome/workspaces.txt` file exists, init appends the absolute workspace path (deduplicated) so the catalog-clone reference-counting (see [catalog-extensions-p3.md](./catalog-extensions-p3.md)) can find this workspace. If the registry file doesn't exist, init does NOT create it — registration is opt-in.

To opt in: `mkdir -p ~/.local/state/tome && touch ~/.local/state/tome/workspaces.txt` once. Subsequent `tome workspace init` invocations append automatically.

## Examples

```sh
$ cd ~/projects/acme-app
$ tome workspace init
Initialized workspace at /home/user/projects/acme-app
  catalogs: 0
  config:   /home/user/projects/acme-app/.tome/config.toml
  index:    not yet bootstrapped (will be created on first enable)

$ tome workspace init --inherit-global
Initialized workspace at /home/user/projects/acme-app
  catalogs: 3 (inherited from global)
  …

$ tome workspace init      # second run, no --force
error[4]: workspace already exists at /home/user/projects/acme-app/.tome
  hint: pass --force to replace, or `tome workspace info` to inspect

$ tome workspace init --force
Initialized workspace at /home/user/projects/acme-app
  catalogs: 0
  …
```

## What init does NOT do

- Does not run `tome plugin enable` on inherited catalogs. Enablement is per-workspace and explicit.
- Does not copy the global `index.db`. Each workspace starts with an empty index.
- Does not validate that inherited catalog URLs are reachable. The next `tome catalog update` does that.
- Does not write `.gitignore`. The developer decides whether `.tome/` is checked into the project's VCS (typically not — it's per-developer state).
- Does not modify the global `config.toml`.
