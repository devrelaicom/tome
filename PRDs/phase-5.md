# Tome — Phase 5 PRD

## Overview

Phase 5 brings plugin commands into Tome's indexing pipeline alongside skills,
unifying them as a single "entry" concept with kind metadata, and introduces
MCP prompts as the primary user-invocation surface for entries that warrant
it. The phase also adds a richer variable-substitution layer covering both
Claude Code-compatible argument syntax and a new Tome-namespaced variable
system that gives skill authors a portable way to reference paths, plugin
metadata, persistent state directories, and environment variables.

After Phase 5, Tome's MCP server exposes three discovery tools
(`search_skills`, `get_skill_info`, `get_skill`) plus N prompts (one per
user-invocable entry), all backed by a unified central index. Skills and
commands become functionally equivalent — markdown bodies with frontmatter
— differing only in their default invocation patterns.

Agents (subagents) and hooks are still deferred. Subagents have no clean
cross-harness mapping and are likely permanent out-of-scope. Hooks are
Phase 6 work.

## Goals

1. Treat commands as first-class entries alongside skills. Scan
   `plugin/commands/*.md` and `plugin/skills/*/SKILL.md` into the same
   table with a `kind` discriminator.
2. Honour Claude Code's `disable-model-invocation` and a new
   `user-invocable` frontmatter flag for fine-grained per-entry control
   over search and prompt exposure.
3. Expose entries as MCP prompts when `user_invocable = 1`, mapping
   Claude Code-style argument frontmatter to MCP prompt argument schemas.
4. Add a third MCP tool, `get_skill_info`, between `search_skills` and
   `get_skill` for cheap mid-tier candidate inspection.
5. Implement portable variable substitution across all returned entry
   content, covering argument substitution (existing Claude Code semantics),
   Tome-namespaced built-ins (paths, metadata, persistent storage), and
   environment variable passthrough with default-value syntax.
6. Index `when_to_use` frontmatter alongside `description` for embedding,
   improving retrieval quality.

## Non-goals (Phase 5)

- Subagents (commands' close cousins) — no cross-harness mapping
- Hooks — Phase 6
- Server-side shell command execution (` !``cmd`` ` syntax) — Phase 6+
- New harnesses beyond Phase 4's five
- Plugin authoring tools (`tome plugin new`, validators, scaffolding)
- HTTP / SSE MCP transport
- A full templating engine (Tera, MiniJinja, etc.) for entry content —
  hand-rolled substitution covers Phase 5 needs without the dependency
  or syntactic ceremony

## Mental model

Commands and skills share their structural anatomy: markdown body with
YAML frontmatter. Claude Code distinguishes them by directory (`skills/`
vs `commands/`); Tome distinguishes them by the `kind` column on the
entries table. Beyond that, they behave identically in the indexing
pipeline, the MCP search surface, and the substitution engine. Their
default invocation pattern differs: skills default to agent-invocable
(searchable), commands default to user-invocable (exposed as MCP prompts)
plus searchable.

Two orthogonal flags govern this:

| Frontmatter | Default for `skills/` | Default for `commands/` | Effect when `true` |
|---|---|---|---|
| `disable-model-invocation` | `false` | `false` | Entry is NOT indexed for `search_skills`; agent can't find it |
| `user-invocable` | `false` | `true` | Entry IS exposed as an MCP prompt; user can invoke via slash |

Both can be overridden in frontmatter. So:

- Default skill: searchable, not user-invocable (current behaviour)
- Default command: searchable AND user-invocable
- Skill with `user-invocable: true`: also slash-invocable
- Command with `disable-model-invocation: true`: prompt-only, hidden
  from search
- Either with both flags set to disable: indexed in DB but invisible
  through both surfaces — effectively dormant. Acceptable; plugin authors
  may have other reasons to ship them.

Tome never executes the markdown. Substitution applies to the body
deterministically (no LLM involvement), then the rendered body is
returned to whichever surface called for it — the agent via `get_skill`,
or the harness via `prompts/get`.

## Frontmatter spec

For both skills and commands, Tome reads:

| Field | Required | Notes |
|---|---|---|
| `name` | no | Falls back to filename stem (sanitised). Debug-logged. |
| `description` | no | Falls back to first 500 chars of body. Debug-logged. |
| `when_to_use` | no | Disambiguation hint for the agent. Indexed alongside `description` for embedding; surfaced in `get_skill_info`. |
| `arguments` | no | Named positional arguments (YAML list or space-separated string). Drives prompt arg schema. |
| `argument-hint` | no | Display hint string. Used as description on the catch-all `args` parameter when no named arguments are declared. |
| `disable-model-invocation` | no | Boolean. When `true`, sets `searchable = 0`. |
| `user-invocable` | no | Boolean. Default depends on entry kind. When `true`, entry exposed as MCP prompt. |
| `prompt_name` | no | Override for the generated MCP prompt name. Replaces both the plugin prefix and the entry portion with a single chosen string. Still subject to sanitisation and length limits. |

Other frontmatter fields are read but unused in v1 (e.g., Claude Code's
`allowed-tools`, `agent`, `context`). They remain available for downstream
processing by harnesses that recognise them.

## Variable substitution

The substitution layer runs on entry content before it's returned from
`get_skill` (when args are passed) and before every `prompts/get`
invocation. Pure deterministic string rewriting — no LLM involvement,
no recursive expansion of substituted content.

### Built-in variables

Twelve built-ins. All `${TOME_*}`-prefixed, uppercase with underscores
to match conventional env var style:

| Variable | Value |
|---|---|
| `${TOME_SKILL_DIR}` | Absolute path to the directory containing this entry's file (read-only) |
| `${TOME_SKILL_PATH}` | Absolute path to this entry's file (read-only) |
| `${TOME_SKILL_NAME}` | This entry's name |
| `${TOME_PLUGIN_DIR}` | Absolute path to the plugin's root directory (read-only) |
| `${TOME_PLUGIN_NAME}` | Plugin identifier |
| `${TOME_PLUGIN_VERSION}` | Plugin version |
| `${TOME_PLUGIN_DATA}` | Absolute path to persistent plugin storage. Writable. Shared across workspaces. Path: `~/.tome/plugin-data/<catalog>/<plugin>/`. Created lazily on first reference. |
| `${TOME_CATALOG_NAME}` | Catalog name |
| `${TOME_WORKSPACE_NAME}` | Active workspace name |
| `${TOME_WORKSPACE_DATA}` | Absolute path to persistent workspace-scoped plugin storage. Writable. Per workspace, per plugin. Path: `~/.tome/workspaces/<workspace>/plugin-data/<catalog>/<plugin>/`. Created lazily on first reference. |
| `${TOME_DATE}` | Current date in `YYYY-MM-DD` format (local time, matches `date` command behaviour) |
| `${TOME_TIMESTAMP}` | Current timestamp in ISO 8601 format with local offset (e.g., `2026-05-22T14:40:36+01:00`) |

The two `_DATA` variables follow Claude Code's `${CLAUDE_PLUGIN_DATA}`
pattern — directories that survive plugin version upgrades and serve as
plugin-managed persistent state areas. Tome `mkdir -p`s them on first
substitution; subsequent calls are idempotent.

Naming conventions enforced:
- Catalog and plugin names sanitised for cross-platform path safety
  (replace illegal Windows chars with `_`, keep readable)
- Path components stay stable across plugin version upgrades

Cleanup of `_DATA` directories is out of scope for Phase 5. They persist
indefinitely. Phase 6+ may add `tome doctor` orphan detection,
`tome plugin remove --purge`, etc.

### Environment variable passthrough

Skill authors can reference user-supplied environment variables via the
`${TOME_ENV_*}` namespace:

- Tome reads the env var of the literal name `TOME_ENV_<NAME>` via
  `std::env::var`
- Match the regex `\$\{TOME_ENV_([A-Z0-9_]+)(?::-(.*?))?\}`
- If set: substitute the value
- If unset and a default is supplied (`${TOME_ENV_X:-default}`): substitute
  the default
- If unset and no default: substitute empty string, debug-log

The `TOME_ENV_` prefix is the user-controlled opt-in. The MCP server
inherits the parent harness's full environment by default — which on most
systems means every shell env var, including secrets. Restricting
substitution to `TOME_ENV_*` ensures plugin authors can't accidentally or
maliciously exfiltrate `${GITHUB_TOKEN}`, `${AWS_SECRET_ACCESS_KEY}`, etc.
To expose a value to skills, users explicitly rename or alias it in their
per-harness MCP config:

```json
{
  "command": "tome",
  "args": ["mcp", "--workspace", "midnight-dev"],
  "env": {
    "TOME_ENV_HARNESS": "claude-code",
    "TOME_ENV_DEPLOY_TARGET": "staging"
  }
}
```

Plugin authors document the env vars their skills expect; users wire them
in. Same model as any tool with config.

Default value syntax (`${VAR:-default}`) is supported on both env
passthrough AND built-ins. Built-ins should always be set so the default
won't trigger in practice, but supporting it uniformly is cheaper than
carving exceptions.

### Argument substitution

Existing Claude Code-compatible rules from earlier phases, applied after
built-ins and env passthrough:

| Pattern | Substitute with |
|---|---|
| `$ARGUMENTS[N]` (integer N) | Nth positional value (0-indexed); empty string if out of range |
| `$N` (single integer) | Same as `$ARGUMENTS[N]` |
| `$ARGUMENTS` | All positional values joined by single space |
| `$<name>` (named) | Named arg value; empty string if not provided |

Building positional and named values from caller args:

- If caller passes a single string `args: "foo bar baz"`: positional =
  `["foo bar baz"]` as a single element (matches Claude Code's
  whole-string behaviour for `$ARGUMENTS`). If the entry declares named
  `arguments`, the string is split on whitespace and zipped with the
  declared names.
- If caller passes an object with named keys: named values come from the
  object directly. Positional values are built from named values in the
  order declared by the `arguments` frontmatter list.
- If no named arguments are declared and an object is passed: read the
  `args` key, treat as single-string case.

`ARGUMENTS:` append fallback applies per Claude Code's documented
behaviour: if args are provided AND the body has no substitution
references (no `$ARGUMENTS`, no `$<name>`, etc.), append `ARGUMENTS: <value>`
to the rendered body. Ensures the agent always sees what the user passed,
regardless of whether the entry author thought to reference it.

### Substitution pipeline order

For any entry body being rendered:

1. **Built-ins** (`${TOME_*}`): substitute all matches; remaining
   unmatched `${TOME_*}` left as-is with debug log
2. **Env passthrough** (`${TOME_ENV_*}`): substitute via env lookup with
   default-value support
3. **Argument substitution** (`$ARGUMENTS` family): substitute against
   caller-provided args, treating arg values as opaque strings (no
   recursive expansion)
4. **`ARGUMENTS:` append fallback**: append if args were passed but no
   substitution references consumed them
5. **Leave untouched**: any `${CLAUDE_*}` (downstream Claude Code may
   process), any other `${VAR}` not in the Tome namespace, any
   `` !`cmd` `` shell exec syntax (deferred to Phase 6)

Substitution runs once over the body. Arg values inserted in step 3 are
NOT scanned for further `${...}` expansion. Matches Claude Code's
documented "substitution runs once over the original file" semantic.

### What we deliberately don't support in Phase 5

- **`${TOME_HARNESS}`** built-in: skills shouldn't branch on harness.
  Users who actually need it can wire `TOME_ENV_HARNESS` per harness MCP
  config.
- **Shell command execution** (` !``cmd`` `): defer to Phase 6. Security
  surface (sandboxing, `allowed-tools` enforcement, user consent UX) is
  large enough to deserve its own phase. Skill content with shell syntax
  passes through unchanged; harnesses that natively support it (Claude
  Code, Gemini CLI) handle it on their side. Skill authors needing
  cross-harness dynamic context bundle scripts and invoke via
  `${TOME_SKILL_DIR}` instead.
- **Recursive substitution**: arg values can contain `${...}` literals;
  they're not re-scanned.
- **Templating constructs** (conditionals, loops, filters): a real
  templating engine would offer them, but at the cost of dependency
  weight and learning surface. Hand-rolled substitution is enough.

## Argument schema generation

When an entry is exposed as an MCP prompt, Tome derives the prompt's
argument schema from frontmatter:

1. **Named arguments declared** (`arguments: [component, from, to]`): each
   name becomes a required string argument:
   ```json
   {
     "name": "midnight-expert__migrate-component",
     "description": "Migrate a component from one framework to another",
     "arguments": [
       { "name": "component", "required": true },
       { "name": "from",      "required": true },
       { "name": "to",        "required": true }
     ]
   }
   ```

2. **No named arguments**: single optional `args` argument. The
   `argument-hint` frontmatter (if present) becomes the `description`:
   ```json
   {
     "name": "midnight-expert__fix-issue",
     "description": "Fix a GitHub issue",
     "arguments": [
       {
         "name": "args",
         "description": "Issue number or freeform input",
         "required": false
       }
     ]
   }
   ```

Even commands without `arguments`, `argument-hint`, or `$ARGUMENTS`
references in the body get the catch-all `args` argument — matches
Claude Code's "always accept args, append `ARGUMENTS:` if not consumed"
behaviour.

`prompts/get` returns the rendered markdown as a single user message:

```json
{
  "messages": [
    {
      "role": "user",
      "content": { "type": "text", "text": "<substituted body>" }
    }
  ]
}
```

The substituted markdown becomes the user's effective message to the
agent — slash invocation injects the rendered content as if the user
typed it.

## Database schema changes

Existing `skills` table from Phase 2/4 gains four columns and a widened
unique constraint:

```sql
ALTER TABLE skills ADD COLUMN kind TEXT NOT NULL DEFAULT 'skill';
ALTER TABLE skills ADD COLUMN searchable INTEGER NOT NULL DEFAULT 1;
ALTER TABLE skills ADD COLUMN user_invocable INTEGER NOT NULL DEFAULT 0;
ALTER TABLE skills ADD COLUMN when_to_use TEXT;

DROP INDEX IF EXISTS skills_unique;
CREATE UNIQUE INDEX skills_unique ON skills (catalog, plugin, kind, name);
```

- `kind`: `'skill'` or `'command'`. Origin marker only; doesn't drive
  behaviour beyond initial defaults for `user_invocable` at insert time.
- `searchable`: indexed for `search_skills`? Default `1`, set to `0`
  when `disable-model-invocation: true`.
- `user_invocable`: exposed as MCP prompt? Default `0` for skills, `1`
  for commands, overridable by `user-invocable` frontmatter.
- `when_to_use`: nullable; populated from frontmatter at index time. Used
  by `get_skill_info` and the embedding text composer.

No new tables. `skill_embeddings` virtual table unchanged.
`workspace_skills` junction unchanged.

Schema version bumps; migration registered in code, runs in-process on
first open.

## Indexing pipeline updates

Plugin scan for an enabled plugin now walks both:

- `plugin/skills/*/SKILL.md` — kind `'skill'`
- `plugin/commands/*.md` — kind `'command'`

Both go into the same `skills` table with different `kind` values.
Argument-spec extraction, content_hash computation, and embedding apply
identically.

Embedding text composition (the string fed to the embedder for each
entry) becomes:

```
{name}

{description}

When to use: {when_to_use}
```

The "When to use:" prefix and blank-line separator only appear when
`when_to_use` is non-empty. Embedding model and dimensionality unchanged
(bge-small-en-v1.5 INT8 ONNX, 384-dim).

Plugins shipping the same entry name across kinds (a `skills/foo/SKILL.md`
AND a `commands/foo.md`) produce two distinct rows. No collision.

## MCP server changes

### `search_skills` — updates

Existing tool gains:
- Filter: `WHERE searchable = 1` (was implicit; now explicit)
- Description truncation: results include description truncated to a
  default of 150 chars (was: full description)
- New optional parameter `description_max_chars` to override the truncation

Tool input schema:

```json
{
  "type": "object",
  "properties": {
    "query":                  { "type": "string" },
    "top_k":                  { "type": "integer", "default": 10 },
    "catalog":                { "type": "string" },
    "plugin":                 { "type": "string" },
    "description_max_chars":  { "type": "integer", "default": 150 }
  },
  "required": ["query"]
}
```

Each result object:

```json
{
  "catalog": "midnight-expert",
  "plugin": "compact-dev",
  "name": "compact-circuits",
  "kind": "skill",
  "description": "<truncated>",
  "path": "/abs/path/to/SKILL.md",
  "score": 0.87
}
```

### `get_skill_info` — new

Mid-tier metadata fetch. Cheap call (~1KB typical), no body.

Input:
- `catalog` (string, required)
- `plugin` (string, required)
- `name` (string, required)
- `kind` (string, optional, default `'skill'`): disambiguates when the
  same name exists in both kinds for a plugin

Output:

```json
{
  "catalog": "midnight-expert",
  "plugin": "compact-dev",
  "name": "compact-circuits",
  "kind": "skill",
  "path": "/abs/path/to/SKILL.md",
  "description": "<full, untruncated>",
  "when_to_use": "<full when_to_use frontmatter content>",
  "plugin_version": "1.4.0",
  "user_invocable": false,
  "resources": {
    "files": [
      "/abs/path/to/skill/config.json"
    ],
    "directories": {
      "scripts": [
        "/abs/path/to/skill/scripts/audit.py",
        "/abs/path/to/skill/scripts/lint.py",
        "/abs/path/to/skill/scripts/build.sh",
        "/abs/path/to/skill/scripts/deploy.sh",
        "/abs/path/to/skill/scripts/test.sh",
        "and 3 more"
      ],
      "references": [
        "/abs/path/to/skill/references/api-spec.md",
        "/abs/path/to/skill/references/glossary.md"
      ],
      "examples": [
        "/abs/path/to/skill/examples/basic.ts",
        "/abs/path/to/skill/examples/advanced.ts"
      ]
    }
  }
}
```

The `resources` object describes the entry's directory contents — files
and subdirectories other than the entry file itself. Structure:

- `files`: array of absolute paths to top-level files in the entry's dir
  (excluding the SKILL.md / command.md file itself)
- `directories`: object keyed by top-level directory name. Each value is
  an array of absolute paths to the directory's immediate children
  (one level deep — nested subdirectories appear as paths with trailing
  `/` and are not recursed into).

**Per-directory cap of 5 entries**: if a directory contains more than
5 children, only the first 5 are listed (sorted alphabetically) followed
by a sentinel string `"and N more"` where N is the count of omitted
entries. The agent can use a filesystem view tool to enumerate further
if needed.

For commands (which are flat `.md` files in `plugin/commands/` rather
than directories), the `resources` object is omitted entirely — commands
don't have associated resource directories. Only skills produce a
`resources` field.

### `get_skill` — updates

Existing tool, shape unchanged. New behaviour: applies substitution
when `args` are provided.

Input:
- `catalog`, `plugin`, `name`, `kind` (as above)
- `args` (string OR object, optional): arguments to substitute

Output:
- `content`: the rendered body (substituted)
- `path`: absolute path to the source `.md`

**Built-in and env passthrough substitution always run**, regardless of
whether `args` are provided. `${TOME_SKILL_DIR}`, `${TOME_PLUGIN_DATA}`,
`${TOME_ENV_*}`, etc. all resolve on every `get_skill` and `prompts/get`
call. Only the argument substitution stage (`$ARGUMENTS` family) is
gated on caller-supplied args.

This matters because skills frequently reference paths in their body
(e.g., `${TOME_SKILL_DIR}/scripts/audit.py`) without any argument
involvement. The agent calling `get_skill` without args should still
receive a fully-resolved body for those references.

### MCP prompts surface

On startup, after loading workspace state, the server enumerates entries
where `user_invocable = 1` and lists them under the `prompts` capability.

`prompts/list` returns the full set with arg schemas. `prompts/get`
returns the rendered body for the named prompt with caller args applied.

The MCP spec's `prompts` capability is declared during initialization:

```json
{ "capabilities": { "prompts": { "listChanged": false } } }
```

`listChanged: false` for now (prompts are static for the session;
workspace switches require server restart, which is per-session anyway).

### Prompt naming

Format: `<plugin>__<entry-name>`. The harness prepends its own
`mcp__<server>__` (Tome contributes only the plugin and entry portions).

Sanitisation rules:
- Lowercase; non-alphanumeric chars except `_` and `-` replaced with `_`;
  runs of `_` collapsed
- Plugin truncated to 16 chars
- Entry name truncated to ~32 chars (budget minus prefix and separator)
- `prompt_name` frontmatter override replaces both portions with a single
  user-chosen string, still subject to sanitisation and length limits

Truncation logged at debug level — no warning needed.

### Collision handling

If two entries resolve to the same prompt name after sanitisation:
- Order by `indexed_at` ascending; oldest wins the unsuffixed name
- Subsequent entries get a counter: `foo__bar`, `foo__bar2`, `foo__bar3`
- Each collision logged at warn level — typically a sign of name ambiguity
  worth resolving via `prompt_name`

Same algorithm regardless of whether the colliding entries are
skills/skills, commands/commands, or skill/command from the same plugin.

## Plugin enable/disable extensions

Phase 4's enable/disable flow operates on `workspace_skills`. Phase 5
keeps that — `workspace_skills` now references entries of either kind via
the widened unique constraint.

**Enable**:
1. Parse plugin's SKILL.md files (existing) and `commands/*.md` files
   (new), creating entries with the appropriate `kind`.
2. For each entry: compute content_hash, insert/update `skills` row,
   re-embed if hash changed.
3. Insert `workspace_skills` rows for the active workspace, covering
   both kinds.
4. Summary regeneration step (existing) now includes command names and
   descriptions in the LLM input.
5. Rules-file / MCP-config sync runs as Phase 4.

**Disable**:
1. Delete `workspace_skills` rows for the plugin (all kinds).
2. Summary regen + integration sync (existing).

**Reindex** unchanged: only re-embed entries whose content_hash changed
(or all, with `--force`).

## Summary regeneration impact

Phase 4's workspace summary regeneration ran the bundled
Qwen2.5-0.5B-Instruct model over enabled entries. Phase 5 expands the
input to include commands alongside skills (no change to prompts or
model). Output naturally mentions commands when present:

> ...skills for Compact contract authoring and zero-knowledge circuit
> review, plus commands for issue fixing and component migration.

Regeneration triggers expand to include enabled entries of both kinds —
no logic changes beyond the broader input.

## Doctor extensions

`tome doctor` for Phase 5:

- Effective MCP surface for the current project: tool list (always 3)
  plus enumerated prompts grouped by plugin
- Prompt name collisions surfaced (counters applied, original names)
- Truncated descriptions logged separately from full-length descriptions
- Orphaned `_DATA` directories (referenced plugin no longer enabled in
  any workspace) — informational only in Phase 5; cleanup is Phase 6+
- Embedding index status: counts per kind, entries pending re-embedding

`--fix` doesn't gain new capabilities. Phase 5 issues are configuration
or content concerns, not auto-repairable.

## CLI surface

No new CLI commands. The Phase 1-4 surface unchanged.

`tome plugin show <catalog>/<plugin>` output now lists both skills and
commands separately, indicating each entry's `searchable` and
`user_invocable` state and the derived prompt name where applicable.

`tome query <text>` from Phase 2 stays in place — searches the unified
`skills` table, returns entries of either kind.

## Exit codes (additions)

| Code | Meaning |
|---|---|
| 21 | Entry not found |
| 22 | Substitution failed (e.g., required named arg missing) |
| 23 | Invalid argument frontmatter (malformed list, illegal names) |
| 24 | Prompt argument count exceeds caller-supplied args |
| 25 | Workspace data directory write failed |

## Success criteria

Phase 5 is done when:

- A fixture catalog containing a plugin with both `skills/` and
  `commands/` directories can be enabled. After enable: `skills` table
  contains entries of both `kind` values with correct defaults applied.
- `search_skills "fix issue"` returns relevant entries from both kinds
  with descriptions truncated to 150 chars.
- `get_skill_info` returns `when_to_use`, full description, and resource
  paths for a target entry.
- `get_skill` with no args returns the body with `${TOME_*}` and
  `${TOME_ENV_*}` substituted but no argument substitution applied.
- `get_skill` with args substitutes `$ARGUMENTS`, `$ARGUMENTS[N]`, `$N`,
  and `$<name>` correctly. `ARGUMENTS:` append fallback triggers when
  args are passed but the body has no references.
- A command with `arguments: [component, from, to]` called via
  `prompts/get` with `{component: "SearchBar", from: "React", to: "Vue"}`
  substitutes all positional and named forms in the rendered body.
- A skill with `user-invocable: true` appears as an MCP prompt.
- A command with `disable-model-invocation: true` is absent from
  `search_skills` results but present in `prompts/list`.
- `${TOME_PLUGIN_DATA}` resolves to
  `~/.tome/plugin-data/<catalog>/<plugin>/`; directory exists after
  substitution. Survives a `tome reindex --force` on the plugin.
- `${TOME_WORKSPACE_DATA}` resolves to the per-workspace path; isolated
  from `${TOME_PLUGIN_DATA}`.
- `${TOME_ENV_FOO}` returns the value of env var `TOME_ENV_FOO`;
  `${TOME_ENV_FOO:-default}` returns `default` when unset.
- A skill body containing `${GITHUB_TOKEN}` is NOT substituted (token
  not exfiltrated; placeholder left as-is per non-Tome namespace rule).
- A skill body containing `` !`gh pr diff` `` is NOT executed by Tome
  (deferred to Phase 6); syntax left as-is.
- Two enabled entries producing the same prompt name (after sanitisation)
  receive counter suffixes (`foo__bar`, `foo__bar2`); each collision
  warn-logged.
- Workspace summary regeneration after enabling a command-bearing plugin
  mentions command topics in the new RULES.md.
- `tome doctor` reports the effective prompts list, any collisions, and
  any orphaned `_DATA` directories.
- All Phase 1-4 success criteria still hold.

## Resolved decisions

| Question | Decision |
|---|---|
| Commands as MCP tools or prompts | Prompts. Tools were a category error — commands are user-invoked. |
| Separate tables for commands vs skills | Single `skills` table with `kind` discriminator |
| Unique constraint widening | `(catalog, plugin, kind, name)` |
| Discovery tools | Three-tier: `search_skills` → `get_skill_info` → `get_skill` |
| `search_skills` description truncation | Default 150 chars; configurable via `description_max_chars` parameter |
| `get_skill_info` shape | Full description + `when_to_use` + metadata + resource paths; no body |
| Embedding text composition | `name` + `description` + `when_to_use` (when present) |
| `disable-model-invocation` frontmatter | Honoured. Sets `searchable = 0`. |
| `user-invocable` frontmatter | New. Default `false` for skills, `true` for commands. |
| `show_as_tool` frontmatter | Dropped — irrelevant under prompts model |
| `TOME_EXPOSE_COMMANDS_AS_TOOLS` env var | Dropped — irrelevant under prompts model |
| Codex (no prompts support yet) | Users access commands via `search_skills`/`get_skill` tools until Codex ships prompts support (issue #8342) |
| OpenCode prompts support | Probably ✓; verify at impl time |
| Variable substitution engine | Hand-rolled; no Tera/MiniJinja dependency |
| Built-in variables | 12 total (paths, names, versions, data dirs, date, timestamp) |
| `${TOME_WORKSPACE_DIR}` | Dropped — exposes internal layout for no good reason; `${TOME_WORKSPACE_DATA}` covers the writable use case |
| Env var passthrough | `${TOME_ENV_*}` prefix only; user-controlled opt-in via MCP config |
| Default-value syntax | `${VAR:-default}` on both built-ins and env vars |
| `${CLAUDE_*}` variables | Left untouched; downstream harness can process |
| Substitution order | Built-ins → env passthrough → arg substitution → `ARGUMENTS:` append fallback |
| Recursive expansion of arg values | No — arg values are opaque strings |
| Server-side shell exec (` !``cmd`` `) | Deferred to Phase 6 |
| Prompt name format | `<plugin>__<entry-name>`; always prefixed |
| Plugin name truncation | 16 chars |
| Entry name truncation | ~32 chars (budget remaining) |
| `prompt_name` frontmatter override | Replaces both prefix and entry portions of the generated prompt name with a single user-chosen string |
| Collision handling | Counter starting at 2 (`foo__bar`, `foo__bar2`); warn-logged |
| `${TOME_PLUGIN_DATA}` scope | Global per plugin; shared across workspaces |
| `${TOME_WORKSPACE_DATA}` scope | Per workspace, per plugin |
| `_DATA` dir creation | Lazy: `mkdir -p` on first substitution; idempotent |
| `_DATA` dir cleanup | Out of scope; Phase 6+ |
| `${TOME_HARNESS}` builtin | Not provided; encourages anti-patterns. Users wire `TOME_ENV_HARNESS` if needed. |
| `${TOME_SESSION_ID}` | Not provided; no session concept |
| Templating engine | None — hand-rolled is enough |
| Agents (subagents) translation | Out of scope; likely permanent |
| Hooks translation | Phase 6 |

## Phase 6 preview

Out of scope here, signposted:

- Hooks translation: lifecycle event mapping across harnesses, semantics
  matrix, the gnarliest design problem remaining
- Server-side shell command execution (` !``cmd`` `): `allowed-tools`
  parsing, sandboxing, user consent flow, audit trail
- `_DATA` directory lifecycle: orphan detection, `tome doctor --fix`,
  `tome plugin remove --purge`
- Release tooling, dogfooding pass, public install path
- Additional harnesses (Cline, Goose, Aider, etc.)
- Plugin authoring tools
- HTTP / SSE MCP transport (demand-driven)
