# Phase 6 — Native agent translation

Authoritative contract for emitting native agent files across the four natively-supported harnesses (claude-code, codex, cursor, opencode) from a plugin's `agents/*.md` sources (Markdown + YAML frontmatter). Per FR-030–FR-037 and FR-040–FR-043; research R-7/R-8/R-9/R-14; PRD §2.1/§2.2.

Gemini and Antigravity are NOT natively-supported here — they get guardrails and optional personas only (PRD non-goals). Their `supports_native_agents()` returns the trait default `false`.

## Per-harness emission table

`translate_agent(canonical)` on each harness module declares its agent directory, file format, and body placement (FR-030/031/033). All four are written at **project scope** under the bound project root; native files are always emitted, never relying on a harness's cross-read of `.claude/agents/` (FR-031).

| Harness | `agent_dir(project)` | `agent_format()` | Body placement |
|---|---|---|---|
| `claude-code` | `.claude/agents/` | `MarkdownYaml` | file body |
| `codex` | `.codex/agents/` | `Toml` | triple-quoted `developer_instructions` TOML string (FR-033, R-14) |
| `cursor` | `.cursor/agents/` | `MarkdownYaml` | file body |
| `opencode` | `.opencode/agent/` | `MarkdownYaml` | file body (note: singular `agent/`) |

All directory paths are verified against current harness documentation at implementation time (the Phase 4 ecosystem caveat). Codex TOML is emitted via `toml_edit` (existing dep, R-14) so triple-quote escaping and key/order/comment preservation are correct; no hand-rolled TOML string building.

## Field mapping (FR-032)

Pass through ONLY the frontmatter fields the target harness supports, or that map cleanly to a field it supports. **Drop everything else.** No field is ever passed through on the assumption the harness tolerates unknown keys — a stray key can break a harness's parser. Every dropped field is recorded on `TranslatedAgent.dropped_fields` for the doctor surface (FR-090, informational).

Canonical → per-harness field map (Claude Code-normalized canonical names), reproduced from PRD §2.1:

| Canonical field | Claude Code | Codex (TOML) | Cursor | OpenCode |
|---|---|---|---|---|
| `name` | `name` | `name` | `name` | (filename-derived) |
| `description` | `description` | `description` | `description` | `description` (required — FR-035 fallback) |
| system-prompt body | body | `developer_instructions` | body | body |
| `model` | `model` | `model` | `model` | `model` |
| `tools` (allowlist) | `tools` | — (via sandbox/mcp) → drop | `tools` | `permission` (per-tool) |
| `disallowedTools` | `disallowedTools` | — drop | — drop | `permission: deny` |
| read-only intent | `tools` subset | `sandbox_mode = "read-only"` | `readonly: true` | `permission` (edit/bash → ask/deny) |
| `mode` (primary/subagent) | — (implicit) | — | — | `mode` (default `subagent`) |
| `temperature` | — drop | — drop | — drop | `temperature` |
| `effort`, `maxTurns`, `skills`, `memory`, `isolation`, `initialPrompt`, `background`, `permissionMode`, `mcpServers`, `hooks` | native | mostly drop / Codex-specific | drop | `steps` for `maxTurns`; rest drop |

The privileged fields (`hooks`, `mcpServers`, `permissionMode`) are passed through to **Claude Code** by default (a capability advantage; FR-050) and stripped only when `strip_plugin_agent_privileges` is set — see `settings-p6.md`. Other harness dialects do not carry those three fields at all.

The table is the intended core; exact per-harness supported-field sets are confirmed against current harness docs at implementation time (Phase 4 caveat).

## Value mapping — `model` (FR-034/037, R-8)

Field *names* port; field *values* often do not. Value mapping is **same-vendor only**, driven by a per-harness `ModelAliasTable` (`(harness, source_value) -> Option<harness_native_id>`) declared as a single source of truth in the harness modules. A source value with no entry for a given harness is **dropped** (harness default inherited); an `inherit`-style value is dropped everywhere. Dropped `model` fields are recorded for diagnostics (NFR-005).

**Cross-vendor and strongest-to-strongest heuristics are FORBIDDEN** (FR-034) — they rot and surprise users. No emitted agent file ever carries a cross-vendor model identifier.

This table is the named artefact **SC-002 verifies against** and is pinned here:

| Source canonical value | claude-code | codex | cursor | opencode |
|---|---|---|---|---|
| `opus` | `opus` (native) | **DROP** (never an OpenAI id) | Cursor's Anthropic id where one exists, else DROP | `anthropic/claude-opus-4.7` |
| `sonnet` | `sonnet` (native) | **DROP** | Cursor's Anthropic id where one exists, else DROP | `anthropic/claude-sonnet-4.7` |
| `haiku` | `haiku` (native) | **DROP** | Cursor's Anthropic id where one exists, else DROP | `anthropic/claude-haiku-4.7` |
| `inherit` | **DROP** | **DROP** | **DROP** | **DROP** |
| (any value with no same-vendor target) | per native support | **DROP** | **DROP** unless a same-vendor id exists | **DROP** unless a same-vendor id exists |

`opus → codex` is DROP, never `gpt-5.1` or any OpenAI id. `opus → opencode` is `anthropic/claude-opus-4.7` (same vendor, legitimate). **Ecosystem caveat**: the exact harness-native identifiers above are confirmed against current harness documentation at implementation time; the *policy* (same-vendor-only, drop-on-no-target) is fixed.

## Read-only intent reconstruction (FR-036)

Read-only intent is reconstructed per harness from the source agent's canonical tool posture (`tools` allowlist and/or `disallowedTools`) by a documented inference rule:

> **Rule**: an agent is read-only when its effective tool set contains no write/edit/execute-class tool — i.e. the allowlist (if present) excludes every write/edit/execute tool, OR the disallowed list denies all of them.

The inferred intent is expressed in each harness's own mechanism: Codex `sandbox_mode = "read-only"`; Cursor `readonly: true`; OpenCode per-tool `permission` entries (edit/bash → `ask`/`deny`); Claude Code via the carried-through `tools` subset. Where the rule is **indeterminate** (mixed posture, no allowlist) or the harness cannot express the intent, the field is **dropped** (harness default inherited) and the drop recorded for diagnostics.

## OpenCode specifics (FR-035)

- A translated agent defaults to `mode: subagent` (source agents are subagents).
- `description` is **required** by OpenCode. When the source lacks one, fall back to the **first non-empty line of the body, trimmed**, and record the fallback for diagnostics (FR-035, surfaced via doctor). When the body has no non-empty line, use the documented placeholder: `"Agent <name> (no description provided)."`.

## Filename, provenance, displayed name, removal (FR-040–FR-043, R-9)

- **Filename is always `<plugin>__<name>.<ext>`** (e.g. `midnight-expert__reviewer.md`, `…__reviewer.toml`). This is the **sole provenance mechanism** — Tome adds **no provenance frontmatter key** (an unknown key risks breaking a harness parser; trampling a user agent that happens to use the `<plugin>__*` convention is the accepted lower risk). `.ext` is `md` for MarkdownYaml harnesses, `toml` for Codex. A shared `pub(crate)` filename builder is the single source of truth (R-19).
- **Displayed / registered name** uses the clean `<name>` normally, and the plugin-prefixed `<plugin>-<name>` form ONLY when two or more enabled plugins in the workspace clash on `<name>`, applied to the clashing agents only (FR-041). The filename stays `<plugin>__<name>` regardless of clash.
- **OpenCode** derives the agent name from the filename, so OpenCode agents are necessarily named `<plugin>__<name>` (the prefix cannot be hidden — accepted wart; FR-042).
- **Removal** globs `<plugin>__*.<ext>` in each natively-supporting harness's `agent_dir`, removing files for plugins no longer enabled or harnesses no longer in the effective list, leaving other plugins' agents untouched (FR-043). Reconciled on `tome harness sync` and on `tome plugin disable`.

## Clash set (FR-072)

The clash set is the set of `<name>` values held by **≥ 2 agent-kind rows enabled in the resolved workspace** (joined through the workspace-enablement junction), **computed once per sync** from the workspace scope. The same clash set governs harness-facing displayed-name prefixing (FR-041) and persona naming (`agent-personas.md`, FR-061) **identically** across all of the workspace's bound projects. Exposed via a shared `pub(crate)` clash-set query (R-19).

## Failure modes

| Trigger | Variant | Exit code |
|---|---|---|
| Malformed agent frontmatter, or translation failed for a target harness | `AgentTranslationFailed { agent }` | 45 |

Agent translation failure surfaces with the agent named; the agent is **not partially emitted** — each agent file write is all-or-nothing (atomic, mode-preserving, symlink-refusing per the Phase 4 discipline; FR-084). Sibling components of the same plugin still reconcile where possible (loud-but-isolated; FR-084 forward progress).

## Tests

| Behaviour | Test |
|---|---|
| Agent emitted to all four harness dirs in correct format | `tests/agent_translation.rs::emits_native_file_per_harness` |
| Codex body → triple-quoted `developer_instructions` | `tests/agent_translation.rs::codex_body_in_developer_instructions` |
| `model: opus` mapped for opencode, dropped for codex | `tests/agent_translation.rs::model_same_vendor_only` |
| No emitted file carries a cross-vendor model id (SC-002) | `tests/agent_translation.rs::never_cross_vendor_model` |
| Unsupported field dropped + recorded | `tests/agent_translation.rs::unsupported_field_dropped` |
| OpenCode defaults `mode: subagent` + description fallback | `tests/agent_translation.rs::opencode_mode_and_description_fallback` |
| Read-only intent inferred + expressed per harness | `tests/agent_translation.rs::readonly_intent_reconstructed` |
| Clash → displayed name plugin-prefixed for clashing only | `tests/agent_translation.rs::displayed_name_prefixed_on_clash` |
| Removal globs `<plugin>__*.<ext>` | `tests/agent_translation.rs::removal_globs_plugin_prefix` |
| Idempotent re-sync rewrites nothing | `tests/agent_translation.rs::sync_idempotent` |
