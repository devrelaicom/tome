# Tome — Phase 6 PRD

## Overview

Phase 6 closes out the two component types Tome has deferred since the
start: **hooks** and **agents** (subagents). Both are the divergent end
of the cross-harness spectrum — there is no clean universal mapping for
either — so Phase 6 is deliberately asymmetric about how far it commits
per harness.

Hooks split into two mechanisms. **Real hooks** are supported for Claude
Code only: a plugin's `hooks/hooks.json` is rewritten and merged into the
project's `.claude/settings.local.json` on enable/sync. For every other
harness — and for Claude Code plugins that ship no JSON hooks — Tome
introduces **`GUARDRAILS.md`**, a soft, prose-level fallback that
describes pre/post-action constraints and gets rendered into each
harness's rules file as a Tome-managed, per-plugin region. Guardrails are
advisory by construction: the agent can ignore them. That is worse than a
real hook and strictly better than nothing.

Agents get **full native translation** across four harnesses — Claude
Code, Codex, Cursor, OpenCode — emitting each harness's own agent file
format with a best-effort field-and-value mapping. For harnesses without
native agent support (and, optionally, everywhere), agents can also be
exposed as **MCP-prompt "personas"** — a global, off-by-default fallback
that lets a user assume and drop an agent's persona via slash commands.

Phase 6 also corrects a latent Phase 4 bug: Claude Code does not natively
read `AGENTS.md`, so Tome's rules-file and guardrails content for Claude
Code must target `CLAUDE.md`.

After Phase 6, enabling a plugin in a workspace reconciles, per harness in
the effective list: real hooks (Claude Code), guardrails regions (all),
and native agent files (the four). The MCP server optionally exposes
agent personas as prompts alongside Phase 5's command/skill prompts.

## Goals

1. Real hooks for Claude Code: copy `plugin/hooks/hooks.json` into the
   project's `.claude/settings.local.json`, rewriting plugin-root path
   variables to absolute paths, with exact-match merge on add and
   exact-match removal on sync.
2. `GUARDRAILS.md` soft fallback: a per-plugin, marker-delimited region
   rendered into each harness's rules file (or a sibling file for
   standalone-rules harnesses), suppressed for Claude Code when the same
   plugin ships real JSON hooks.
3. Native agent translation across Claude Code, Codex, Cursor, and
   OpenCode — emitting each harness's file format, mapping only the
   frontmatter fields (and values) the target harness supports or that
   map cleanly to a supported field.
4. Optional agent-as-MCP-prompt personas: a global, off-by-default config
   flag that exposes each agent as a `<name>-persona` prompt plus a single
   global `drop-persona` prompt.
5. Correct Phase 4: Claude Code's rules-file (and guardrails) sink is
   `CLAUDE.md`, not `AGENTS.md`.

## Non-goals (Phase 6)

Explicitly out of scope:

- **Real hooks for any harness other than Claude Code.** Codex (stable
  JSON-shaped hooks since v0.124) and Gemini CLI both have real hook
  systems; supporting them is signposted for a later phase, not built
  here. All other harnesses get guardrails only.
- **Native agent files for Gemini/Antigravity and the P2 harnesses.**
  Gemini has a native MD+YAML agent format but is sunsetting for
  individuals (June 18, 2026); it gets guardrails and, when enabled,
  personas — no native agent files. Antigravity's agent schema is
  undocumented.
- **Emulating Claude Code-only agent semantics.** Tome translates and
  copies frontmatter; it does not reimplement agent teams, worktree
  isolation, persistent agent memory, `initialPrompt` auto-submission,
  or `effort` for harnesses that lack them. Unmappable fields are
  dropped.
- **Semantic search over agents.** Agents are indexed for provenance and
  collision detection only; they are not exposed through `search_skills`
  in v1.
- **Server-side shell execution** (`` !`cmd` ``): still deferred (Phase 5
  carryover).
- **Hook authoring/validation tooling, plugin scaffolding.**
- New harnesses beyond Phase 4's five.

## Mental model

Two component types, two very different stances.

**Hooks are about enforcement, and enforcement doesn't port.** A real
Claude Code hook is a process Claude Code runs and obeys (a non-zero
`PreToolUse` exit blocks the tool call). No other harness shares Claude
Code's hook protocol, and the memory/rules layer is explicitly *not* an
enforcement layer — Anthropic's own docs say memory files are treated as
context, not enforced configuration, and that to actually block an action
you must use a `PreToolUse` hook. So Tome makes a clean split: where real
enforcement is available (Claude Code + JSON hooks), use it; everywhere
else, degrade honestly to a prose guardrail the agent is *asked* to
follow. Guardrails never masquerade as enforcement.

**Agents are about portability, and agents mostly port.** Four harnesses
converged on the same shape — a file with metadata plus a system-prompt
body, stored under `<harness>/agents/`, description-driven delegation,
fresh context per spawn. Tome translates the source agent (Markdown +
YAML frontmatter, the Claude Code/plugin convention) into each target's
dialect, carrying across the fields that survive translation and dropping
the rest. Where a harness can't host a native agent file, the agent's
behaviour can still be approximated by injecting its body as a persona
prompt.

Tome never executes hook commands or agent bodies itself. It rewrites and
places files; the harness owns runtime behaviour.

---

# Part 1 — Hooks

## 1.1 Real hooks (Claude Code only)

### Source

A plugin's hooks live at `plugin/hooks/hooks.json` (the canonical Claude
Code plugin location). Tome reads it on plugin enable and on
`tome harness sync` for any project whose effective harness list includes
`claude-code`.

If a plugin ships no `hooks/hooks.json`, there is nothing to copy — the
guardrails path (§1.2) takes over for Claude Code.

### Target

The rewritten hooks merge into the project's
**`.claude/settings.local.json`** under the `hooks` key. This file is the
correct sink, not `.claude/settings.json`, for two reasons:

- It is gitignored (Claude Code's "local" scope). Tome's rewritten hooks
  reference machine-specific absolute paths (see path rewriting below), so
  they are inherently local and must not be committed.
- It keeps Tome's writes out of the team-shared, committed settings file.

If `.claude/settings.local.json` does not exist, Tome creates it with a
single `hooks` object.

### Path-variable rewriting

Plugin hook commands almost universally reference `${CLAUDE_PLUGIN_ROOT}`
(and occasionally `${CLAUDE_PLUGIN_DATA}`). Once the hook is lifted out of
the plugin and into project settings, Claude Code has no plugin-root
context to resolve those against — Tome installs plugins into its own
cache, not via Claude Code's plugin manager. So at copy time Tome rewrites:

| Source variable | Rewritten to |
|---|---|
| `${CLAUDE_PLUGIN_ROOT}` | Absolute path to the installed plugin root (the same value Phase 5 exposes as `${TOME_PLUGIN_DIR}`) |
| `${CLAUDE_PLUGIN_DATA}` | Absolute path to the plugin's persistent data dir (`${TOME_PLUGIN_DATA}` value: `~/.tome/plugin-data/<catalog>/<plugin>/`) |

All other `${CLAUDE_*}` variables (e.g. `${CLAUDE_PROJECT_DIR}`,
`${CLAUDE_SESSION_ID}`) are **left untouched** — Claude Code resolves
those natively at runtime. The rewrite is a targeted two-variable
substitution, not the full Phase 5 substitution pipeline. Rewriting is
purely textual within string values of the hook JSON.

### Merge semantics (add)

For each hook entry in the plugin's `hooks.json` (after rewriting), keyed
by event:

1. Render the entry to its final form (post-rewrite).
2. Look under the same event in the existing `settings.local.json`
   `hooks` object.
3. If a **structurally identical** entry already exists, skip it.
4. Otherwise, append it to that event's array.

"Structurally identical" means deep JSON equality of the entry object
(matcher group + handler array) after rewriting. This is idempotent:
syncing twice appends nothing the second time, and a plugin sharing a hook
the user already wrote by hand is not duplicated.

### Removal semantics (sync)

On disable, or when `claude-code` leaves a project's effective harness
list, or when a plugin leaves the workspace:

1. Re-derive the plugin's rewritten hook entries (same rendering as add).
2. For each, find the structurally identical entry under its event in
   `settings.local.json` and remove it.
3. If no exact match is found, **skip** — do not guess, do not remove
   near-matches.

This preserves Phase 4's no-sidecar, filesystem-inferred state model: Tome
identifies its own hooks purely by re-deriving what it would have written
and matching structurally. The deliberate consequence: a hook the user
hand-edited after Tome wrote it will no longer match exactly and is left
in place on removal. That is the correct, conservative behaviour — Tome
never deletes a hook it cannot prove it owns.

Empty `hooks` event arrays are pruned; an empty `hooks` object is left in
place (harmless).

## 1.2 `GUARDRAILS.md` — soft fallback

### Source

A new optional file at `plugin/hooks/GUARDRAILS.md`. Plain Markdown, no
required frontmatter. Plugin authors describe constraints in prose:

```markdown
# Guardrails

- Before calling `rm`, always stop and ask for the user's explicit
  permission, quoting the exact paths to be removed.
- Never force-push to `main` or `master`.
- After editing any file under `migrations/`, run `make db-check` and
  report the result before continuing.
```

The body is copied verbatim into the target; Tome does not parse or
interpret it.

### Where it goes, per harness

Guardrails render into a **Tome-managed, per-plugin region** in the same
rules file Phase 4's `tome:begin/end` include block lives in — adjacent to
it, not inside it. The region is delimited by markers carrying provenance:

```markdown
<!-- START GUARDRAILS: <catalog>:<plugin> -->
<contents of the plugin's GUARDRAILS.md>
<!-- END GUARDRAILS: <catalog>:<plugin> -->
```

(`<catalog>` is Tome's term for what Claude Code calls a marketplace; the
markers use Tome's vocabulary for consistency with the rest of the CLI.)

Target resolution reuses Phase 4's harness modules:

| Harness | Rules-file strategy | Guardrails target |
|---|---|---|
| `claude-code` | `BlockInExistingFile` | `CLAUDE.md` (see Phase 4 correction, §3) |
| `codex` | `BlockInExistingFile` | `AGENTS.md` |
| `gemini` | `BlockInExistingFile` | `AGENTS.md` > `GEMINI.md` |
| `opencode` | `BlockInExistingFile` | `AGENTS.md` |
| `cursor` | `StandaloneFile` | sibling file `.cursor/rules/TOME_GUARDRAILS.md` |

For `BlockInExistingFile` harnesses, each enabled plugin with a
`GUARDRAILS.md` contributes one marker-delimited region to the target
file. For Cursor, all enabled plugins' guardrails are written to a single
Tome-owned sibling file (`TOME_GUARDRAILS.md`), separate from
`TOME_SKILLS.md`, each plugin still wrapped in its own markers for
per-plugin removal.

### Claude Code suppression rule

For Claude Code specifically: if a plugin ships **any** `hooks/hooks.json`,
its guardrails are **not** rendered into `CLAUDE.md` — the real hooks
supersede the prose fallback. Simple version: presence of a JSON file
suppresses the guardrails region for that plugin, on Claude Code only.
(Per-event merging of "JSON for some events, prose for others" is
explicitly not attempted in v1.)

Because Claude Code targets `CLAUDE.md` (its own file) while Codex, Gemini,
and OpenCode share `AGENTS.md`, the suppression never creates a conflict
on a shared file: a plugin's guardrails can be present in `AGENTS.md` (for
the other harnesses) and absent from `CLAUDE.md` (suppressed by JSON
hooks) simultaneously, with no contradiction.

### Sync and removal

Guardrails reconciliation is **per file**, mirroring Phase 4's rules-block
logic:

- For each target file derived from the effective harness list, ensure
  exactly one region per enabled-plugin-with-guardrails (minus any plugin
  suppressed for that file's harness), with current content. Re-syncing
  overwrites the content between existing markers in place.
- For any file Tome could have written guardrails into, remove any region
  whose `<catalog>:<plugin>` marker corresponds to a plugin no longer
  enabled, or whose harness left the effective list, or (for Claude Code)
  whose plugin now ships JSON hooks.
- The Cursor sibling file is deleted entirely when no enabled plugin
  contributes guardrails to it.

State is filesystem-inferred via the marker pairs — no sidecar.

---

# Part 2 — Agents

Native agent files are emitted for **Claude Code, Codex, Cursor, and
OpenCode**. Source agents are Markdown + YAML frontmatter (the Claude
Code/plugin convention), living at `plugin/agents/*.md`.

## 2.1 Translation model

Each harness module gains an agent-emission capability declaring:

- `agent_dir(project_path)` — where agent files are written.
- `agent_format()` — `MarkdownYaml` or `Toml`.
- `translate_agent(canonical)` — maps the canonical (Claude Code) field
  set to the harness's dialect, dropping unsupported fields.

| Harness | Format | Project agent dir |
|---|---|---|
| `claude-code` | Markdown + YAML | `.claude/agents/` |
| `codex` | TOML | `.codex/agents/` |
| `cursor` | Markdown + YAML | `.cursor/agents/` |
| `opencode` | Markdown + YAML | `.opencode/agent/` |

All four are written at **project scope** (consistent with Phase 4 sync
running from a project directory). Cursor and OpenCode natively read
`.claude/agents/`, but Tome emits native files anyway — relying on the
cross-read would ship subtly wrong agents (e.g. an untranslated `model`
value those harnesses can't interpret).

### Field mapping (general principle)

**Pass through only the frontmatter fields the target harness supports, or
that map cleanly to a field it supports. Drop everything else.** No field
is passed through verbatim on the assumption the harness will tolerate
unknown keys.

Canonical → per-harness field map (Claude Code-normalized names):

| Canonical field | Claude Code | Codex (TOML) | Cursor | OpenCode |
|---|---|---|---|---|
| `name` | `name` | `name` | `name` | (filename-derived) |
| `description` | `description` | `description` | `description` | `description` |
| system-prompt body | body | `developer_instructions` | body | body |
| `model` | `model` | `model` | `model` | `model` |
| `tools` (allowlist) | `tools` | — (via sandbox/mcp) → drop | `tools` | `permission` (per-tool) |
| `disallowedTools` | `disallowedTools` | — drop | — drop | `permission: deny` |
| read-only intent | `tools` subset | `sandbox_mode = "read-only"` | `readonly: true` | `permission` (edit/bash → ask/deny) |
| `mode` (primary/subagent) | — (implicit) | — | — | `mode` (default `subagent`) |
| `temperature` | — drop | — drop | — drop | `temperature` |
| `effort`, `maxTurns`, `skills`, `memory`, `isolation`, `initialPrompt`, `background`, `permissionMode`, `mcpServers`, `hooks` | native | mostly drop / Codex-specific | drop | `steps` for `maxTurns`; rest drop |

The table is the intended core; per-harness specifics are verified against
current docs at implementation time (the ecosystem moves fast — same
caveat as Phase 4).

Notes:

- **Codex body → `developer_instructions`.** The Markdown body becomes a
  TOML triple-quoted string under `developer_instructions`. Frontmatter
  keys become TOML keys.
- **OpenCode `mode`.** A translated agent defaults to `mode: subagent`
  (source agents are subagents). `description` is required by OpenCode;
  if the source lacks one, fall back to the first line of the body and
  debug-log.
- **Read-only intent** is reconstructed per harness from the source's tool
  posture where expressible, dropped where not.

### Value mapping (`model` is the hard case)

Field *names* port; field *values* often don't. The policy: **if a value
has no sane same-vendor target, drop the field and let the harness
inherit its default.** Never map across vendors.

- `model: opus` → Codex: **dropped** (never `gpt-5.1` or any OpenAI ID).
- `model: opus` → OpenCode: `anthropic/claude-opus-4.7` (same vendor, a
  legitimate mapping).
- `model: opus` → Cursor: mapped to Cursor's Anthropic model identifier
  where one exists, else dropped.
- `model: inherit` → dropped everywhere (inheriting is the default).

Dropped fields are debug-logged. Cross-vendor "strongest-to-strongest"
heuristics are explicitly forbidden — they rot and surprise users.

## 2.2 File naming, provenance, and removal

- **Filenames are always namespaced**: `<plugin>__<name>.<ext>` (e.g.
  `midnight-expert__reviewer.md`, `midnight-expert__reviewer.toml`). This
  is the sole provenance mechanism.
- **No provenance frontmatter key.** A stray unknown key risks breaking a
  harness's parser; the probability of trampling a user-authored agent
  that happens to use the `<plugin>__*` filename convention is far lower.
  Removal globs `<plugin>__*.<ext>` in each harness's agent dir.
- **Emitted (displayed/registered) name** uses the clean `<name>`
  normally, and `<plugin>-<name>` only when two enabled plugins in the
  workspace clash on `<name>` (prevents an in-harness name collision). The
  *filename* stays `<plugin>__<name>` regardless of clash.
- **OpenCode caveat**: OpenCode derives the agent name from the filename,
  so OpenCode agents are always named `<plugin>__<name>` (the prefix can't
  be hidden). An accepted wart of OpenCode's model.

Removal is reconciled in `tome harness sync` and on disable: for each
harness with native agent support in the effective list, ensure enabled
plugins' agents are present and translated, and remove `<plugin>__*.<ext>`
files for plugins no longer enabled or harnesses no longer in the list.

## 2.3 The plugin-frontmatter privilege ban — and why Tome ignores it

Claude Code forbids **plugin-shipped** subagents from setting `hooks`,
`mcpServers`, or `permissionMode` in frontmatter (anti-privilege-
escalation). That ban applies *only* to plugin subagents. Agents placed in
`.claude/agents/` as project/user files support the full field set —
including those three. Anthropic's own guidance is "if you need them, copy
the file into `.claude/agents/` and own it," which is exactly what Tome
does.

Therefore: **Tome passes those fields through by default**, because doing
so is a genuine capability advantage of installing a plugin's agents via
Tome over installing the same plugin through Claude Code's native plugin
manager.

The trade-off is honest and must be visible: passing them through also
removes the guardrail the native plugin manager enforces. A third-party
plugin's `hooks`/`mcpServers`/`permissionMode` that Claude Code would
refuse will be honoured once Tome lands the agent in `.claude/agents/`.
So:

- **`tome doctor` reports** every installed agent that carries `hooks`,
  `mcpServers`, or `permissionMode`, grouped by plugin, so the escalation
  surface is auditable.
- A **workspace/global config setting** (`strip_plugin_agent_privileges`,
  default `false`) restores Claude Code's plugin-parity behaviour by
  stripping those three fields from emitted Claude Code agents. It lives
  in the same settings layering as `harnesses` (project → workspace →
  global), so a cautious user can enforce it org-wide.

(This reverses the defensive stripping suggested during design review:
stripping by default would throw away the advantage. We pass through, make
it visible, and offer the strip as opt-in.)

## 2.4 Agent personas (MCP-prompt fallback)

For harnesses without native agent files — and, since MCP prompts are
harness-agnostic, for every connected client — agents can be exposed as
**personas**: MCP prompts that inject the agent's body as an assumed role.
This reuses Phase 5's prompt machinery and substitution pipeline.

### Opt-in

Disabled by default. Enabled via a single **workspace/global config
flag** (`expose_agents_as_personas`, default `false`). Because MCP prompts
can't be scoped to specific harnesses, enabling it exposes personas to all
connected clients — including the four that already get native agent
files. That redundancy is harmless and is precisely why the feature is
global and off by default.

### Persona name resolution

The persona's `<name>` is taken from the agent's frontmatter `name` field
(read **before** the frontmatter is stripped during conversion); if the
frontmatter has no `name`, fall back to the agent Markdown filename stem.

Prompt naming:

- `<name>-persona` normally (unprefixed — usually unique enough).
- `<plugin>-<name>-persona` only when two or more enabled plugins clash on
  `<name>`, applied to the clashing agents only.

Subject to Phase 5's sanitisation and length limits and collision
counters as a final backstop.

### Persona prompt body

Frontmatter is stripped; the body is wrapped:

```
Assume the following <Name> persona until instructed otherwise.

<<name>-persona>
<agent markdown body, with Phase 5 substitution applied>
</<name>-persona>

While acting as the <Name> persona, you must: $ARGUMENTS
```

`$ARGUMENTS` runs through the **exact Phase 5 substitution pipeline**: a
single catch-all `args` argument on the prompt schema, with the
`ARGUMENTS:` append fallback when the user passes input the template
doesn't consume. No parallel substitution path.

Users invoke it as `/<name>-persona <prompt text>` (or
`/<plugin>-<name>-persona ...` on clash).

### Drop-persona

A single **global, unnamespaced** prompt, `drop-persona`, exposed once
when `expose_agents_as_personas` is on (not per plugin):

```
Stop acting as any assumed persona and return to your default behaviour
and personality.
```

### Advisory-state caveat

A persona is conversational context, not enforced configuration — the
agent can drift or ignore it, exactly like guardrails. The PRD and any
user-facing docs state this plainly so personas are never mistaken for the
isolation/tool-scoping a native subagent provides.

### Indexing

Agents are indexed into the existing `skills` table with `kind = 'agent'`
(the `kind` column from Phase 5 gains a third value):

- `searchable = 0` always (agents are not in `search_skills` in v1).
- Reused columns only; **no new columns required**. The resolved persona
  name is derived at render time; the body is read from the plugin's
  on-disk agent file (same sourcing model as skills read their
  `SKILL.md`).
- The unique constraint `(catalog, plugin, kind, name)` from Phase 5
  accommodates agents without change.

Indexing exists for two reasons: provenance/enabled-state tracking, and
cheap **cross-plugin name-collision detection** (the `<name>` clash query
that drives both filename/emitted-name prefixing and persona naming is a
workspace-scoped query over `kind = 'agent'` rows).

Persona prompts are emitted by the MCP server's `prompts/list` /
`prompts/get` layer from agent rows when `expose_agents_as_personas` is
on — a specialised path (template-wrapped body, `-persona` naming) parallel
to Phase 5's command/skill prompts, not folded into them.

---

# Part 3 — Phase 4 correction: Claude Code reads `CLAUDE.md`, not `AGENTS.md`

Phase 4's harness table lists Claude Code's rules-file target as
`AGENTS.md > CLAUDE.md`. This is wrong: Claude Code does not natively read
`AGENTS.md` (the feature request is open and unshipped). Claude Code loads
the `CLAUDE.md` hierarchy plus auto-memory; the only way `AGENTS.md`
content reaches Claude Code is via an explicit `@AGENTS.md` import inside a
`CLAUDE.md`. Consequence under Phase 4: a Tome-managed project with an
`AGENTS.md` (shared with Codex/Gemini/OpenCode) but no `CLAUDE.md` would
have its `tome:begin/end` include block — and would have had its
guardrails — invisible to Claude Code.

### The fix

**Claude Code's harness module targets `CLAUDE.md`** for both the Phase 4
rules-include block and Phase 6 guardrails. The `@.tome/RULES.md` include
goes into `CLAUDE.md`; Codex/Gemini/OpenCode keep sharing one `AGENTS.md`
block. Both pointers resolve to the same `.tome/RULES.md`, so there is no
content duplication — two small include directives in two files, one of
which Claude Code actually reads. No transitive
`CLAUDE.md → @AGENTS.md → @RULES.md` chain, and no dependence on Claude
Code ever shipping `AGENTS.md` support.

The user's existing-rules-file precedence from Phase 4 still applies
*within* Claude Code's own file set (an existing `CLAUDE.md` is used and
its block updated in place). The optional courtesy of scaffolding a
`CLAUDE.md` with `@AGENTS.md` to surface a user's hand-written `AGENTS.md`
into Claude Code is **out of scope** (deliberately skipped).

---

# Harness-module additions

Each harness module (Phase 4) gains:

- `hooks_strategy()` → `RealJson` (Claude Code) | `GuardrailsOnly`
  (all others).
- `hook_settings_path(project_path)` → `.claude/settings.local.json`
  (Claude Code); unused for `GuardrailsOnly`.
- `guardrails_target(project_path)` → file + placement (in-file region vs
  standalone sibling), and the Claude-Code-only suppression predicate
  (`plugin ships hooks.json`).
- `supports_native_agents()` → `true` for `claude-code`, `codex`,
  `cursor`, `opencode`; `false` otherwise.
- `agent_dir(project_path)`, `agent_format()`, `translate_agent(...)` —
  per §2.1.

And the Phase 4 correction:

- `claude-code`'s `rules_file_target(...)` now resolves to `CLAUDE.md`
  (not `AGENTS.md`).

---

# Database changes

Minimal. The `skills` table's `kind` column (Phase 5) gains a third
permitted value, `'agent'`. No new columns, no new tables, no constraint
changes. Hooks are **not** indexed — they are config artifacts reconciled
on the filesystem, consistent with Phase 4's no-sidecar model.

Schema version bumps only if the migration registry requires a marker for
the widened `kind` domain; no data migration is needed (the column is
free-text).

---

# Indexing pipeline updates

The plugin scan (Phase 5 walks `skills/*/SKILL.md` and `commands/*.md`)
now also walks:

- `plugin/agents/*.md` → `kind = 'agent'`, `searchable = 0`.

Agent rows carry `name` (frontmatter `name`, else filename stem),
`description`, and `content_hash`. Embedding is **skipped** for agents in
v1 (not searchable). Hook files (`hooks/hooks.json`, `hooks/GUARDRAILS.md`)
are read during harness sync, not during indexing.

---

# CLI surface

No new top-level commands. The Phase 1–5 surface is unchanged.

Behaviour extensions:

- `tome harness sync` additionally reconciles, per harness in the
  effective list: real hooks (Claude Code → `settings.local.json`),
  guardrails regions, and native agent files. Idempotent — a second run
  changes nothing.
- `tome plugin enable | disable` triggers the same reconciliation for the
  affected plugin across the workspace's bound projects' effective
  harnesses.
- `tome plugin show <catalog>/<plugin>` now lists agents, whether the
  plugin ships `hooks/hooks.json` and/or `hooks/GUARDRAILS.md`, and (for
  agents) the resolved persona name when `expose_agents_as_personas` is on.

Config settings (in `settings.toml`, project → workspace → global
layering, same as `harnesses`):

| Setting | Default | Effect |
|---|---|---|
| `expose_agents_as_personas` | `false` | Expose each agent as a `<name>-persona` MCP prompt plus a global `drop-persona`. |
| `strip_plugin_agent_privileges` | `false` | Strip `hooks`/`mcpServers`/`permissionMode` from emitted Claude Code agents (restores native-plugin-manager parity). |

---

# Doctor extensions

`tome doctor` for Phase 6:

- **Hooks (Claude Code):** which enabled plugins contributed hooks to
  `.claude/settings.local.json`, and any plugin-derived hook entries Tome
  expected but couldn't find (drift from user edits).
- **Guardrails:** per target file, the `<catalog>:<plugin>` regions
  present; orphaned regions (plugin no longer enabled); regions suppressed
  by JSON hooks on Claude Code.
- **Agents:** per harness, the `<plugin>__*` agent files present and any
  orphans; fields dropped during translation (informational); the
  privilege-escalation report — agents carrying `hooks`/`mcpServers`/
  `permissionMode` — grouped by plugin.
- **Personas:** when enabled, the effective persona prompt list with
  resolved names and any clash-prefixed names.

`--fix` repairs the safe, derivable cases (re-render stale guardrails
regions, re-emit missing agent files, remove orphaned `<plugin>__*` agent
files). It never removes a hook from `settings.local.json` that doesn't
exactly match a re-derived entry, and never deletes user-authored content.

---

# Exit codes (additions)

| Code | Meaning |
|---|---|
| 30 | Plugin `hooks/hooks.json` malformed or unparsable |
| 31 | Hook write to `.claude/settings.local.json` failed (read/merge/write) |
| 32 | Agent frontmatter malformed, or agent translation failed |
| 33 | Guardrails render/write failed (rules file or sibling file) |

(Phase 5 occupies through code 29.)

---

# Success criteria

Phase 6 is done when:

**Hooks (real, Claude Code):**

- A plugin shipping `hooks/hooks.json` with a `${CLAUDE_PLUGIN_ROOT}`
  reference, enabled in a `claude-code` project, results in
  `.claude/settings.local.json` containing the hook with
  `${CLAUDE_PLUGIN_ROOT}` rewritten to the absolute installed-plugin path
  and other `${CLAUDE_*}` variables left intact.
- Re-running `tome harness sync` adds no duplicate hook entry.
- A user-authored hook identical to a plugin hook is not duplicated on
  add.
- Disabling the plugin removes exactly the plugin's hook entries by
  structural match; a user-edited copy is left untouched.
- `.claude/settings.local.json` is used, never `.claude/settings.json`.

**Guardrails (soft fallback):**

- A plugin shipping `hooks/GUARDRAILS.md` and no `hooks.json`, enabled
  across all five harnesses, produces a
  `<!-- START GUARDRAILS: <catalog>:<plugin> -->`-delimited region in
  `CLAUDE.md`, `AGENTS.md`, and `.cursor/rules/TOME_GUARDRAILS.md`.
- A plugin shipping both `hooks.json` and `GUARDRAILS.md` has its
  guardrails region present in `AGENTS.md` (Codex/Gemini/OpenCode) and
  **absent** from `CLAUDE.md` (suppressed by the JSON hooks).
- Two enabled plugins each shipping `GUARDRAILS.md` produce two distinct
  regions in the shared `AGENTS.md`.
- Disabling a plugin removes only its region (matched by `<catalog>:
  <plugin>`); other plugins' regions remain.
- Re-syncing overwrites content between existing markers in place.

**Agents (native):**

- A plugin agent `reviewer.md` is emitted to `.claude/agents/`,
  `.codex/agents/`, `.cursor/agents/`, and `.opencode/agent/` as
  `midnight-expert__reviewer.{md,toml,md,md}`, with the body becoming
  `developer_instructions` in the Codex TOML and `mode: subagent` defaulted
  for OpenCode.
- `model: opus` maps to `anthropic/claude-opus-4.7` for OpenCode and is
  **dropped** for Codex (never an OpenAI ID).
- An unsupported field (e.g. `isolation: worktree`) is dropped for Cursor
  and OpenCode and debug-logged.
- Two plugins both shipping `reviewer` produce
  `pluginA__reviewer.*` / `pluginB__reviewer.*` files; their emitted names
  become `pluginA-reviewer` / `pluginB-reviewer` (clash-prefixed) while
  non-clashing agents keep clean names.
- Disabling a plugin removes its `<plugin>__*` agent files from every
  harness's agent dir; other plugins' agents remain.

**Privilege ban:**

- A plugin agent declaring `hooks` / `mcpServers` / `permissionMode` is
  emitted to `.claude/agents/` **with those fields intact** by default.
- `tome doctor` lists that agent under the privilege-escalation report.
- With `strip_plugin_agent_privileges = true` (workspace or global), the
  same agent is emitted to `.claude/agents/` **without** those three
  fields.

**Personas:**

- With `expose_agents_as_personas = false` (default), no persona prompts
  appear in `prompts/list`.
- With it `true`, each agent appears as `<name>-persona` (or
  `<plugin>-<name>-persona` on clash) and a single global `drop-persona`
  is present.
- `prompts/get` for a persona returns the wrapped body with frontmatter
  stripped, Phase 5 `${TOME_*}`/`${TOME_ENV_*}` substitution applied, and
  `$ARGUMENTS` resolved from the caller's `args`.
- The persona `<name>` is taken from frontmatter `name` when present, else
  the agent filename stem.

**Phase 4 correction:**

- A `claude-code` project gets its `@.tome/RULES.md` include block in
  `CLAUDE.md`, not `AGENTS.md`; a project shared with Codex still has one
  `AGENTS.md` block for the other harnesses; both resolve the same
  `.tome/RULES.md` with no duplicated content.

**General:**

- `tome harness sync` is idempotent across hooks, guardrails, and agents.
- `tome doctor` accurately reports hooks, guardrails regions, agent files,
  the privilege-escalation report, and personas; `--fix` repairs the safe
  cases without deleting unowned content.
- All Phase 1–5 success criteria still hold.

---

# Resolved decisions

| Question | Decision |
|---|---|
| Real hooks scope | Claude Code only (Codex/Gemini real hooks deferred) |
| Hook target file | `.claude/settings.local.json` (gitignored, local-scoped) |
| `${CLAUDE_PLUGIN_ROOT}` / `${CLAUDE_PLUGIN_DATA}` | Rewritten to absolute paths at copy time; other `${CLAUDE_*}` left for CC runtime |
| Hook add semantics | Exact structural match → skip; else append under event |
| Hook removal semantics | Exact structural match → remove; else skip (never delete unowned/edited hooks) |
| Hook provenance | None; re-derive and structurally match (no sidecar, no markers) |
| Guardrails source | `plugin/hooks/GUARDRAILS.md`, verbatim copy |
| Guardrails placement | Per-plugin marker region in the rules file; Cursor → sibling `TOME_GUARDRAILS.md` |
| Guardrails markers | `<!-- START GUARDRAILS: <catalog>:<plugin> -->` … `END` (Tome's "catalog", not "marketplace") |
| Re-sync behaviour | Overwrite content between existing markers in place |
| Claude Code guardrails suppression | Any `hooks.json` present → guardrails not rendered to `CLAUDE.md` for that plugin (simple version, no per-event merge) |
| Guardrails reconciliation scope | Per file (like the Phase 4 rules block) |
| Native agent harnesses | Claude Code, Codex, Cursor, OpenCode |
| Gemini/Antigravity agents | No native files; guardrails + (optional) personas only |
| Agent format | MD+YAML (CC/Cursor/OpenCode); TOML with `developer_instructions` body (Codex) |
| Cursor/OpenCode `.claude/agents/` cross-read | Not relied upon; emit native files |
| Field translation | Pass through only fields the harness supports or that map to a supported field; drop the rest |
| `model` value mapping | Same-vendor only; drop when no sane target (`opus`→`anthropic/claude-opus-4.7` ok; never `opus`→`gpt-5.1`) |
| Agent filename | Always `<plugin>__<name>.<ext>` (sole provenance) |
| Provenance frontmatter key | None — unknown keys risk harness parse breakage |
| Removal | Glob `<plugin>__*.<ext>` per harness agent dir |
| Emitted/registered agent name | Clean `<name>`; `<plugin>-<name>` only on cross-plugin clash |
| OpenCode name wart | Name derives from filename → always `<plugin>__<name>` |
| Plugin-agent privilege ban | Pass `hooks`/`mcpServers`/`permissionMode` through by default (Tome advantage over native plugin manager) |
| Privilege visibility | `tome doctor` reports agents carrying those fields |
| Privilege strip | `strip_plugin_agent_privileges` workspace/global config, default off |
| Personas | Off by default; `expose_agents_as_personas` workspace/global flag; global exposure (MCP prompts aren't harness-scoped) |
| Persona `<name>` source | Frontmatter `name` first, else agent filename stem |
| Persona naming | `<name>-persona`; `<plugin>-<name>-persona` only on clash |
| `drop-persona` | Single global, unnamespaced prompt |
| Persona substitution | Reuse Phase 5 pipeline (catch-all `args`, `ARGUMENTS:` fallback) |
| Persona statefulness | Advisory; documented as not-enforced |
| Agent indexing | `kind = 'agent'` in `skills` table, `searchable = 0`, no new columns; for provenance + collision detection |
| Hook indexing | None (config artifacts, filesystem-reconciled) |
| Claude Code rules/guardrails sink | `CLAUDE.md`, not `AGENTS.md` (Phase 4 correction) |
| `@AGENTS.md` courtesy scaffolding | Skipped |

---

# Phase 7 preview

Out of scope here, signposted:

- **Real hooks for Codex** (stable, JSON-shaped, `command` handler — the
  lowest-effort "real hooks #2" given existing full Codex agent support)
  and possibly Gemini (which even aliases `CLAUDE_PROJECT_DIR`).
- **Semantic search over agents** (`kind = 'agent'` rows are already
  indexed; embedding + `search_skills` exposure is the increment).
- Per-event guardrails/JSON-hook merging for Claude Code (instead of
  whole-file suppression).
- Server-side shell execution (`` !`cmd` ``) — the long-deferred Phase 5
  carryover.
- `_DATA` directory lifecycle (orphan detection, `--purge`).
- Additional harnesses (Gemini/Antigravity native agents if the platform
  stabilises, Copilot, Goose, Cline, Aider).
- Release tooling, dogfooding pass, public install path.
