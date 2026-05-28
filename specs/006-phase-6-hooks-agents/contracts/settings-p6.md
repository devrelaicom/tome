# Phase 6 — Settings additions

Authoritative contract for the two new configuration settings introduced in Phase 6. Per FR-050/052/053 and FR-060/067; research R-12; PRD §2.3/§2.4. Contrast with the `harnesses` composition grammar (`settings-composition.md`).

## The two new fields

Two typed `bool` fields, both **defaulting to `false`**, added to each of the three Tome-owned settings structs — `GlobalSettings`, `WorkspaceSettings`, and `ProjectMarkerConfig` (`src/settings/mod.rs`). All three structs sit on the **strict** side of the strictness boundary (`#[serde(deny_unknown_fields)]`), unchanged by Phase 6 (FR-053, NFR-010).

| Field | Default | Effect |
|---|---|---|
| `expose_agents_as_personas` | `false` | Expose each enabled agent as a `<name>-persona` MCP prompt plus a single global `drop-persona` (FR-060). Effect spec: `agent-personas.md`. |
| `strip_plugin_agent_privileges` | `false` | Strip `hooks` / `mcpServers` / `permissionMode` from emitted **Claude Code** agent files (FR-052). |

Because the fields default to `false` and the structs carry `#[serde(default)]` per field, an existing settings file that omits both keys parses unchanged — no migration of settings files is required.

## Layering (FR-053, R-12)

Both scalars resolve by a **first-declarer-wins priority walk** across **project → workspace → global**: the nearest scope that **declares** the field wins; a project `false` overrides a global `true`. "Declares" means the key is present in that scope's settings file (Phase 4's optional-field model — an absent key falls through to the next scope; a present key, even `false`, terminates the walk).

```text
resolve_scalar(field, project_marker, workspace_settings, global_settings):
  if project_marker.<field> is Some(v):   return v   // declared at project → wins
  if workspace_settings.<field> is Some(v): return v  // else declared at workspace → wins
  if global_settings.<field> is Some(v):    return v  // else declared at global → wins
  return false                                        // default when nowhere declared
```

This is **NOT** the harnesses composition reference/exclusion grammar. The composition forms (`"[workspace]"`, `"[global]"`, `"!<name>"`, cycle detection — see `settings-composition.md`) are meaningless for a scalar boolean: there is no list to compose, union, or subtract. Specifically:

| Aspect | `harnesses` (list) | `expose_agents_as_personas` / `strip_plugin_agent_privileges` (scalar) |
|---|---|---|
| Resolution | priority walk that **stops at first declarer**, then composition references expand from that declarer's list (FR-441/449) | priority walk that **stops at first declarer**, returns that scalar; no expansion |
| Reference/exclusion grammar | yes (`[workspace]`, `[global]`, `!<name>`) | none — bracketed/`!` forms are not parsed |
| Cycles | possible (exit 17) | impossible (no references) |
| "project false overrides global true" | n/a (lists union) | yes — the defining behaviour |

The on-disk resolution above governs the **agent-emission (sync) time** value for `strip_plugin_agent_privileges`. For `expose_agents_as_personas`, the *running MCP server's* effective value is read from the server's single startup scope, not project-layered per session — see `agent-personas.md` and FR-067.

## `strip_plugin_agent_privileges` effect (FR-050/052, PRD §2.3)

By **default** (`false`), Tome passes the three privileged frontmatter fields — `hooks`, `mcpServers`, `permissionMode` — through to emitted **Claude Code** agent files. This is a deliberate capability advantage of installing a plugin's agents via Tome over Claude Code's native plugin manager, which forbids those fields for plugin-shipped subagents: an agent file placed in `.claude/agents/` as a project/user file supports the full set, including those three, and Anthropic's own guidance is "copy the file into `.claude/agents/` and own it" — which is exactly what Tome does (PRD §2.3).

When the setting resolves to `true` (at any scope per the layering above), Tome **strips those three fields from emitted Claude Code agent files**, restoring native-plugin-manager parity. So a security-conscious user can enforce it org-wide via the global scope.

- The strip is a **no-op** for agents that carry none of the three privileged fields (FR-052).
- The strip concerns **Claude Code emission specifically**. Other harness dialects (Codex, Cursor, OpenCode) do not carry `hooks` / `mcpServers` / `permissionMode` in the first place (those canonical fields are dropped during translation per `agent-translation.md`), so the setting has no effect on their emitted files.
- The escalation surface (agents carrying any of the three fields) is auditable via the doctor `PrivilegeEscalationReport` regardless of this setting's value (FR-051; `doctor-extensions-p6.md`). The pass-through is visible and reversible (NFR-006).

## `expose_agents_as_personas` effect

See `agent-personas.md`. Summary: when `true` (resolved against the MCP server startup scope per FR-067), the MCP server exposes each enabled agent as a `<name>-persona` prompt plus one global `drop-persona`; when `false`, the prompt surface is exactly the Phase 5 surface (NFR-008).

## Config-settings summary

| Setting | Default | Scope layering | Effect |
|---|---|---|---|
| `expose_agents_as_personas` | `false` | first-declarer-wins project → workspace → global (effective value read from MCP server startup scope, FR-067) | personas exposed to all connected clients (`agent-personas.md`) |
| `strip_plugin_agent_privileges` | `false` | first-declarer-wins project → workspace → global (resolved at agent-emission/sync time) | strips `hooks`/`mcpServers`/`permissionMode` from emitted Claude Code agents |

## Tests

| Behaviour | Test |
|---|---|
| Both fields default to `false` when absent | `tests/settings_p6.rs::defaults_false_when_absent` |
| Strict struct rejects an unknown key | `tests/settings_p6.rs::deny_unknown_fields_preserved` |
| First-declarer-wins: project `false` overrides global `true` | `tests/settings_p6.rs::project_false_overrides_global_true` |
| Fall-through when project + workspace absent → global declares | `tests/settings_p6.rs::falls_through_to_global` |
| Privileged fields passed through by default (Claude Code) | `tests/agent_privilege.rs::passthrough_by_default` |
| Strip removes the three fields when set (Claude Code) | `tests/agent_privilege.rs::strip_removes_three_fields` |
| Strip is a no-op for non-privileged agents | `tests/agent_privilege.rs::strip_noop_when_none` |
| Strip does not affect non-Claude-Code emission | `tests/agent_privilege.rs::strip_claude_code_only` |
