# Phase 6 Data Model — Hooks and Agents

**Branch**: `006-phase-6-hooks-agents` | **Date**: 2026-05-28
**Input**: [spec.md](./spec.md), [plan.md](./plan.md), [research.md](./research.md)

Types are described at the data-model level (names, fields, relationships, invariants). Exact Rust signatures land in the contracts and the implementation. Phase 6 adds **no new SQLite columns or tables** — the only storage change is widening the free-text `kind` domain and the Rust `EntryKind` enum.

---

## 1. Entry kind widening

### `EntryKind` (`src/plugin/identity.rs`, existing — widened)

```
enum EntryKind { Skill, Command, Agent }   // Agent is new
```

- `FromStr` / `Display` gain the `"agent"` mapping. **Every exhaustive match** over `EntryKind` is updated (FR-070a): the per-kind count aggregation behind `tome plugin list` / `tome plugin show`, the doctor entry-count surface, and any MCP dispatch. No match may regress to a catch-all (canonical-enum-dispatch discipline, Phase 5).
- Storage: the `skills.kind` column (TEXT, free-text) admits `'agent'`. Agent rows: `searchable = 0` always, `user_invocable = 0` (agents are not prompts; personas are a separate specialised path), embedding skipped. Identity = `(catalog, plugin, kind, name)` — unchanged constraint.

### `AgentRow` (conceptual — a `skills`-table row with `kind='agent'`)

| Field | Source | Notes |
|---|---|---|
| `catalog`, `plugin` | enrolment | provenance |
| `kind` | `'agent'` | the new domain value |
| `name` | frontmatter `name`, else filename stem | drives clash detection (FR-072) |
| `description` | frontmatter `description`, else first non-empty body line | informational |
| `content_hash` | hash of the agent `.md` | reindex diffing |
| `searchable` | `0` | never in `search_skills` (FR-070) |
| embedding | (skipped) | agents not embedded in v1 |

---

## 2. Hooks (Claude Code only)

### `HooksStrategy`
```
enum HooksStrategy { RealJson, GuardrailsOnly }
```
`claude-code → RealJson`; all others `→ GuardrailsOnly`.

### `HookEntry` (conceptual)
A `serde_json::Value` object keyed under an event in the plugin's `hooks/hooks.json`, **after** the two-variable path rewrite (`${CLAUDE_PLUGIN_ROOT}`, `${CLAUDE_PLUGIN_DATA}` → absolute paths; all other `${CLAUDE_*}` verbatim). Ownership is established by re-derivation + deep structural equality — no stored representation, no sidecar (NFR-003).

### Merge invariants
- Target: `.claude/settings.local.json` under `hooks` (created with a single `hooks` object if absent). The committed `.claude/settings.json` is never written.
- Add: append a rewritten entry under its event only if no deep-equal entry exists there; else skip (idempotent, FR-004).
- Remove: delete the deep-equal entry; skip if absent (never remove non-matching/edited entries, FR-005).
- Prune empty event arrays; keep an otherwise-empty `hooks` object (FR-006).

---

## 3. Guardrails

### `GuardrailsTarget`
```
enum GuardrailsPlacement { InFileRegion { file: PathBuf }, StandaloneSibling { file: PathBuf } }
struct GuardrailsTarget { placement: GuardrailsPlacement, suppress_if_hooks_present: bool }
```
- claude-code → `InFileRegion { CLAUDE.md }`, `suppress_if_hooks_present = true`.
- codex / opencode → `InFileRegion { AGENTS.md }`; gemini → `InFileRegion { AGENTS.md else GEMINI.md }`; all `suppress_if_hooks_present = false`.
- cursor → `StandaloneSibling { .cursor/rules/TOME_GUARDRAILS.md }`, `suppress_if_hooks_present = false`.

### `GuardrailsRegion` (conceptual)
Marker-delimited block: `<!-- START GUARDRAILS: <catalog>:<plugin> -->` … verbatim `GUARDRAILS.md` … `<!-- END GUARDRAILS: <catalog>:<plugin> -->`. Distinct from the Phase 4 `tome:begin/end` rules block. The `<catalog>:<plugin>` marker is the sole per-plugin removal key (filesystem-inferred state, NFR-004).

### Reconciliation invariants
- Deterministic placement: rules-include block first, then one region per contributing plugin in lexicographic `<catalog>:<plugin>` order (FR-011, idempotence).
- Overwrite-between-markers in place; remove orphaned regions (plugin disabled / harness gone / on Claude Code, plugin now ships `hooks.json`); delete the Cursor sibling when empty (FR-014/015).
- Suppression input (Claude Code): computed before guardrails reconciliation in the same sync (FR-016).

---

## 4. Agents (native translation)

### `AgentFormat`
```
enum AgentFormat { MarkdownYaml, Toml }
```
claude-code / cursor / opencode → `MarkdownYaml`; codex → `Toml`.

### `CanonicalAgent` (parsed source `agents/*.md`)
| Field | Notes |
|---|---|
| `name` | frontmatter `name`, else filename stem |
| `description` | optional |
| `body` | system-prompt Markdown |
| `model` | optional canonical value (`opus`, `inherit`, …) |
| `tools` / `disallowed_tools` | tool posture → read-only inference (FR-036) |
| privileged: `hooks`, `mcp_servers`, `permission_mode` | passed through to Claude Code by default (FR-050); stripped under config (FR-052) |
| other frontmatter | dropped unless it maps cleanly (FR-032) |

### `TranslatedAgent` (per-harness emission result)
| Field | Notes |
|---|---|
| `dir` | `agent_dir(project)` for the harness |
| `filename` | always `<plugin>__<name>.<ext>` (FR-040) |
| `displayed_name` | clean `<name>`, or `<plugin>-<name>` on clash (FR-041); OpenCode always `<plugin>__<name>` (FR-042) |
| `format` | `MarkdownYaml` | `Toml` |
| `rendered` | body in file body, or in a triple-quoted `developer_instructions` TOML string (FR-033) |
| `dropped_fields` | recorded for diagnostics (FR-032/034/036) |

### `ModelAliasTable` (per-harness, same-vendor only — FR-037)
`(harness, source_value) -> Option<harness_native_id>`; `None` ⇒ drop the field. Pinned in `contracts/agent-translation.md`; the named artefact SC-002 verifies against. Examples: `(opencode, "opus") -> "anthropic/claude-opus-4.7"`; `(codex, "opus") -> None`; `(*, "inherit") -> None`.

### Clash set (FR-072)
The set of `<name>` values held by ≥ 2 agent-kind rows enabled in the resolved workspace, computed once per sync. Governs filename display-prefix, harness-facing displayed-name prefix, and persona naming identically across the workspace's bound projects.

### Removal
Glob `<plugin>__*.<ext>` in each natively-supporting harness's `agent_dir`; remove for plugins no longer enabled / harnesses no longer in the effective list (FR-043).

---

## 5. Personas (MCP prompts, opt-in)

### `PersonaPrompt` (conceptual — a view onto an agent row when `expose_agents_as_personas` is on)
| Field | Notes |
|---|---|
| `prompt_name` | `<name>-persona`, or `<plugin>-<name>-persona` on clash (FR-061); subject to Phase 5 sanitisation/length/collision |
| `body` | agent body, frontmatter stripped, wrapped in the role-assumption template, Phase 5 built-in + env substitution applied (FR-062) |
| `arguments` | single catch-all `args` through the Phase 5 argument pipeline + ARGUMENTS append fallback (FR-062) |

### `drop-persona`
A single global, unnamespaced, **reserved** prompt name (FR-063): if any command/skill/persona would derive to `drop-persona`, the other entry is counter-suffixed and `drop-persona` stays unique.

### Collision namespace (FR-066)
Persona derived names join the **single** Phase 5 prompt-name collision namespace (union of command, skill, persona names). The agent-clash plugin prefix applies before the Phase 5 counter-suffix backstop.

### Scope resolution (FR-067)
`expose_agents_as_personas` is resolved against the MCP server's single startup scope (workspace, or global fallback). Project-scope layering has no effect on a running server.

---

## 6. Settings (`src/settings/mod.rs`, existing structs — extended)

Two `bool` fields, default `false`, added to **`GlobalSettings`, `WorkspaceSettings`, `ProjectMarkerConfig`** (all Tome-owned, `deny_unknown_fields`, strict):

| Field | Effect |
|---|---|
| `expose_agents_as_personas` | expose each enabled agent as a `<name>-persona` MCP prompt + a global `drop-persona` (FR-060) |
| `strip_plugin_agent_privileges` | strip `hooks`/`mcp_servers`/`permission_mode` from emitted Claude Code agents (FR-052) |

**Layering**: first-declarer-wins priority walk project → workspace → global (FR-053). NOT the `harnesses` composition reference/exclusion grammar.

---

## 7. Doctor extension records (`src/doctor/`, emit-only — wire-pinned)

| Record | Contents (FR-090) |
|---|---|
| `HooksReport` | per enabled plugin: hooks contributed to `settings.local.json`; expected-but-missing entries (drift from user edits) |
| `GuardrailsReport` | per target file: present `<catalog>:<plugin>` regions; orphaned regions; regions suppressed by JSON hooks (Claude Code) |
| `AgentsReport` | per harness: present + orphaned `<plugin>__*` files; dropped-field info (informational); the privilege-escalation grouping |
| `PrivilegeEscalationReport` | installed agents carrying `hooks`/`mcp_servers`/`permission_mode`, grouped by plugin (FR-051) |
| `PersonaReport` | when enabled: effective persona prompt list with resolved + clash-prefixed names |

`--fix` repairs only safe derivable cases: re-render stale guardrails regions, re-emit missing agent files, remove orphaned `<plugin>__*` files; never removes a non-matching hook entry, never deletes user-authored content (FR-091).

All records are emit-only `Serialize` types with byte-stable JSON wire-shape pins (NFR-011); no `deny_unknown_fields` (boundary is inputs only — Phase 5 convention).

---

## 8. Errors (`src/error.rs`, existing enum — +4 variants)

| Variant (illustrative) | Exit code | Trigger |
|---|---|---|
| `HookSpecParseError { path }` | 43 | malformed/unparsable plugin `hooks/hooks.json` (FR-092) |
| `HookSettingsWriteFailed { path }` | 44 | read/merge/write failure on `.claude/settings.local.json` (FR-092) |
| `AgentTranslationFailed { agent }` | 45 | malformed agent frontmatter or failed translation (FR-092) |
| `GuardrailsWriteFailed { path }` | 46 | guardrails render/write failure (FR-092) |

Closed-set discipline (constitution §II): no `Other`/`Unknown` arm; numbers pinned in `contracts/exit-codes-p6.md`; reuse none of the occupied codes (1–9, 13–37, 40–42, 50–54, 60–61, 70, 73–75). Variant names are illustrative; final names land in F1.

---

## 9. `HarnessModule` trait additions (`src/harness/mod.rs`, existing trait — extended)

| Method | Returns | Default |
|---|---|---|
| `hooks_strategy()` | `HooksStrategy` | `GuardrailsOnly` |
| `hook_settings_path(project)` | `Option<PathBuf>` | `None` |
| `guardrails_target(project)` | `GuardrailsTarget` | in-file region on the harness's rules-file target, no suppression |
| `supports_native_agents()` | `bool` | `false` |
| `agent_dir(project)` | `Option<PathBuf>` | `None` |
| `agent_format()` | `Option<AgentFormat>` | `None` |
| `translate_agent(canonical)` | `TranslatedAgent` | (only called when `supports_native_agents()`) |

Plus the Phase 4 correction: `claude_code`'s `rules_file_target` candidate list becomes `CLAUDE.md` > `.claude/CLAUDE.md` (drop `AGENTS.md`). `StubHarness` implements the new methods for tests (R-16).
