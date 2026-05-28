# Phase 6 Quickstart — Hooks and Agents

**Branch**: `006-phase-6-hooks-agents` | **Date**: 2026-05-28

An end-to-end walkthrough that exercises every Phase 6 success criterion (SC-001…SC-011). Assumes Phase 1–5 are shipped (catalogs, plugins, workspaces, harness sync, MCP server with prompts + substitution). Run from a fresh `<home>/.tome/`.

The example plugin `demo-pack` ships:
- `agents/reviewer.md` (frontmatter `name: reviewer`, `description`, `model: opus`, a `tools` allowlist, and a privileged `permissionMode`), plus a body.
- `hooks/hooks.json` with a `PreToolUse` hook whose `command` references `${CLAUDE_PLUGIN_ROOT}/scripts/guard.sh`.
- `hooks/GUARDRAILS.md` with two prose constraints.

## 0. Set up a multi-harness workspace

```sh
tome catalog add <url-of-catalog-containing-demo-pack>
tome workspace init demo
# make demo's effective harness list include all five:
tome harness use claude-code --scope workspace
tome harness use codex      --scope workspace
tome harness use cursor     --scope workspace
tome harness use gemini     --scope workspace
tome harness use opencode   --scope workspace
cd /path/to/project && tome workspace use demo      # binds the project
tome plugin enable demo-pack/demo-pack              # triggers reconciliation
```

## 1. Native agents across four harnesses (SC-001, SC-002, SC-003) — US1

After enable + sync, each natively-supporting harness has the agent, plugin-namespaced:

```sh
cat .claude/agents/demo-pack__reviewer.md     # MD+YAML, body in file body
cat .codex/agents/demo-pack__reviewer.toml    # TOML, body in developer_instructions = """..."""
cat .cursor/agents/demo-pack__reviewer.md      # MD+YAML
cat .opencode/agent/demo-pack__reviewer.md     # MD+YAML, mode: subagent, name is filename-derived
ls .gemini/                                     # NO native agent file (Gemini = guardrails + optional persona only)
```

- **SC-001**: all four files present, plugin-namespaced, body in the harness-appropriate location, harness defaults applied to omitted fields.
- **SC-002**: `model: opus` → `.opencode/agent/...` shows `anthropic/claude-opus-4.7` (same-vendor map); `.codex/agents/...` has **no** `model` key (dropped — never an OpenAI id). No emitted file carries a cross-vendor identifier. Verifiable against the `ModelAliasTable` in `contracts/agent-translation.md`.
- A dropped field (e.g. an unsupported `isolation`) appears in `tome doctor --json` under the agent's `dropped_fields`; nothing crashes.

## 2. Disable + idempotence (SC-003) — US1

```sh
tome plugin disable demo-pack/demo-pack
ls .claude/agents/ .codex/agents/ .cursor/agents/ .opencode/agent/   # demo-pack__* gone; other plugins' agents remain
tome plugin enable demo-pack/demo-pack
tome harness sync && tome harness sync                                # second run is a no-op
```

- **SC-003**: disable removes exactly `demo-pack__*` from every harness; a second `sync` rewrites/removes nothing across hooks, guardrails, and agents (capture mtimes, re-run, assert unchanged — the `MTIME_TICK` idempotence pattern).

## 3. Real hooks for Claude Code (SC-004, SC-005) — US2

```sh
cat .claude/settings.local.json     # hooks.PreToolUse[…].command has ${CLAUDE_PLUGIN_ROOT} rewritten to the absolute installed-plugin path
                                     # any ${CLAUDE_PROJECT_DIR}/${CLAUDE_SESSION_ID} left verbatim
ls .claude/settings.json 2>/dev/null # NOT written by Tome
```

- **SC-004**: the rewrite resolves the plugin-root variable to the absolute path; other Claude-native variables are intact; the committed settings file is untouched.
- **SC-005**: `tome harness sync` again adds no duplicate; a user-authored identical hook is not duplicated; `tome plugin disable demo-pack/demo-pack` removes only the structurally-matching plugin hooks and leaves a user-edited copy in place; empty event arrays are pruned, an empty `hooks` object is kept.

## 4. Guardrails everywhere + the Phase 4 correction (SC-006, SC-007) — US3

With a guardrails-only plugin `prose-pack` (ships `GUARDRAILS.md`, no `hooks.json`) also enabled:

```sh
grep -n "START GUARDRAILS: " CLAUDE.md AGENTS.md .cursor/rules/TOME_GUARDRAILS.md
```

- **SC-006**: a `prose-pack` region appears in `CLAUDE.md`, the shared `AGENTS.md`, and the Cursor sibling. For `demo-pack` (ships both `GUARDRAILS.md` and `hooks.json`): its region is present in `AGENTS.md` (for codex/gemini/opencode) and **absent** from `CLAUDE.md` (suppressed by real hooks). Two guardrails-shipping plugins yield two distinct regions in `AGENTS.md`. Disabling one removes only its region; re-sync overwrites between markers in place.
- **SC-007**: the Phase 4 rules-include block is in `CLAUDE.md`, not `AGENTS.md`; the shared `AGENTS.md` has one block for the other harnesses; both resolve the same `.tome/RULES.md` (no duplicated content). Verify `grep -n "@.tome/RULES.md" CLAUDE.md AGENTS.md`.

## 5. Personas (SC-008) — US4

```sh
# default: personas off
tome mcp & ; (list prompts via the harness)   # no <name>-persona, no drop-persona
# enable globally
tome harness ... # set expose_agents_as_personas = true at workspace/global scope (settings.toml)
# restart the MCP server, then list prompts:
#   reviewer-persona   (or demo-pack-reviewer-persona on cross-plugin clash)
#   drop-persona       (exactly one, global, unnamespaced)
```

- **SC-008**: off ⇒ no persona prompts. On ⇒ each agent appears as `<name>-persona` (clash-prefixed where required) plus exactly one global `drop-persona`. `prompts/get` for `reviewer-persona` returns the wrapped, frontmatter-stripped body with Phase 5 `${TOME_*}`/`${TOME_ENV_*}` substitution applied and a free-form `args` resolved through the Phase 5 pipeline.

## 6. Privilege governance (SC-009) — US5

```sh
grep -n "permissionMode" .claude/agents/demo-pack__reviewer.md   # present by default
tome doctor --json | jq '.agents.privilege_escalation'           # demo-pack/reviewer listed, grouped by plugin
# now set strip_plugin_agent_privileges = true (workspace or global), re-sync:
grep -n "permissionMode" .claude/agents/demo-pack__reviewer.md   # absent; no plugin edit required
```

- **SC-009**: privileged fields emitted intact by default and reported by doctor; with the strip setting on (workspace/global), the same agent is emitted without `hooks`/`mcpServers`/`permissionMode`.

## 7. Doctor + regressions (SC-010, SC-011)

```sh
tome doctor            # human: hooks contributed/missing; guardrails present/orphaned/suppressed; agents present/orphaned + dropped fields + privilege report; personas (if on)
tome doctor --json     # byte-stable wire shapes for every new record
tome doctor --fix      # re-renders stale guardrails, re-emits missing agents, removes orphaned <plugin>__* files; never removes an unowned hook or user content
tome plugin show demo-pack/demo-pack   # lists agents + "ships hooks.json / GUARDRAILS.md" + resolved persona name (if personas on)
cargo test             # all Phase 1–5 suites still green
```

- **SC-010**: doctor accurately reports all four subsystems; `--fix` repairs only the safe cases.
- **SC-011**: every Phase 1–5 success criterion still holds; agents never appear in `search_skills`; the existing search/read surfaces are unchanged.

## Development setup

This is an existing Rust project; tooling is already complete (see § below in the plan / `.sdd/codebase/`). The standard loop:

```sh
cargo build
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
typos
cargo test
```

Git hooks are versioned under `.githooks/` (`git config core.hooksPath .githooks`).
