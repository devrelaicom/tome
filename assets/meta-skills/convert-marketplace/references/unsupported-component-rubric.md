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
| **hooks Tome can't model** | Event hooks beyond what Tome reconciles | **Hand-port** or **Document** | Tome reconciles real Claude Code hooks and renders a prose `GUARDRAILS.md` fallback for others. If the hook encodes a *rule* ("never run X", "always confirm Y"), hand-port it as guardrail prose; otherwise document it. |
| **harness-specific variables** | `CLAUDE_*` placeholders, legacy positional args | **Auto (no action)** | `tome catalog convert` rewrites these to their Tome equivalents automatically; `tome catalog lint` flags any residual and `--autofix` fixes the mechanical ones. You only act if lint still reports one. |
| **commands/skills/agents** | The core entries | **Auto (no action)** | These convert natively. Only revisit one if `tome catalog lint` reports a finding against it. |

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
```

The user sees this full list before they decide whether to register anything —
that is what makes the report-and-confirm gate meaningful.
