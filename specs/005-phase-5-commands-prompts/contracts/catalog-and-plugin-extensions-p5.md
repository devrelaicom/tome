# Phase 5 — Catalog & plugin command extensions

Authoritative contract for the Phase 5 updates to existing CLI surfaces: `tome plugin enable`, `tome plugin disable`, `tome plugin show`, `tome plugin list`, `tome catalog update`, `tome reindex`.

No new CLI commands. The Phase 4 surface (`tome workspace`, `tome harness`, `tome doctor`) is also unchanged in its top-level shape; Phase 5 only extends behaviour within these existing commands.

## `tome plugin enable`

Per FR-001 + FR-002 + FR-112 + FR-114.

### Behaviour change

The enable pipeline now walks two directories:
- `<plugin>/skills/*/SKILL.md` — kind `skill` (existing).
- `<plugin>/commands/*.md` — kind `command` (new).

Both walks produce `EntryRow`s under the same insertion path. UPSERT keyed on `(catalog, plugin, kind, name)`.

The plugin enable transaction:
1. Parse `plugin.json` (existing).
2. Walk skills directory; for each `SKILL.md`:
   - Parse frontmatter (lenient).
   - Compute `searchable`, `user_invocable` (kind = Skill).
   - Compute `embedding_text` (including `when_to_use` if set).
   - Compute `content_hash`.
   - UPSERT into `skills`; re-embed if hash changed.
3. Walk commands directory; for each `*.md`:
   - Same as above but kind = Command.
4. Synchronise `workspace_skills` junction for the active workspace (both kinds).
5. Trigger workspace summary regeneration (existing Phase 4 trigger; expanded input per FR-114).

### Atomicity

Per Phase 2 + Phase 4 discipline: each plugin's full insert/upsert + workspace junction sync happens under the advisory lock in a single SQLite transaction. Partial enables are not possible.

### Output

Human-mode (extended):

```
Enabled plugin midnight-expert/compact-dev (v1.4.0):
  Skills:   3 added, 0 modified, 0 removed
  Commands: 2 added, 0 modified, 0 removed
  Workspace: midnight-dev — 5 entries enrolled

Regenerating workspace summary…
✓ Workspace summary updated.
```

JSON-mode (extended):

```json
{
  "ok": true,
  "plugin": "midnight-expert/compact-dev",
  "version": "1.4.0",
  "workspace": "midnight-dev",
  "entries": {
    "skills":   { "added": 3, "modified": 0, "removed": 0, "unchanged": 0 },
    "commands": { "added": 2, "modified": 0, "removed": 0, "unchanged": 0 }
  },
  "summariser_triggered": true
}
```

## `tome plugin disable`

Per FR-113. Disenrols both kinds from the active workspace's `workspace_skills` junction. No changes to the underlying `skills` rows (which remain indexed but unbound). Output extended to separate counts for skills vs commands disenrolled.

## `tome plugin show`

Per FR-130. Lists skills and commands separately, each annotated with effective `searchable` / `user_invocable` flags and the derived prompt name.

### Human-mode output

```
Plugin: midnight-expert/compact-dev
Version: 1.4.0
Catalog: midnight-expert

Skills (3):
  compact-circuits     searchable=true  user_invocable=false
    description: Compact zero-knowledge circuit patterns
  compact-witness      searchable=true  user_invocable=false
    description: Patterns for Compact witness implementation
  legacy-context       searchable=false user_invocable=false  [dormant]
    description: Internal scaffolding referenced by other entries

Commands (2):
  fix-issue            searchable=true  user_invocable=true   prompt=midnight_expert__fix_issue
    description: Fix a GitHub issue
    arguments: (none — accepts free-form 'args')
  migrate-component    searchable=true  user_invocable=true   prompt=midnight_expert__migrate_component
    description: Migrate a component from one framework to another
    arguments: [component, from, to]
```

- `searchable=` / `user_invocable=` shown verbatim per resolved flag.
- `prompt=` shown only for `user_invocable=true` entries.
- `[dormant]` annotation when both flags are false.
- `arguments:` shows declared names OR `(none — accepts free-form 'args')` for the catch-all case.

### JSON-mode output

```json
{
  "plugin": "midnight-expert/compact-dev",
  "version": "1.4.0",
  "catalog": "midnight-expert",
  "skills": [
    {
      "name": "compact-circuits",
      "description": "...",
      "when_to_use": "...",
      "searchable": true,
      "user_invocable": false,
      "prompt_name": null
    }
  ],
  "commands": [
    {
      "name": "fix-issue",
      "description": "...",
      "when_to_use": null,
      "searchable": true,
      "user_invocable": true,
      "prompt_name": "midnight_expert__fix_issue",
      "arguments": [],
      "argument_hint": "[issue-number]"
    },
    {
      "name": "migrate-component",
      "description": "...",
      "when_to_use": null,
      "searchable": true,
      "user_invocable": true,
      "prompt_name": "midnight_expert__migrate_component",
      "arguments": ["component", "from", "to"],
      "argument_hint": null
    }
  ]
}
```

`prompt_name` is the derived prompt name AFTER sanitisation, truncation, and collision suffixing — Tome's contribution only (the harness-side `mcp__tome__` prefix is NOT included).

## `tome plugin list`

Behaviour unchanged. Per-plugin row continues to show `(<n> skills)` count; Phase 5 extends to `(<n> skills, <m> commands)` when commands are present.

```
NAME                              VERSION   CATALOG          STATUS    ENTRIES
midnight-expert/compact-dev       1.4.0     midnight-expert  enabled   (3 skills, 2 commands)
midnight-expert/compact-cli-dev   1.4.0     midnight-expert  enabled   (5 skills)
```

## `tome catalog update`

Per-plugin reindex pass now walks both directories. Plugins whose `commands/` directory newly appears or disappears between catalog snapshots produce the appropriate Added/Removed records. No CLI surface change; output extended to count both kinds.

## `tome reindex`

Per FR-004. The per-entry diffing pipeline handles both kinds identically — content-hash diffing, embedding refresh on hash change, removal of deleted entries. No new flags. The per-plugin atomicity (each plugin reindex acquires its own advisory lock, established in Phase 2 / US7) carries forward.

Output extended to report both kinds:

```
Reindex aggregate:
  Plugins reindexed: 4
  Skills:   12 added, 3 modified, 0 removed, 32 unchanged
  Commands:  3 added, 0 modified, 1 removed,  4 unchanged
```

## `tome query`

Per FR-091 + FR-093:
- Results include the `kind` discriminator.
- Ranking unchanged for the skill-only case.
- The query CLI does NOT invoke the substitution layer (CLI surface, not MCP `get_skill`).

Output extended to display kind:

```
$ tome query "fix github issue"
1. midnight-expert/compact-dev/fix-issue            (command)  score 0.91
   Fix a GitHub issue
2. midnight-expert/compact-dev/compact-circuits     (skill)    score 0.42
   Compact zero-knowledge circuit patterns
```

## `tome workspace regen-summary`

Per FR-114. Input expanded to include commands. The summariser input string format extends:

```
Skills:
- compact-circuits: Compact zero-knowledge circuit patterns
- compact-witness: Patterns for Compact witness implementation

Commands:
- fix-issue: Fix a GitHub issue
- migrate-component: Migrate a component from one framework to another
```

`Commands:` block omitted when no command-bearing plugin is enabled (preserves Phase 4 skill-only summary input).

## Exit codes

| Surface | New exit codes |
|---|---|
| `tome plugin enable` | 29 (`InvalidArgumentFrontmatter`) when a frontmatter `arguments` field is malformed |
| `tome plugin show` | 27 (`EntryNotFound`) when plugin doesn't exist in the active workspace |
| `tome catalog update` | 29 same as enable (plugin frontmatter malformed during reindex) |
| `tome reindex` | 29 same as above |

Other failure modes reuse Phase 1–4 codes per existing contracts.

## Tests

| Behaviour | Test |
|---|---|
| `plugin enable` walks both directories | `tests/entry_kind_indexing.rs::plugin_enable_walks_both_directories` |
| `plugin enable` workspace_skills synchronises both kinds | `tests/entry_kind_indexing.rs::enable_synchronises_both_kinds_into_junction` |
| `plugin disable` removes both kinds from workspace | `tests/plugin_disable.rs::disable_removes_both_kinds_from_workspace` |
| `plugin show` annotates kind, searchable, user_invocable, prompt_name | `tests/plugin_show_p5.rs::*` |
| `plugin show` `(dormant)` annotation when both flags false | `tests/plugin_show_p5.rs::dormant_entry_annotated` |
| `plugin list` counts include commands | `tests/plugin_list.rs::counts_include_commands` |
| `catalog update` handles new commands directory | `tests/catalog_update_p5.rs::new_commands_directory_indexed` |
| `reindex` per-kind output | `tests/reindex.rs::per_kind_summary` |
| `query` results include kind | `tests/query.rs::results_include_kind` |
| Summariser input includes commands | `tests/summariser_triggers.rs::input_includes_commands_when_present` |
| JSON wire-shape pin for `plugin show` | `tests/plugin_show_p5_json_shape.rs` |
