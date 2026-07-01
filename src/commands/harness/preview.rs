//! `tome harness preview <harness> [--plugin <id>] [--json]` — the read-only
//! per-harness fidelity preview (issue #288).
//!
//! Reports, per enabled entry in the resolved workspace (or a single
//! `--plugin`), what the target harness would receive vs drop when
//! `harness sync` runs. Every verdict is computed by
//! [`crate::harness::preview::pipeline`], which reuses the ACTUAL translation
//! logic the sync reconcilers use — so the preview matches what sync produces.
//!
//! Read-only: no writes, no harness files touched, the DB opened read-only, no
//! advisory lock.

use std::io::Write;

use crate::cli::HarnessPreviewArgs;
use crate::error::TomeError;
use crate::harness::preview::{
    AgentDelivery, AgentPreview, EntryDelivery, HookPreview, PreviewReport,
};
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::workspace::ResolvedScope;

use super::home_root;

pub fn run(
    args: HarnessPreviewArgs,
    scope: &ResolvedScope,
    paths: &Paths,
    mode: Mode,
) -> Result<(), TomeError> {
    let home = home_root()?;
    let report = crate::harness::preview::pipeline(
        &args.harness,
        args.plugin.as_deref(),
        scope,
        paths,
        &home,
    )?;
    match mode {
        Mode::Human => emit_human(&report),
        Mode::Json => write_json(&report),
    }
}

/// `true` when the report has no enabled entries of any kind to preview.
fn is_empty(report: &PreviewReport) -> bool {
    report.agents.is_empty() && report.entries.is_empty() && report.hooks.is_empty()
}

fn emit_human(report: &PreviewReport) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    render_human(&mut out, report)
}

/// Render the human report into `out`. Split from [`emit_human`] so tests can
/// drive the real renderer into a buffer (a writer seam) rather than asserting a
/// re-derived `format!` copy.
fn render_human(out: &mut impl Write, report: &PreviewReport) -> Result<(), TomeError> {
    writeln!(out, "Preview: {} ({})", report.harness, report.description)?;
    writeln!(out, "  Workspace:       {}", report.workspace)?;
    if let Some(p) = &report.plugin_filter {
        writeln!(out, "  Plugin filter:   {p}")?;
    }
    match &report.rules_target {
        Some(p) => writeln!(out, "  Rules directive: {}", p.display())?,
        None => writeln!(out, "  Rules directive: — (no project resolved)")?,
    }
    match (&report.mcp_target, report.mcp_manual_only) {
        (Some(p), _) => writeln!(out, "  MCP register:    {}", p.display())?,
        (None, true) => writeln!(
            out,
            "  MCP register:    manual (paste snippet — see `tome harness info {}`)",
            report.harness
        )?,
        (None, false) => writeln!(out, "  MCP register:    — (no project resolved)")?,
    }

    if is_empty(report) {
        writeln!(out)?;
        writeln!(
            out,
            "Nothing enabled in workspace `{}`{} to preview.",
            report.workspace,
            report
                .plugin_filter
                .as_deref()
                .map(|p| format!(" for plugin `{p}`"))
                .unwrap_or_default(),
        )?;
        return Ok(());
    }

    // ----- Agents -----
    writeln!(out)?;
    if report.agents.is_empty() {
        writeln!(out, "Agents: (none enabled)")?;
    } else if report.supports_native_agents {
        writeln!(out, "Agents (native translation):")?;
        for a in &report.agents {
            emit_agent_line(out, a)?;
        }
    } else {
        // Rules-only harness: agents become personas or are unrepresented.
        let how = if report.personas_enabled {
            "MCP personas (expose_agents_as_personas on)"
        } else {
            "unrepresented (no native agent form; enable expose_agents_as_personas for MCP personas)"
        };
        writeln!(out, "Agents ({how}):")?;
        for a in &report.agents {
            emit_agent_line(out, a)?;
        }
    }
    // The preview reports model/tools drops + delivery routing; it does not model
    // the Claude-Code-only `strip_plugin_agent_privileges` clone (which clears the
    // privileged hooks/mcpServers/permissionMode passthrough at sync time). Note
    // that scope only for claude-code, where the setting has an effect.
    if report.harness == "claude-code" && !report.agents.is_empty() {
        writeln!(
            out,
            "  Note: model/tools drops shown; privileged passthrough \
             (hooks/mcpServers/permissionMode) is not modelled — if \
             `strip_plugin_agent_privileges` is set, sync clears those fields."
        )?;
    }

    // ----- Skills / commands -----
    writeln!(out)?;
    if report.entries.is_empty() {
        writeln!(out, "Skills / commands: (none enabled)")?;
    } else {
        writeln!(out, "Skills / commands (via the Tome MCP server):")?;
        for e in &report.entries {
            let how = match e.delivery {
                EntryDelivery::McpPrompt => "MCP prompt",
                EntryDelivery::McpGetSkill => "get_skill",
            };
            writeln!(
                out,
                "  [{}] {}/{}/{}  → {how}",
                e.kind, e.catalog, e.plugin, e.name
            )?;
        }
    }

    // ----- Hooks -----
    writeln!(out)?;
    if report.hooks.is_empty() {
        writeln!(
            out,
            "Hooks: (no enabled plugin ships hooks or GUARDRAILS.md)"
        )?;
    } else {
        let native = if report.supports_native_hooks {
            "native plugin-hook translation"
        } else {
            "no native hook translation (GUARDRAILS.md prose fallback)"
        };
        writeln!(out, "Hooks ({native}):")?;
        for h in &report.hooks {
            emit_hook_line(out, h)?;
        }
    }
    // Surface a swallowed hook-enumeration error (e.g. a malformed
    // `hooks/hooks.json`), consistent with the agent path's per-entry errors —
    // honest, not silently omitted.
    if let Some(err) = &report.hooks_error {
        writeln!(out, "  ! hook read error (some hooks omitted): {err}")?;
    }

    Ok(())
}

fn emit_agent_line(out: &mut impl Write, a: &AgentPreview) -> Result<(), TomeError> {
    let id = format!("{}/{}/{}", a.catalog, a.plugin, a.name);
    match &a.delivery {
        AgentDelivery::Native {
            filename,
            displayed_name,
            dropped_fields,
        } => {
            let drops = if dropped_fields.is_empty() {
                "no fields dropped".to_string()
            } else {
                format!("drops: {}", dropped_fields.join(", "))
            };
            writeln!(
                out,
                "  {id}  → native {filename} (as `{displayed_name}`; {drops})"
            )?;
        }
        AgentDelivery::Persona => {
            writeln!(out, "  {id}  → MCP persona")?;
        }
        AgentDelivery::Unrepresented => {
            writeln!(out, "  {id}  → unrepresented (dropped)")?;
        }
    }
    if let Some(err) = &a.error {
        writeln!(out, "      ! parse error: {err}")?;
    }
    Ok(())
}

fn emit_hook_line(out: &mut impl Write, h: &HookPreview) -> Result<(), TomeError> {
    writeln!(out, "  {}/{}", h.catalog, h.plugin)?;
    if !h.native_events.is_empty() {
        writeln!(out, "      native: {}", h.native_events.join(", "))?;
    }
    if !h.guardrails_events.is_empty() {
        writeln!(
            out,
            "      → GUARDRAILS: {}",
            h.guardrails_events.join(", ")
        )?;
    }
    if h.has_guardrails_prose {
        writeln!(out, "      GUARDRAILS.md prose present")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::preview::EntryPreview;

    fn base_report() -> PreviewReport {
        PreviewReport {
            harness: "opencode".into(),
            description: "OpenCode".into(),
            workspace: "global".into(),
            plugin_filter: None,
            supports_native_agents: true,
            personas_enabled: false,
            rules_target: Some(std::path::PathBuf::from("/p/AGENTS.md")),
            mcp_target: Some(std::path::PathBuf::from("/p/opencode.json")),
            mcp_manual_only: false,
            supports_native_hooks: false,
            agents: vec![],
            entries: vec![],
            hooks: vec![],
            hooks_error: None,
        }
    }

    #[test]
    fn is_empty_true_when_no_entries() {
        assert!(is_empty(&base_report()));
    }

    #[test]
    fn is_empty_false_with_an_entry() {
        let mut r = base_report();
        r.entries.push(EntryPreview {
            catalog: "c".into(),
            plugin: "p".into(),
            name: "n".into(),
            kind: "skill".into(),
            delivery: EntryDelivery::McpGetSkill,
        });
        assert!(!is_empty(&r));
    }

    /// The empty-preview human output — driven through the REAL `render_human`
    /// (writer seam), not a re-derived `format!` copy — names the workspace and
    /// gives a clear "nothing enabled" message with the header context.
    #[test]
    fn empty_report_human_output_is_clear() {
        let mut buf = Vec::new();
        render_human(&mut buf, &base_report()).expect("render");
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("Preview: opencode"), "header present: {s}");
        assert!(
            s.contains("Nothing enabled in workspace `global` to preview."),
            "empty message present: {s}"
        );
    }

    /// The empty message names the plugin filter when scoped, through the real
    /// renderer.
    #[test]
    fn empty_report_names_plugin_filter() {
        let mut r = base_report();
        r.plugin_filter = Some("myplug".into());
        let mut buf = Vec::new();
        render_human(&mut buf, &r).expect("render");
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.contains("Nothing enabled in workspace `global` for plugin `myplug` to preview."),
            "scoped empty message: {s}"
        );
    }

    /// The Claude-Code privileged-passthrough note appears only for claude-code
    /// (with agents present), scoping the "matches sync" claim in the output.
    #[test]
    fn claude_code_privilege_note_present_only_for_claude_code() {
        // claude-code with an agent → note present.
        let mut cc = base_report();
        cc.harness = "claude-code".into();
        cc.agents.push(AgentPreview {
            catalog: "c".into(),
            plugin: "p".into(),
            name: "a".into(),
            delivery: AgentDelivery::Native {
                filename: "p__a.md".into(),
                displayed_name: "a".into(),
                dropped_fields: vec![],
            },
            error: None,
        });
        let mut buf = Vec::new();
        render_human(&mut buf, &cc).expect("render");
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.contains("strip_plugin_agent_privileges"),
            "claude-code must carry the privilege note: {s}"
        );

        // A different native-agent harness → no note.
        let mut oc = base_report(); // opencode
        oc.agents = cc.agents.clone();
        let mut buf2 = Vec::new();
        render_human(&mut buf2, &oc).expect("render");
        let s2 = String::from_utf8(buf2).unwrap();
        assert!(
            !s2.contains("strip_plugin_agent_privileges"),
            "non-claude-code must NOT carry the privilege note: {s2}"
        );
    }

    /// A swallowed hook-enumeration error is surfaced in the human output.
    #[test]
    fn hooks_error_is_surfaced() {
        let mut r = base_report();
        r.hooks_error = Some("bad hooks.json at /x".into());
        // Give it one entry so it isn't the empty branch.
        r.entries.push(EntryPreview {
            catalog: "c".into(),
            plugin: "p".into(),
            name: "n".into(),
            kind: "skill".into(),
            delivery: EntryDelivery::McpGetSkill,
        });
        let mut buf = Vec::new();
        render_human(&mut buf, &r).expect("render");
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.contains("hook read error (some hooks omitted): bad hooks.json at /x"),
            "hooks_error must be surfaced: {s}"
        );
    }

    #[test]
    fn native_agent_line_reports_dropped_fields() {
        let mut buf = Vec::new();
        emit_agent_line(
            &mut buf,
            &AgentPreview {
                catalog: "cat".into(),
                plugin: "p".into(),
                name: "reviewer".into(),
                delivery: AgentDelivery::Native {
                    filename: "p__reviewer.md".into(),
                    displayed_name: "reviewer".into(),
                    dropped_fields: vec!["model".into(), "tools".into()],
                },
                error: None,
            },
        )
        .unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("native p__reviewer.md"), "got: {s}");
        assert!(s.contains("drops: model, tools"), "got: {s}");
    }

    #[test]
    fn unrepresented_agent_line_marks_dropped() {
        let mut buf = Vec::new();
        emit_agent_line(
            &mut buf,
            &AgentPreview {
                catalog: "cat".into(),
                plugin: "p".into(),
                name: "a".into(),
                delivery: AgentDelivery::Unrepresented,
                error: None,
            },
        )
        .unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("unrepresented (dropped)"), "got: {s}");
    }

    #[test]
    fn hook_line_splits_native_and_guardrails_events() {
        let mut buf = Vec::new();
        emit_hook_line(
            &mut buf,
            &HookPreview {
                catalog: "cat".into(),
                plugin: "p".into(),
                native_events: vec!["PreToolUse".into()],
                guardrails_events: vec!["Notification".into()],
                has_guardrails_prose: true,
            },
        )
        .unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("native: PreToolUse"), "got: {s}");
        assert!(s.contains("→ GUARDRAILS: Notification"), "got: {s}");
        assert!(s.contains("GUARDRAILS.md prose present"), "got: {s}");
    }
}
