# Phase 4 Quickstart

**Branch**: `004-phase-4-refactor-harnesses` | **Date**: 2026-05-14 | **Plan**: [plan.md](./plan.md)

End-to-end smoke test of Phase 4: refactor to central architecture, create a named workspace, bind a project, get cross-harness integration.

## Prerequisites

- Tome v0.4.0 installed (`cargo install --path .` from the Phase 4 branch).
- Network connection (for the first-run model downloads — embedder ~66 MB, reranker ~280 MB, summariser ~400 MB).
- One supported harness installed on the local machine (any of: Claude Code, Codex CLI, Gemini CLI, Cursor, OpenCode). The presence check is per-user directory existence (`~/.claude/`, `~/.codex/`, etc.).
- For developers upgrading from Phase 3: manually wipe `~/.local/share/tome/`, `~/.config/tome/`, and `~/.local/state/tome/`. Phase 4 does NOT detect or migrate Phase 3 state.

## 1. First-run bootstrap

```bash
tome models download
```

Downloads all three models into `<home>/.tome/models/`. Idempotent — re-runs skip already-installed models.

```bash
tome status
```

Reports embedder, reranker, summariser, index, and (eventually) drift state. Exits 0 on healthy bootstrap. Verify:

- `<home>/.tome/index.db` exists.
- `<home>/.tome/models/{bge-small-en-v1.5,bge-reranker-base,qwen2.5-0.5b-instruct}/` exist.
- The `global` workspace row is present.

## 2. Create a named workspace

```bash
tome workspace init my-stack --inherit-global
```

Creates `<home>/.tome/workspaces/my-stack/` containing:

- `settings.toml` — workspace's name + (with `--inherit-global`) the global workspace's catalog list copied over.
- `RULES.md` — empty placeholder; populated on first summarisation.

Verify:

```bash
tome workspace list
```

Tabular output names both workspaces (`global` and `my-stack`); `my-stack` has the inherited catalog count.

## 3. Enrol a catalog and enable a plugin

```bash
tome catalog add github.com/example/skills --workspace my-stack
```

Clones the catalog into `<home>/.tome/catalogs/<sha256>/` (or reuses if any workspace already has it cloned). Inserts a `workspace_catalogs` row for `my-stack`.

```bash
tome plugin enable example/awesome --workspace my-stack
```

Parses SKILL.md files, embeds skills into the central DB, inserts `workspace_skills` rows for `my-stack`. Triggers summary regeneration — both short and long summaries are computed and cached in `<home>/.tome/workspaces/my-stack/settings.toml` under `[summaries]`.

Verify:

```bash
tome workspace info my-stack
```

Reports: 1 catalog enrolled, 1 plugin enabled, N skills indexed, 0 bound projects, summary lengths populated.

## 4. Configure harnesses at the workspace scope

```bash
tome harness use claude-code --scope workspace --workspace my-stack
```

Updates `<home>/.tome/workspaces/my-stack/settings.toml`'s `harnesses` array to include `"claude-code"`. The change does NOT affect any other workspace.

(Alternatively: `tome harness use claude-code --scope global` to apply to every workspace that walks-through to global.)

## 5. Bind a project

```bash
cd ~/projects/my-app
tome workspace use my-stack
```

Creates `<project>/.tome/`:

- `config.toml` — `workspace = "my-stack"`.
- `RULES.md` — copy of `<home>/.tome/workspaces/my-stack/RULES.md`.

Inserts `workspace_projects(project_path = canonical(~/projects/my-app), workspace_id = id("my-stack"))`. Automatically runs the harness sync algorithm — for each harness in the effective list (here: just `claude-code`), writes the Tome block to the rules-file target and the Tome entry to the MCP config.

Verify:

```bash
ls -la .tome/
# .tome/config.toml exists
# .tome/RULES.md exists

cat AGENTS.md  # or CLAUDE.md, whichever Claude Code's precedence picks
# Contains:
#   <!-- tome:begin -->
#   @.tome/RULES.md
#   <!-- tome:end -->

cat .claude/settings.json | jq '.mcpServers.tome'
# {
#   "command": "tome",
#   "args": ["mcp", "--workspace", "my-stack"]
# }
```

## 6. Test agent integration

Launch Claude Code in `~/projects/my-app`. Inside an agent session:

```
> What topics is this workspace's skill library focused on?
```

The agent reads the rules block at session start; the body (via `@.tome/RULES.md`) tells it which topics are covered + that `search_skills` should be called when those topics come up. The agent's response should reflect the workspace's plugin set.

```
> Search for skills relevant to <a topic from your enabled plugins>
```

The agent calls the `search_skills` MCP tool; results are returned from the central DB filtered to `my-stack`'s enabled plugins.

## 7. Idempotent sync

```bash
tome harness sync
```

Reports "no changes" — every file already matches the effective harness list. The underlying `rename()` syscall count for this invocation is zero (verifiable via `strace -e rename,renameat tome harness sync` on Linux).

Re-running `tome harness sync` repeatedly produces zero file writes per invocation.

## 8. Doctor health check

```bash
tome doctor
```

Reports every subsystem as healthy:

```
Workspace:        my-stack (project-marker resolution from .tome/config.toml)
Binding:          OK
Project rules:    OK (matches workspace)
Embedder:         OK (bge-small-en-v1.5)
Reranker:         OK (bge-reranker-base)
Summariser:       OK (qwen2.5-0.5b-instruct)
Index:            OK (schema v2, 47 skills indexed)
Drift:            OK
Catalog example:  OK
Harness claude-code:
  rules-file:     OK (AGENTS.md)
  mcp-config:     OK (.claude/settings.json)
Detected (not configured):  cursor

Overall: OK
```

Exit 0.

Try injecting a fault:

```bash
rm .claude/settings.json
tome doctor
# Reports: Harness claude-code mcp-config: BROKEN (file missing); SuggestedFix: tome harness sync
# Exit 1

tome doctor --fix
# Re-runs harness sync for claude-code, recreating .claude/settings.json
# Re-reports: Harness claude-code mcp-config: OK
# Exit 0
```

## 9. Two-project, one-workspace propagation

```bash
cd ~/projects/another-app
tome workspace use my-stack
# Same binding flow as step 5; .tome/ created in second project
```

Modify the workspace's enabled plugin set:

```bash
tome plugin disable example/awesome --workspace my-stack
# Drops workspace_skills rows for my-stack
# Triggers summary regeneration
# Triggers integration sync — but only for the currently-resolved project; the other bound project is not auto-touched
```

Manually propagate to the other project:

```bash
cd ~/projects/my-app
tome workspace sync my-stack
# Re-copies <home>/.tome/workspaces/my-stack/RULES.md to <project>/.tome/RULES.md
```

The cross-project rules-file sync is via explicit `tome workspace sync`; this is per FR-406. The MCP integration is per-project (no propagation needed — the MCP entry already names the workspace; future agent sessions read the freshest cached summary at server startup).

## 10. Cleanup

```bash
cd /tmp  # away from any project
tome workspace remove my-stack --force
# Tears down integration in every bound project, deletes the workspace dir, refcount-cleans catalog clones.
```

Verify:

```bash
tome workspace list
# Only `global` remains.

ls <project1>/.tome/   # ENOENT — removed by the cascade
ls <project2>/.tome/   # ENOENT — removed by the cascade

cat <project1>/AGENTS.md   # The Tome block has been removed; non-Tome content preserved.
cat <project1>/.claude/settings.json   # The `tome` entry has been removed; other entries preserved.
```

## Coverage map

The smoke-test sequence above exercises these spec success criteria end-to-end:

- SC-101 (every path under `<home>/.tome/`): verified by `ls` checks.
- SC-102 (single central DB): verified by absence of `<project>/.tome/index.db`.
- SC-103 (`workspace init`): step 2.
- SC-104 (`workspace use`): step 5.
- SC-105 (two-project propagation): step 9.
- SC-106 (summarisation): steps 3 and `[summaries]` cache check.
- SC-107 (layered resolution): step 4 + step 5 honour the workspace-scoped declaration.
- SC-110 (auto-integration): step 5 writes rules block + MCP entry without explicit user action.
- SC-113 (idempotent sync): step 7.
- SC-114 (doctor reports + `--fix`): step 8.
- SC-117 (`global` cannot be removed): try `tome workspace remove global` — refuses.

## Troubleshooting

| Symptom | Likely cause | Fix |
|---------|--------------|-----|
| `workspace use` exits 2 with "refusing to bind home directory" | CWD is `$HOME` | `cd` into an actual project directory. |
| `workspace use` exits 13 with "workspace `X` not found" | The workspace doesn't exist in the central registry. | `tome workspace init X` first. |
| `harness use` exits 18 with "harness `X` is not supported" | Typo or unsupported harness name. | Run `tome harness` (bare) to list supported names. |
| `harness sync` exits 19 with "harness clash" | Existing `tome` MCP entry isn't Tome-shaped. | Inspect the named file; either edit it manually or rerun with `--force`. |
| `plugin enable` exits 20 with "summariser failure: model missing" | Qwen2.5-0.5B not downloaded. | `tome models download` (or `tome doctor --fix`). |
| `tome doctor` reports `Binding: BROKEN (workspace 'X' not found)` | The bound workspace was deleted out-from-under the project. | `tome workspace use <existing>` to rebind, or `tome workspace init X` to recreate. `doctor --fix` will NOT auto-resolve this case. |
