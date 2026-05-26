# Phase 5 — Quickstart

End-to-end smoke test for the Phase 5 deliverable. Exercises every user story in one walkthrough. Run against a clean Tome install with a fresh `<home>/.tome/` directory.

## Prerequisites

- Tome built from Phase 5 branch (`005-phase-5-commands-prompts`).
- Models downloaded: `tome models download embedder`, `tome models download reranker`, `tome models download summariser`.
- A test plugin tree containing both `skills/` and `commands/` directories with substitution-bearing entries (see §Test plugin fixture below).

## Test plugin fixture

`tests/fixtures/sample-plugin-catalog/phase5-plugin/` should contain:

```
phase5-plugin/
├── plugin.json                  # name: phase5-demo, version: 1.0.0
├── skills/
│   ├── circuits/
│   │   ├── SKILL.md             # default skill (searchable; not user_invocable)
│   │   └── scripts/audit.py     # referenced via ${TOME_SKILL_DIR}
│   └── slash-skill/
│       └── SKILL.md             # frontmatter: user-invocable: true
├── commands/
│   ├── fix-issue.md             # no arguments
│   ├── migrate-component.md     # arguments: [component, from, to]
│   ├── private-command.md       # frontmatter: disable-model-invocation: true
│   └── overridden.md            # frontmatter: prompt_name: short-name
```

Sample `commands/migrate-component.md` body:

```markdown
---
description: Migrate a component from one framework to another
arguments:
  - component
  - from
  - to
---

Migrate the $component component from $from to $to.

Reference the audit script at ${TOME_PLUGIN_DIR}/skills/circuits/scripts/audit.py.
Plugin data lives at ${TOME_PLUGIN_DATA}; workspace data at ${TOME_WORKSPACE_DATA}.
Deployment target: ${TOME_ENV_DEPLOY_TARGET:-staging}.
The current date is ${TOME_DATE}.
```

Sample `commands/fix-issue.md` body:

```markdown
---
description: Fix a GitHub issue
argument-hint: "[issue-number]"
---

Fix GitHub issue $ARGUMENTS following our coding standards.
```

(No `$ARGUMENTS` placeholder in body → triggers the append-fallback footer when called with args.)

## Walkthrough

### 1. Initialise workspace and enable plugin

```bash
$ tome workspace init phase5-test
✓ Created workspace `phase5-test` at <home>/.tome/workspaces/phase5-test/

$ tome catalog add file:///path/to/sample-plugin-catalog --workspace phase5-test
✓ Cloned catalog. 1 plugins discovered.

$ tome plugin enable phase5-demo --workspace phase5-test
Enabled plugin sample-plugin-catalog/phase5-demo (v1.0.0):
  Skills:   2 added, 0 modified, 0 removed
  Commands: 4 added, 0 modified, 0 removed
  Workspace: phase5-test — 6 entries enrolled
Regenerating workspace summary…
✓ Workspace summary updated.
```

**Verifies**: FR-001 (both directories indexed), FR-112 (workspace_skills syncs both kinds), FR-114 (summariser input expanded), SC-007 (Phase 4 rows preserved if any).

### 2. Inspect entries via `plugin show`

```bash
$ tome plugin show phase5-demo --workspace phase5-test
Plugin: sample-plugin-catalog/phase5-demo
Version: 1.0.0
Catalog: sample-plugin-catalog

Skills (2):
  circuits             searchable=true  user_invocable=false
    description: Compact zero-knowledge circuit patterns
  slash-skill          searchable=true  user_invocable=true  prompt=phase5_demo__slash_skill
    description: A skill that's also slash-invocable

Commands (4):
  fix-issue            searchable=true  user_invocable=true  prompt=phase5_demo__fix_issue
    description: Fix a GitHub issue
    arguments: (none — accepts free-form 'args')
  migrate-component    searchable=true  user_invocable=true  prompt=phase5_demo__migrate_component
    description: Migrate a component from one framework to another
    arguments: [component, from, to]
  overridden           searchable=true  user_invocable=true  prompt=short-name
    description: An entry with a prompt_name override
  private-command      searchable=false user_invocable=true  prompt=phase5_demo__private_command
    description: A user-invocable command hidden from agent search
```

**Verifies**: FR-130 (annotations), FR-063 (override replaces both portions), FR-013 (`searchable=false` from `disable-model-invocation: true`).

### 3. Bind a project and configure harness

```bash
$ cd /tmp/test-project
$ tome workspace use phase5-test
✓ Bound /tmp/test-project to workspace `phase5-test`.
✓ Synced harness configs: claude-code.
```

**Verifies**: Phase 4 binding workflow remains intact.

### 4. Launch harness; verify prompts surface

(Manual step — outside the smoke-test script.)

Open Claude Code in `/tmp/test-project`. Type `/`. Expected behaviour:

- Slash menu shows:
  - `/mcp__tome__phase5_demo__slash_skill` (slash-skill — user-invocable skill)
  - `/mcp__tome__phase5_demo__fix_issue`
  - `/mcp__tome__phase5_demo__migrate_component`
  - `/mcp__tome__short-name` (prompt_name override)
  - `/mcp__tome__phase5_demo__private_command` (hidden from agent search but slash-invocable)
- Slash menu does NOT show:
  - `circuits` skill (default `user_invocable=false` for skills)

**Verifies**: FR-060 (prompts capability declared), FR-061 (prompts/list shape), FR-012 (default per-kind behaviour), FR-063 (override).

### 5. Invoke a command via slash menu

In the harness: type `/mcp__tome__phase5_demo__migrate_component`. The harness UI offers three structured input fields: `component`, `from`, `to`. Fill in `SearchBar`, `React`, `Vue`. Submit.

Expected body that lands in the conversation (assuming `TOME_ENV_DEPLOY_TARGET` is unset; today's date is 2026-05-26):

```
Migrate the SearchBar component from React to Vue.

Reference the audit script at /Users/<you>/.tome/catalogs/<sha>/phase5-plugin/skills/circuits/scripts/audit.py.
Plugin data lives at /Users/<you>/.tome/plugin-data/sample-plugin-catalog/phase5-demo; workspace data at /Users/<you>/.tome/workspaces/phase5-test/plugin-data/sample-plugin-catalog/phase5-demo.
Deployment target: staging.
The current date is 2026-05-26.
```

**Verifies**:
- FR-040 (named-argument substitution: `$component` → "SearchBar").
- FR-020 (`${TOME_PLUGIN_DIR}`, `${TOME_PLUGIN_DATA}`, `${TOME_WORKSPACE_DATA}` resolved).
- FR-021 (data dirs created on disk after first invocation — verifiable via `ls`).
- FR-031 (`${TOME_ENV_DEPLOY_TARGET:-staging}` resolves to "staging" with default).
- FR-020 (`${TOME_DATE}` resolves).
- SC-001 (end-to-end demonstrates the full slash-menu invocation flow).

### 6. Invoke an arg-less command with arguments

In the harness: type `/mcp__tome__phase5_demo__fix_issue 123`. The harness sends `args: "123"` to `prompts/get`.

Expected body:

```
Fix GitHub issue 123 following our coding standards.

ARGUMENTS: 123
```

**Verifies**:
- FR-040 (`$ARGUMENTS` substitution: "123").
- FR-044 (append-fallback NOT triggered because `$ARGUMENTS` consumed the arg).

If the body had no `$ARGUMENTS` reference: the body would end with `ARGUMENTS: 123` instead (append-fallback footer).

### 7. Inspect via the middle-tier tool

(Programmatic check from a test agent.)

Call `get_skill_info` for the `circuits` skill:

```json
{
  "catalog": "sample-plugin-catalog",
  "plugin": "phase5-demo",
  "name": "circuits",
  "kind": "skill"
}
```

Expected response:

```json
{
  "catalog": "sample-plugin-catalog",
  "plugin": "phase5-demo",
  "name": "circuits",
  "kind": "skill",
  "path": "/Users/<you>/.tome/catalogs/<sha>/phase5-plugin/skills/circuits/SKILL.md",
  "description": "<full untruncated>",
  "when_to_use": "...",
  "plugin_version": "1.0.0",
  "user_invocable": false,
  "resources": {
    "files": [],
    "directories": {
      "scripts": [
        "/Users/<you>/.tome/catalogs/<sha>/phase5-plugin/skills/circuits/scripts/audit.py"
      ]
    }
  }
}
```

**Verifies**: FR-080–085, SC-004 (response is small relative to body).

Call `get_skill_info` for the `fix-issue` command:

```json
{
  "catalog": "sample-plugin-catalog",
  "plugin": "phase5-demo",
  "name": "fix-issue",
  "kind": "command"
}
```

Expected response: same shape minus `resources` field (FR-083).

### 8. Search excludes `disable-model-invocation: true` entries

Call `search_skills`:

```json
{ "query": "user-invocable command hidden", "top_k": 5 }
```

Expected: `private-command` (which has `disable-model-invocation: true`) does NOT appear in results, regardless of query semantic match. All other entries are searchable.

**Verifies**: FR-010, FR-090.

Description truncation: each result's `description` field is exactly 150 characters (default) OR full untruncated if shorter. Override via `description_max_chars: 1000` returns full descriptions.

**Verifies**: FR-092.

### 9. Inspect via `tome doctor`

```bash
$ tome doctor --workspace phase5-test
✓ Embedder, reranker, summariser: healthy
✓ Index integrity: OK
✓ Workspace binding: OK
✓ Harness rules + MCP: OK

Phase 5 prompts surface (4 prompts, 0 collisions):
  phase5-demo:
    /mcp__tome__phase5_demo__slash_skill         skill
    /mcp__tome__phase5_demo__fix_issue           command
    /mcp__tome__phase5_demo__migrate_component   command
    /mcp__tome__short-name                       command (override)
    /mcp__tome__phase5_demo__private_command     command (searchable=false)

Entry counts:
  Skills: 2, Commands: 4, Pending re-embedding: 0

Orphan persistent data directories: none.
```

**Verifies**: FR-120 (prompts enumeration), FR-121 (collision report — empty here), FR-122 (orphan report), FR-123 (entry counts), FR-124 (read-only — directories already exist, doctor doesn't create new ones).

### 10. Trigger collision detection

Enable a second plugin (e.g. `phase5-plugin-2/`) that ships a command named `fix-issue` in its `commands/` directory.

```bash
$ tome plugin enable phase5-demo-2 --workspace phase5-test
✓ ...
WARN collision_resolved derived_name="phase5_demo__fix_issue" entries=[...]
```

Restart the harness and re-run `tome doctor`:

```
Phase 5 prompts surface (8 prompts, 1 collision):
  ...
  /mcp__tome__phase5_demo__fix_issue           command
  /mcp__tome__phase5_demo_2__fix_issue         command  (sanitised would have collided with phase5-demo — distinct due to plugin portion)

Collisions: none
```

Then enable a third plugin whose `prompt_name: phase5_demo__fix_issue` explicitly collides:

```
WARN collision_resolved derived_name="phase5_demo__fix_issue" entries=[
  { ..., final_name: "phase5_demo__fix_issue" },
  { ..., final_name: "phase5_demo__fix_issue2" }
]
```

**Verifies**: FR-062 (counter suffixing), FR-121 (doctor surfaces collisions), SC-012.

### 11. Trigger orphan detection

```bash
$ tome plugin disable phase5-demo --workspace phase5-test
✓ ...

$ ls <home>/.tome/plugin-data/sample-plugin-catalog/
phase5-demo/      # still on disk

$ tome doctor --workspace phase5-test
...
Orphan persistent data directories:
  plugin-data (no longer enabled in any workspace):
    /Users/<you>/.tome/plugin-data/sample-plugin-catalog/phase5-demo/

  workspace-data:
    /Users/<you>/.tome/workspaces/phase5-test/plugin-data/sample-plugin-catalog/phase5-demo/

Cleanup: not auto-fixable in Phase 5. Manual rm -rf required.
```

**Verifies**: FR-122 (orphan informational only).

### 12. Confirm secrets don't leak

Add to a skill body: `${GITHUB_TOKEN}`, `${AWS_SECRET_ACCESS_KEY}`. Set those env vars on the harness via `claude mcp add --env`. Trigger a `get_skill` against that entry.

Expected: the returned body contains the literal strings `${GITHUB_TOKEN}` and `${AWS_SECRET_ACCESS_KEY}` — verbatim, NOT substituted. No host env value appears in the response or in Tome's logs.

**Verifies**: FR-033, NFR-005, SC-010.

### 13. Confirm shell-exec syntax passes through

Add to a skill body: `` Use !`gh pr diff` to inspect the PR. ``. Trigger a `get_skill`.

Expected: the returned body contains the literal string `` !`gh pr diff` `` — verbatim, NOT executed. No `gh` invocation in Tome's process tree.

**Verifies**: FR-052, SC-011.

## Success checklist

All of the following hold after the walkthrough:

- [ ] **SC-001**: Slash menu shows commands without per-plugin config.
- [ ] **SC-002**: Path references resolve correctly on this machine; same skill on a second machine resolves to that machine's paths.
- [ ] **SC-003**: `migrate-component` runs from the harness slash menu with structured args → body substituted correctly.
- [ ] **SC-004**: `get_skill_info` response for `circuits` is much smaller than `get_skill` response for the same.
- [ ] **SC-005**: Flipping `user-invocable: true` on `circuits` and reindexing makes it appear in the next harness session's slash menu; flipping it back removes it.
- [ ] **SC-006**: Codex connected to this Tome works for search + read (no slash menu for it).
- [ ] **SC-007**: All Phase 4 plugin rows preserved at the same identity; Phase 5 command rows are new.
- [ ] **SC-008**: Doctor's prompts list matches the harness's slash menu (including counter-suffixed collisions).
- [ ] **SC-009**: Workspace summary mentions command topics when commands are enabled.
- [ ] **SC-010**: `${GITHUB_TOKEN}` in a skill body passes through verbatim.
- [ ] **SC-011**: `` !`gh pr diff` `` in a skill body passes through verbatim.
- [ ] **SC-012**: Two enabled plugins with colliding entry names get counter-suffixed prompt names; doctor surfaces the collision.
- [ ] **SC-013**: Every Phase 5 MCP response shape and diagnostic record has a byte-stable serialisation pin test in the repo.

## Failure modes to exercise

| Failure | Expected exit code | Manual reproduction |
|---|---|---|
| Entry not found | 27 | `get_skill` with a name not in the workspace's enabled set |
| Substitution failure (data-dir permission denied) | 25 | `chmod 0000 <home>/.tome/plugin-data/`; trigger any substitution |
| Invalid argument frontmatter | 29 | Author a plugin with `arguments: 5` (integer) and enable |
| Prompt argument count exceeds | 26 | Call `prompts/get migrate-component` with `args: {component, from, to, fourth}` |

## Notes

- Phase 5 NO LONGER pads `description` in `search_skills` results unconditionally. Callers who relied on full descriptions in Phase 4 should pass `description_max_chars: 99999` OR switch to `get_skill_info` for full text.
- The MCP server caches the prompt registry at startup. Workspace switches, plugin enables/disables, and reindexes do NOT update the running server's prompt list mid-session. The harness picks up changes on next launch (per NFR-008).
