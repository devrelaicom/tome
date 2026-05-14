# `tome doctor` — Command Contract

```
tome doctor [--fix] [--verify] [--json]
```

- `--fix` runs the three safe automatic repairs (model re-download, catalog re-clone, schema forward-migration).
- `--verify` re-hashes the primary model artefacts (embedder + reranker) against the registry SHA-256s. Without `--verify`, doctor uses the cheap manifest+size probe (same as `tome status`).
- `--json` emits the structured report instead of the human renderer.

Read-only-by-default diagnostic command. Reports model state, index state, catalog-cache integrity, workspace context, and locally-installed agentic-coding harnesses. With `--fix`, performs the three safe automatic repairs (re-download missing/corrupt models, re-clone broken catalog caches, run pending forward DB migrations). Destructive repairs are never automatic; they are surfaced as suggested commands the developer must run.

## Behaviour

1. Resolve scope (workspace or global; honours the same flags as every command).
2. Build the workspace-context section (same shape as [workspace-info.md](./workspace-info.md)).
3. Check models (embedder + reranker): manifest present, files present, sizes match. Without `--fix`, this is the same `cheap_state` Phase 2 status uses.
4. Open the resolved DB read-only; run `PRAGMA integrity_check`; read meta rows; detect drift.
5. For every catalog in the resolved config: check the on-disk clone (`${XDG_DATA_HOME}/tome/catalogs/<sha-of-url>/`) — exists, is a Git repo (has `.git/`), contains a parseable `tome-catalog.toml` (read leniently — doctor only reports whether parse succeeds, not its content).
6. Probe each well-known harness directory: `~/.claude/`, `~/.codex/`, `~/.cursor/`, `~/.gemini/`, `~/.opencode/`, `~/.continue/`. Existence only — no content reads.
7. Classify per-subsystem findings; compute overall classification.
8. If `--fix`, apply the safe repairs in order; re-classify each affected subsystem.
9. Emit report.

## Output (human)

Healthy:

```
Tome:            0.3.0

Workspace:       /home/user/projects/acme-app  (CWD walk)
  catalogs:      3
  plugins:       12 total, 5 enabled
  skills:        47 indexed
  schema:        v1
  embedder:      bge-small-en-v1.5 1.5

Models:
  embedder       bge-small-en-v1.5 (1.5)     ✓ ok   66.5 MiB
  reranker       bge-reranker-base (base)    ✓ ok   279.3 MiB

Index database:  ✓ ok (integrity ok, 12 plugins enabled, 47 skills indexed, 1.8 MiB)
Schema version:  1
Drift:           none

Catalog caches:
  acme-catalog                       ✓ ok
  shared-toolkit                     ✓ ok
  experimental-plugins               ✓ ok

Detected harnesses:
  ✓ Claude Code     ~/.claude/
  ✓ Cursor          ~/.cursor/
  · Codex           ~/.codex/         (not detected)
  · Gemini CLI      ~/.gemini/        (not detected)
  · OpenCode        ~/.opencode/      (not detected)
  · Continue        ~/.continue/      (not detected)

Overall:         ✓ healthy
```

Degraded (e.g., reranker missing):

```
…

Models:
  embedder       bge-small-en-v1.5 (1.5)     ✓ ok    66.5 MiB
  reranker       bge-reranker-base (base)    ✗ missing

Index database:  ✓ ok (integrity ok, …)

Suggested fixes:
  Re-download the reranker (automatically with --fix):
    tome models download

Overall:         ⚠ degraded
```

Unhealthy (e.g., catalog cache broken):

```
…

Catalog caches:
  acme-catalog                       ✓ ok
  shared-toolkit                     ✗ not a git repo (cache directory exists but lacks .git/)
  experimental-plugins               ✓ ok

Suggested fixes:
  Re-clone the broken catalog cache (automatically with --fix):
    tome catalog update shared-toolkit

Overall:         ✗ unhealthy
```

When `--fix` is passed:

```
…

Suggested fixes:
  Re-download the reranker:
    [running] tome models download
    [done]    reranker installed (279.3 MiB, sha256 verified)

  Re-clone the catalog cache 'shared-toolkit':
    [running] tome catalog update shared-toolkit
    [done]    catalog updated to ref abc1234

Overall:         ✓ healthy (after fixes)
```

Status glyphs:
- `✓` green — healthy
- `⚠` yellow — degraded (non-fatal)
- `✗` red — unhealthy
- `·` dim — informational / not detected

On non-TTY: ASCII fallback (`[ok]`, `[warn]`, `[fail]`, `[—]`).

## Output (`--json`)

```json
{
  "tome_version": "0.3.0",
  "workspace": { /* WorkspaceInfo from workspace-info.md */ },
  "embedder": { "name": "bge-small-en-v1.5", "version": "1.5", "state": "ok", "size_bytes": 66499584 },
  "reranker": { "name": "bge-reranker-base", "version": "base", "state": "missing" },
  "index": { "present": true, "integrity_ok": true, "plugins_enabled": 5, "skills_indexed": 47, "size_bytes": 1887436 },
  "drift": "none",
  "catalogs": [
    { "name": "acme-catalog", "url": "https://github.com/acme/catalog", "cache_path": "…", "state": "ok" },
    { "name": "shared-toolkit", "url": "…", "cache_path": "…", "state": "not_a_repo" }
  ],
  "harnesses": [
    { "name": "claude_code", "path": "/home/user/.claude", "present": true },
    { "name": "codex", "path": "/home/user/.codex", "present": false }
  ],
  "overall": "unhealthy",
  "suggested_fixes": [
    {
      "subsystem": "reranker",
      "diagnosis": "model file missing",
      "command": "tome models download",
      "auto_fixable": true
    },
    {
      "subsystem": "catalog:shared-toolkit",
      "diagnosis": "cache directory is not a git repo",
      "command": "tome catalog update shared-toolkit",
      "auto_fixable": true
    }
  ]
}
```

Schema details in [data-model.md](../data-model.md) §5.

## `--fix` semantics

Three repair classes are auto-applied:

| Subsystem | Repair | Underlying call |
|---|---|---|
| Model missing/corrupt | Re-download | `embedding::download::download_model(entry)` |
| Catalog cache broken | Re-clone | `catalog::git::Git::clone(url, dest, ref)` against the catalog's recorded URL and pinned ref |
| Schema older than expected | Apply pending migrations | `index::migrations::apply_pending(conn, current, target)` |

Each repair runs in order; if a repair fails, doctor records the failure in the report and continues with the next. After all repairs run, every affected subsystem is re-checked and re-classified.

Repairs NOT auto-applied (each surfaces as a "suggested fix" with `auto_fixable: false`):

- Embedder drift / reranker drift — requires `tome reindex --force` which is a deliberate user action.
- Schema newer than expected — requires upgrading Tome; doctor refuses to mutate a newer schema (FR-165).
- Catalog manifest invalid (cache exists, is a git repo, but `tome-catalog.toml` doesn't parse) — manual investigation.
- Orphan catalog clone (a clone on disk with no config reference) — surfaces as cleanup candidate; the developer decides whether to remove.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Overall classification is Ok. |
| 1 | Overall classification is Degraded or Unhealthy. Same code for both; the JSON / structured output identifies which. |
| 2 | Usage error. |
| 7 | I/O error preventing the report from running. |
| 75 | `--fix` was passed with no fixable subsystems (informational), OR `--fix` ran but the developer needs to take a manual action that doctor refuses to automate. The exit code communicates "fix did something, but the work isn't done." |

Note: exit 1 is the report classification; exit 75 only fires when `--fix` is passed and the run completed but un-fixable issues remain.

## Workspace context

Doctor honours the same workspace resolution as every other command. From inside a workspace:

- Workspace section reflects the resolved workspace.
- Model section reports the globally shared models (workspaces share models — FR-134).
- Index section reflects the workspace DB.
- Catalog caches section enumerates the workspace's catalog list (catalog clones themselves are globally shared but their list is per-scope).
- Harness section is unchanged (harness detection is per-user, not per-workspace).

With `--global` from inside a workspace, every section operates on global state and the report notes which workspace was overridden:

```
Workspace:       (global)
  resolved via:  --global flag (overrode /home/user/projects/acme-app via CWD walk)
```

## What `tome doctor` does NOT do

- Does not parse any harness's MCP configuration file or any other harness config. Detection is directory existence only (FR-167).
- Does not read or run any non-Tome command. The suggested fixes are commands the developer (or `--fix`) runs through Tome.
- Does not network unless `--fix` is invoked.
- Does not write to the index unless `--fix` runs a migration.
- Does not take the workspace's advisory lockfile for the read-only report. The migration apply path (with `--fix`) does acquire the lock for the migration transaction.
- Does not modify the global state from inside a workspace (or vice versa) unless `--global` / `--workspace` explicitly redirects.

## Relationship to `tome status`

| | `tome status` | `tome doctor` |
|---|---|---|
| Cost | Fast (~200 ms). | Slower (catalog enumeration, harness probing). |
| Lock-free read | Yes. | Yes (`--fix` may acquire lock for migration only). |
| Network | No, ever. | Only with `--fix` (re-download / re-clone). |
| Coverage | Models + index + drift. | Models + index + drift + workspace + catalogs + harnesses + suggested fixes. |
| `--fix` | Not available. | Available. |
| Audience | Pre-flight inside scripts / CI. | "Something is wrong, what do I do?" |

Both exist. Status is the narrow fast path; doctor is the broad slower path. Doctor's classification rules match status's where they overlap (embedder failure → Unhealthy in both; reranker-only drift → Degraded in both).
