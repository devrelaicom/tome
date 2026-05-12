# `tome plugin` — Command Contracts

Covers `tome plugin enable | disable | list | show` and the no-subcommand interactive form.

Global flags from Phase 1 apply to every form: `--json`, `--no-color`, `-v` / `-vv`. Errors go to stderr; primary output to stdout.

---

## `tome plugin enable <catalog>/<plugin>`

### Synopsis

```
tome plugin enable <catalog>/<plugin>
                  [--json]
```

### Behaviour

1. Resolve `<catalog>/<plugin>` against the catalog registry; if catalog is unknown → exit 3 (Phase 1 `CatalogNotFound`). If plugin directory does not exist → exit 20 (`PluginNotFound`).
2. Read `${catalog cache}/<plugin>/.claude-plugin/plugin.json`. Parse leniently. If syntactically invalid or missing required identity → exit 22 (`PluginManifestParseError`).
3. If the plugin is already enabled in the index → exit 21 (`PluginAlreadyInState`).
4. Check that the embedder and reranker models are installed; if either is missing:
   - TTY (stdout AND stderr): prompt to download with size + licence shown. Default yes.
   - Non-TTY: exit 30 (`ModelMissing`) with a message pointing at `tome models download`.
5. Acquire the index advisory lock. On contention → exit 50 (`IndexBusy`).
6. Open a SQLite transaction.
7. Walk `${plugin path}/skills/*/SKILL.md`. For each:
   - Parse frontmatter leniently. Apply FR-011 / FR-012 fallbacks; warn (stderr) on each fallback.
   - On syntactically invalid frontmatter → exit 23 (`SkillFrontmatterParseError`) but ONLY if the failure is in the frontmatter delimiters themselves; per FR-013c a malformed body of the header is logged and the skill is skipped without aborting the enable. Implementation distinguishes "header could not be located" (exit) from "header located but YAML body invalid" (skip).
   - Compute `content_hash` (research R8 / data-model §9).
   - If a matching row exists with the same `content_hash`: skip embedding, flip `enabled=1`.
   - Otherwise: run the embedder, INSERT or UPDATE the row, INSERT into `skill_embeddings`. On embedder failure → roll back transaction, exit 36 (`EmbeddingGenerationFailure`).
8. COMMIT the transaction. Release the lock.
9. Report: `Enabled <catalog>/<plugin>: <N> skills indexed (<M> newly embedded).`

### Output (human)

```
Enabling midnight-experts/compact-expert…
✓ 12 skills indexed (12 newly embedded) in 8.4s
```

### Output (`--json`)

```json
{
  "plugin": "midnight-experts/compact-expert",
  "status": "enabled",
  "skills_indexed": 12,
  "skills_newly_embedded": 12,
  "duration_ms": 8412
}
```

### Atomicity

The entire transaction runs inside one SQLite `BEGIN…COMMIT`. SIGINT at any skill boundary rolls back. On a partial embedder failure, the plugin remains disabled. No half-indexed state is ever observable.

---

## `tome plugin disable <catalog>/<plugin>`

### Synopsis

```
tome plugin disable <catalog>/<plugin>
                   [--force]
                   [--json]
```

### Behaviour

1. Resolve the plugin (same as enable; exit 20 on unknown).
2. If the plugin is already disabled → exit 21.
3. Unless `--force`, prompt for confirmation (`Disable <id>? [y/N]`, default no). In a non-TTY context without `--force` → exit 54 (`NotATerminal`).
4. Acquire the index lock. On contention → exit 50.
5. `UPDATE skills SET enabled = 0 WHERE catalog = ? AND plugin = ?` inside one transaction.
6. Release the lock. Report.

### Output (human)

```
Disabled midnight-experts/compact-expert (12 skill records retained).
```

### Output (`--json`)

```json
{
  "plugin": "midnight-experts/compact-expert",
  "status": "disabled",
  "skills_retained": 12
}
```

Embeddings are kept on disk so re-enable is cheap (FR-005, FR-006).

---

## `tome plugin list`

### Synopsis

```
tome plugin list
               [--catalog <name>]
               [--enabled-only]
               [--json]
```

### Behaviour

Walk every registered catalog (or just one when `--catalog` is given), enumerate plugin directories, and join with index state. No DB write.

### Output (human, default — `comfy-table`)

```
Catalog            Plugin              Version  Status     Skills  Last indexed
─────────────────  ──────────────────  ───────  ─────────  ──────  ────────────
midnight-experts   compact-expert      1.2.0    ✓ enabled       12  2h ago
midnight-experts   ledger-helper       0.4.1    ✗ disabled       8  3d ago
midnight-experts   broken-plugin       —        ⚠ unindexable    —  —
```

- `✓` is green, `✗` is dim red, `⚠` is yellow. `NO_COLOR` removes the colours; the glyphs remain.
- Sort order: catalog name asc, then plugin name asc.

### Output (`--json`)

Array of `PluginRecord` JSON records (see data-model §2).

---

## `tome plugin show <catalog>/<plugin>`

Rich plugin view (the same content as the interactive flow's step 3).

### Output (human)

```
Plugin:       midnight-experts/compact-expert
Version:      1.2.0
Status:       ✓ enabled (last indexed 2 hours ago)
Last updated: 3 days ago — Alice <alice@example.com>
Description:  An expert on writing Compact smart contracts on Midnight.

Component breakdown:
  Component    Count
  ───────────  ─────
  Skills          12
  Agents           2
  Commands         5
  Hooks            1
  MCP servers      0
```

Status colours per `tome plugin list`. Last-updated thresholds: green ≤ 7d, yellow ≤ 30d, red older.

### Output (`--json`)

A single `PluginRecord` JSON object.

### Errors

- Unknown catalog → exit 3
- Unknown plugin → exit 20
- Plugin manifest unparsable → exit 22

---

## `tome plugin` (no subcommand — interactive)

### Behaviour

1. Refuse without TTY (FR-051): exit 54 with "This command requires a terminal. Try `tome plugin list` or `tome plugin show <id>`."
2. Loop:
   - Show catalog selector (`inquire::Select`). "Quit" trailing entry exits.
   - Show plugin browser for selected catalog. "Back" trailing entry goes up one level.
   - Show plugin view (same as `plugin show`).
   - Action prompt: Enable / Disable (whichever applies) / Back.
   - On Enable: run the same flow as `plugin enable` with progress bars; on completion, redraw plugin view.
   - On Disable: prompt to confirm, then run `plugin disable --force` equivalent; redraw.
3. Exit cleanly on Quit / EOF / SIGINT.

### Exit

Always 0 on a clean exit. Errors during enable/disable propagate the same exit codes as the non-interactive forms.

### Constraints

- Every level has Back / Quit.
- `inquire` handles arrow-key navigation, terminal resize, and Ctrl-C consistently.
- The view never erases the catalog or plugin list scrollback; instead, redraws at the current cursor.

---

## Common error mapping

| Trigger | Exit |
|---|---|
| Catalog not found | 3 |
| Plugin not found | 20 |
| Plugin already enabled / disabled | 21 |
| `plugin.json` malformed | 22 |
| Skill header malformed | 23 |
| Embedder OOM / refusal | 36 |
| Reranker refusal | 37 |
| Index busy | 50 |
| Index integrity check failure | 51 |
| Non-TTY interactive | 54 |
