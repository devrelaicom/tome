# Workspace Commands — Contract

**Spec source**: [spec.md FR-400 through FR-411](../spec.md)

Eight workspace lifecycle commands. Every command honours the global `--json` flag (structured output to stdout, byte-stable wire shape) and exits with the appropriate `TomeError` code on failure (see [exit-codes-p4.md](./exit-codes-p4.md)).

## `tome workspace init <name> [--inherit-global]`

Creates a new workspace.

**Arguments**:

| Argument | Semantics |
|----------|-----------|
| `<name>` (positional, required) | Workspace name; parsed against the FR-347 rule. |
| `--inherit-global` | Seed the new workspace's `[catalogs]` from the global workspace's enrolments at the moment of creation. |
| `--json` | Structured output. |

**Algorithm**:

1. Parse `<name>` via `WorkspaceName::parse` — exit 15 on failure.
2. Open central DB under the advisory lockfile.
3. Check `workspaces.name = <name>`: if present, exit 14.
4. Inside one DB transaction:
   - Insert `workspaces` row (`created_at = now`, `last_used_at = now`).
   - If `--inherit-global`: copy `workspace_catalogs` rows from `global` workspace, rewriting `workspace_id` to the new row's id.
5. Outside the DB, land the workspace directory via `atomic_dir::land_directory(<root>/workspaces/<name>/, 0o700, |dir| { … })`:
   - Write `dir/settings.toml` with `name = "<name>"`, the inherited catalogs (if any), and an empty `[summaries]` section.
   - Write `dir/RULES.md` containing a single comment line indicating "no summary yet."

**Output**:

```text
Initialised workspace `<name>` at /home/user/.tome/workspaces/<name>
  catalogs: 3 (inherited from global)
  next:     `cd <project> && tome workspace use <name>` to bind a project
```

`--json`:

```json
{"name":"<name>","path":"/home/user/.tome/workspaces/<name>","catalogs_inherited":3,"id":7}
```

## `tome workspace list`

Reports every workspace in the central registry.

**Algorithm**: One SQL query joins `workspaces` × counts from `workspace_catalogs` × `workspace_skills` × `workspace_projects`.

**Output**: Tabular per Phase 2 style. Columns: name, catalogs, plugins (enabled), skills (indexed), bound projects, last used.

`--json`: Array of `WorkspaceListEntry` records.

## `tome workspace info [<name>]`

Reports one workspace's details.

**Arguments**: `<name>` optional. If omitted, reports the currently resolved workspace per the FR-344 algorithm. If named, reports that workspace regardless of CWD.

**Output**: Multi-line human form; structured form per [data-model.md §4](../data-model.md) (`WorkspaceInfo` style, Phase 4 widened to include catalog-list, enabled-plugin-list, bound-project-list, and cached-summary lengths).

**Failure**: `<name>` missing → exit 13.

## `tome workspace use <name>`

Binds the current project to a workspace.

**Arguments**:

| Argument | Semantics |
|----------|-----------|
| `<name>` (required) | Workspace name. |
| `--json` | Structured output. |

**Pre-conditions**:

- CWD canonical path ≠ user's home directory.
- CWD canonical path ≠ filesystem root.
- `<name>` exists in `workspaces` table — exit 13 otherwise.

**Algorithm** (two-phase lock; mirrors the sync algorithm's contention profile):

**Phase A — DB + project marker (under the lock)**:

1. Acquire the central DB's advisory lockfile.
2. Validate `<name>` exists in `workspaces` table — exit 13 otherwise.
3. UPSERT into `workspace_projects` keyed on `project_path = canonical(CWD)`. New value: `workspace_id = id(<name>)`, `bound_at = now`.
4. Land the project marker via `atomic_dir::land_directory(<CWD>/.tome/, 0o700, |dir| { … })`:
   - Write `dir/config.toml` containing `workspace = "<name>"`.
   - Copy `<root>/workspaces/<name>/RULES.md` to `dir/RULES.md`.
5. Release the advisory lockfile.

**Atomicity tiers for phase A**:

| Tier | Behaviour on failure |
|------|---------------------|
| 2 (DB UPSERT) commits before 4 (marker landing) starts | If 2 succeeds and 4 fails, the central DB has a binding row but no `.tome/` on disk; the lockfile is released by the unwinding stack via `Drop`. Doctor's `Binding` subsystem flags the orphan; `tome workspace use <same-name>` retried recovers the marker without changing the DB row. |
| 4 (atomic-rename-on-same-FS) commits at the rename | If 4 fails before the rename, the staged temp dir is auto-cleaned by `TempDir::drop`. If 4 fails after `keep()` but before rename, an orphan `.tome.tmp.*` directory remains; the doctor `--fix` orphan-cleanup catches it (Phase 4 follow-up to P3's TD-016). |

**Phase B — harness sync (NO lock held)**:

6. Recompute the effective harness list for the project (FR-441; see [settings-composition.md](./settings-composition.md)).
7. Run the sync algorithm phase B for that effective list (see [sync-algorithm.md](./sync-algorithm.md) §Concurrency — phase B holds no lock).

If phase B fails (e.g. harness clash on an MCP config), the phase-A state remains committed; the command exits with the appropriate sync-error code (19, 7, etc.); doctor reports the harness drift; the developer re-runs `tome harness sync` after addressing the cause.

**Failure modes**:

- `<CWD>` is `<home>` or `/` → exit 2 (usage error) with the specific refusal message.
- `<name>` not found → exit 13.
- Sync step encounters a harness clash → binding has committed; exit 19; doctor reports drift.
- Sync step encounters any I/O failure → binding has committed; exit 7 (Io); doctor reports drift.

## `tome workspace rename <old> <new>`

Renames a workspace.

**Arguments**: `<old>`, `<new>`; both validated against FR-347.

**Algorithm**:

1. Parse `<new>` — exit 15 on failure.
2. Acquire advisory lockfile.
3. Check `<old>` exists, `<new>` does not — exits 13 / 14 respectively.
4. Pre-check: for every `workspace_projects.project_path` bound to `<old>`, verify the directory exists on disk. If any are missing, exit 70 (`WorkspaceMalformed`-class) with no state change.
5. Inside one DB transaction:
   - Update every bound project's `<project>/.tome/config.toml` to name `<new>` (the per-file atomic-write applies; ordered lexicographically by project path).
   - `UPDATE workspaces SET name = '<new>' WHERE name = '<old>'`.
6. Outside the transaction (with the lock still held): rename `<root>/workspaces/<old>/` → `<root>/workspaces/<new>/` using the same-FS atomic rename idiom.

If step 5's transaction fails mid-loop, the transaction rolls back and step 6 does not run. If step 6 fails (e.g. cross-filesystem somehow), the database transaction is already committed — log a hard error and bubble; doctor `--fix` is not safe here (manual recovery).

## `tome workspace remove <name> [--force]`

Removes a workspace.

**Algorithm**:

1. Validate `<name>` is not reserved (`global`). Exit 15 if reserved.
2. Acquire advisory lockfile.
3. Check `workspaces.name = <name>` — exit 13 otherwise.
4. Count bound projects; if ≥ 1 and `--force` not set, exit 16.
5. With `--force` or zero bound projects, run the cascade in this **explicit numbered order** (FR-405):
   1. For each bound project: run the rules-file-block removal and MCP-entry removal logic for every harness in that project's effective list.
   2. Remove each bound project's `.tome/` directory.
   3. Inside one DB transaction: `DELETE FROM workspace_skills WHERE workspace_id = ?`; `DELETE FROM workspace_catalogs WHERE workspace_id = ?`; `DELETE FROM workspace_projects WHERE workspace_id = ?`; `DELETE FROM workspaces WHERE id = ?`. Commit.
   4. Remove `<root>/workspaces/<name>/` directory.
   5. Refcount-clean: enumerate `workspace_catalogs` for the remaining URLs; delete any `<root>/catalogs/<hash>/` whose URL no longer appears.

Failure at any step before (3) leaves the database consistent. Failure at (4) or (5) may leave an orphaned directory; doctor detects and `--fix` cleans.

## `tome workspace sync [<name>]`

Copies the workspace's central RULES.md to every bound project's marker copy.

**Arguments**: `<name>` optional. Without it, syncs every workspace.

**Algorithm**: For each workspace in scope, enumerate `workspace_projects` rows. For each project: read `<root>/workspaces/<name>/RULES.md`; write to `<project>/.tome/RULES.md` if contents differ. Use per-file atomic write. Skip missing project directories with a debug-verbosity log line.

The command is idempotent — re-running it without intervening changes produces no further file writes (no `rename()` syscall).

## `tome workspace regen-summary <name>`

Forces regeneration of a workspace's cached summaries.

**Algorithm**:

1. Check `<name>` exists — exit 13.
2. Read the workspace's enabled plugins + their skill names/descriptions (one query joining `workspace_skills` × `skills`).
3. Construct `PluginSummariesInput` (see [data-model.md §13](../data-model.md)).
4. Invoke the configured `Summariser` (production = `LlamaSummariser`; tests = `StubSummariser`).
5. Write the resulting `SummariserOutput` to the workspace's `settings.toml` `[summaries]` section (atomic file write).
6. Rewrite `<root>/workspaces/<name>/RULES.md` from the long summary.
7. Run `tome workspace sync <name>` automatically.

**Failure modes**: summariser model missing / failed → exit 24; underlying I/O failure → exit 7.

The Phase 3 forward-progress rule (FR-385) does NOT apply here — `regen-summary` is the explicit summarisation command; if it fails, the failure is the result, not a side effect.
