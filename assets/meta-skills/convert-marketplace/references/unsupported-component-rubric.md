# Unsupported-component decision rubric

When `tome convert` flags a Claude Code component it cannot represent natively,
use this table to choose **drop**, **hand-port**, or **document** for it. The
right call depends on whether the behaviour matters to the user and whether Tome
has a native way to express it. When in doubt, prefer **document** over silently
dropping — a recorded gap is recoverable; a silent loss is not.

| Component | What it is in Claude Code | Default decision | Why / how |
|---|---|---|---|
| **monitors/** | Background watchers that react to repo/file events | **Document** | No Tome-native equivalent. If the watch behaviour is important, note it as a gap; do not fabricate a substitute. |
| **themes/** | Editor/UI colour themes | **Drop** | Pure presentation, harness-specific, no behavioural value across harnesses. |
| **output-styles/** | Response-formatting presets | **Drop** (usually) | Harness-only presentation. **Hand-port** only if a style encodes real guidance (e.g. "always answer in X format") — move that into the relevant agent/skill prose. |
| **LSP servers** | Language-server integrations | **Document** | Tome does not manage language servers. Record which servers the plugin expected so the user can wire them in their harness directly. |
| **status-line scripts** | Scripts that render a status line | **Drop** or **Document** | Harness-specific UI. Drop if cosmetic; document if it surfaced load-bearing state. |
| **channels** | Inter-component message channels | **Document** | No Tome model for this. Note the coupling so the user understands what won't carry over. |
| **bin/ helpers** | Bundled executables/scripts a plugin shells out to | **Hand-port** or **Document** | If a skill/command depends on the helper, keep the helper in the plugin's files and reference it via the Tome data-dir path; if it's harness-glue only, document the gap. |
| **userConfig / custom settings** | Plugin-specific configuration schema | **Document** | Tome has no generic user-config surface. Record the settings the plugin expected and any defaults so behaviour is reproducible. |
| **hooks** | Claude Code event hooks (`hooks/hooks.json` + scripts) | **Passed through — verify** | Converted plugins keep their `hooks/` directory verbatim. Verify two things: (1) the top level of the hooks file is the event map (`{"PreToolUse": [...]}`), not the wrapped `{"description", "hooks": {...}}` form — if it is wrapped, unwrap it, or `tome sync` fails with exit 43; (2) every `${CLAUDE_PLUGIN_ROOT}/…` path the hooks reference exists in the converted plugin (see the `scripts/` row). At sync time Tome translates a subset of hooks natively for Codex, Cursor, Devin, Gemini and Copilot CLI, reconciles real hooks into Claude Code's own settings, and renders the `GUARDRAILS.md` prose fallback everywhere else. If a hook encodes a *rule* that matters on a harness with no native hook support, also hand-port it as guardrail prose. |
| **harness-specific variables** | `CLAUDE_*` placeholders, legacy positional args | **Auto (no action)** | `tome catalog convert` rewrites these to their Tome equivalents automatically; `tome catalog lint` flags any residual and `--autofix` fixes the mechanical ones. You only act if lint still reports one. |
| **commands / skills** | The core entries | **Auto (no action)** | These convert natively. Only revisit one if `tome catalog lint` reports a finding against it. |
| **agents** | Subagent definitions (`agents/*.md`) | **Hand-port** | The body converts, but convert strips the frontmatter (`model`, `tools`, `allowed-tools`, `disallowed-tools`, and so on) and warns `convert/agent-lossy` / `convert/tool-restriction-dropped`. `tome sync` translates those fields into each harness's native agent format, so re-add the ones that matter to the emitted `agents/<name>.md`; `tome catalog lint` tolerates them. |
| **plugin-root `scripts/` / `lib/`** | Support directories a hook or command shells out to | **Hand-port** | Convert neither reads nor warns about these. If any hook or command references `${CLAUDE_PLUGIN_ROOT}/scripts/…`, copy the directory into the converted plugin by hand, or the reference breaks silently. |
| **nested `commands/` / `agents/` subdirs** | Namespaced entries (`commands/git/commit.md`) | **Hand-port** | Dropped with no diagnostic. Re-create each as a top-level entry in the converted plugin (for example `commands/git-commit.md`). |

## Recording decisions

For every flagged component, keep a one-line record you can report in Step 5:

```
<component>  →  <drop | hand-port | document>  —  <reason / what you did>
```

Example:
```
monitors/auto-format         → document   — no Tome watcher; gap noted in plugin README
output-styles/terse          → hand-port  — folded the "be terse" guidance into the agent body
themes/midnight              → drop       — cosmetic, harness-only
bin/lint.sh                  → hand-port  — kept in plugin files, referenced via the data-dir path
agents/reviewer              → hand-port  — re-added model+disallowed-tools frontmatter to agents/reviewer.md
scripts/                     → hand-port  — copied into the plugin; a PreToolUse hook shells out to it
```

The user sees this full list before they decide whether to register anything —
that is what makes the report-and-confirm gate meaningful.
