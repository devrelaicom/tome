//! `tome harness info <name>` — per-harness details for the current
//! project.
//!
//! Reports: identity, detection (with the per-user dir probed),
//! rules-file + MCP config targets, currently-integrated state
//! (rules block + MCP entry presence), and which settings scopes
//! reference this harness.

use std::io::Write;
use std::path::PathBuf;

use serde::Serialize;

use crate::cli::HarnessInfoArgs;
use crate::error::TomeError;
use crate::harness::{
    BlockBodyStyle, McpDialect, RulesFileStrategy, mcp_config, rules_file, with_effective_modules,
};
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::settings::resolver::resolve_effective_list;
use crate::workspace::ResolvedScope;

use super::home_root;

/// One reason this harness appears in the effective list. Each entry
/// names a settings scope plus, optionally, the composition reference
/// in another scope that pulled this harness in.
///
/// Examples:
///
/// * `{ scope: "project", via: None }` — declared directly in the
///   project marker.
/// * `{ scope: "project", via: Some("[global]") }` — declared in global
///   settings, pulled into the effective list by the project marker's
///   `[global]` reference.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HarnessReference {
    pub scope: String,
    pub via: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HarnessInfoOutcome {
    pub name: String,
    pub description: String,
    pub detected: bool,
    pub detected_path: PathBuf,
    pub rules_target: Option<PathBuf>,
    pub mcp_target: Option<PathBuf>,
    pub rules_block_present: Option<bool>,
    pub mcp_entry_present: Option<bool>,
    pub mcp_tome_owned: Option<bool>,
    /// Why this harness appears in the effective list.
    ///
    /// Populated from the resolver's `source_chain` for the named
    /// harness when a project is resolved: the first chain element is
    /// the entry-point scope (e.g. `"project"`), and any subsequent
    /// element is the composition reference that pulled this harness
    /// into the effective list (e.g. `"[global]"`). When `project_root`
    /// is `None`, the resolver cannot run with project + workspace
    /// context, so this falls back to direct-declaration scanning of
    /// workspace + global settings (C-B3 from US3 review).
    pub references: Vec<HarnessReference>,
    /// Phase 11 / US5 (T063): the paste-able MCP-server snippet — the EXACT
    /// bytes Tome would write for this harness's [`McpDialect`], built with
    /// the canonical `["mcp", "--workspace", "<ws>", "--harness", "<name>"]`
    /// args. For a manual-only harness (jetbrains-ai) this is the primary
    /// recovery artifact; for every harness it is the self-heal paste target.
    ///
    /// Appended LAST + `skip_serializing_if`-gated so the byte-stable `--json`
    /// pins for the pre-Phase-11 fields don't move. Always populated in
    /// practice (every dialect renders), but `Option` keeps the wire shape
    /// additive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_snippet: Option<String>,
}

/// Per-harness snapshot captured outside the registry's read guard.
struct ModuleSnapshot {
    name: String,
    description: String,
    rules_strategy: RulesFileStrategy,
    mcp_dialect: McpDialect,
    detected: bool,
    detected_path: PathBuf,
    rules_target: Option<PathBuf>,
    mcp_target: Option<PathBuf>,
    #[allow(dead_code)]
    block_body_style: BlockBodyStyle,
}

pub fn run(
    args: HarnessInfoArgs,
    scope: &ResolvedScope,
    paths: &Paths,
    mode: Mode,
) -> Result<(), TomeError> {
    let home = home_root()?;
    let project_root = scope.project_root.clone();
    let snap = with_effective_modules(|mods| {
        mods.iter()
            .find(|m| m.name() == args.name)
            .map(|m| ModuleSnapshot {
                name: m.name().to_string(),
                description: m.description().to_string(),
                rules_strategy: m.rules_file_strategy(),
                mcp_dialect: m.mcp_dialect(),
                detected: m.detect(&home),
                detected_path: m.detect_path(&home),
                rules_target: project_root.as_deref().map(|p| m.rules_file_target(p)),
                mcp_target: project_root.as_deref().map(|p| m.mcp_config_path(p, &home)),
                block_body_style: m.block_body_style(),
            })
    });
    let snap = snap.ok_or_else(|| TomeError::HarnessNotSupported {
        name: args.name.clone(),
    })?;

    let (rules_block_present, mcp_entry_present, mcp_tome_owned) =
        match (&snap.rules_target, &snap.mcp_target) {
            (Some(rules_path), Some(mcp_path)) => {
                let block_present = probe_rules_block(rules_path, snap.rules_strategy)?;
                let entry = mcp_config::read_entry(mcp_path, &snap.mcp_dialect)?;
                let entry_present = entry.is_some();
                let entry_tome_owned = entry.as_ref().map(mcp_config::is_tome_owned);
                (Some(block_present), Some(entry_present), entry_tome_owned)
            }
            _ => (None, None, None),
        };

    let references = collect_references(scope, paths, &snap.name)?;

    // Phase 11 / US5 (T063): render the paste-able MCP snippet from the
    // harness's dialect, built with the canonical args the sync writer uses
    // (`mcp --workspace <ws> --harness <name>`, the `--harness` trailing so
    // the ownership marker survives) — so the snippet bytes match what sync
    // writes. Keyed off the resolved workspace name + the harness name.
    let snippet_entry = mcp_config::TomeEntry::new(
        "tome".to_string(),
        vec![
            "mcp".to_string(),
            "--workspace".to_string(),
            scope.scope.name().as_str().to_string(),
            "--harness".to_string(),
            snap.name.clone(),
        ],
    );
    let mcp_snippet = Some(mcp_config::render_entry_snippet(
        &snap.mcp_dialect,
        &snippet_entry,
    ));

    let outcome = HarnessInfoOutcome {
        name: snap.name,
        description: snap.description,
        detected: snap.detected,
        detected_path: snap.detected_path,
        rules_target: snap.rules_target,
        mcp_target: snap.mcp_target,
        rules_block_present,
        mcp_entry_present,
        mcp_tome_owned,
        references,
        mcp_snippet,
    };

    match mode {
        Mode::Human => emit_human(&outcome),
        Mode::Json => write_json(&outcome),
    }
}

fn probe_rules_block(
    target: &std::path::Path,
    strategy: RulesFileStrategy,
) -> Result<bool, TomeError> {
    match strategy {
        RulesFileStrategy::BlockInExistingFile => {
            let body =
                match crate::util::bounded_read_to_string(target, crate::util::HARNESS_RULES_MAX) {
                    Ok(s) => s,
                    Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                        return Ok(false);
                    }
                    Err(e) => return Err(e),
                };
            Ok(rules_file::parse_block(&body)?.is_some())
        }
        RulesFileStrategy::StandaloneFile => Ok(target.exists()),
    }
}

/// Collect references that contribute the named harness to the
/// effective list (C-B3 from US3 review).
///
/// Compute the effective list via the production resolver and find
/// every entry with `name == name`. For each match, translate its
/// `source_chain` into a [`HarnessReference`]:
///
/// * Chain of length 1 (e.g. `["project"]`) → `{ scope: "project",
///   via: None }` — direct declaration.
/// * Longer chain (e.g. `["project", "[global]"]`) → `{ scope:
///   "project", via: Some("[global]") }` — pulled in via the last
///   composition reference. The intermediate steps narrate the path but
///   the user-actionable signal is "disabling the last reference would
///   remove this harness".
///
/// When the resolver fails (e.g. malformed settings or an unknown
/// `[workspaces.<name>]` reference) we fall back to direct-declaration
/// scanning rather than propagating the error — `tome harness info` is
/// a diagnostic; a partial report beats no report.
fn collect_references(
    scope: &ResolvedScope,
    paths: &Paths,
    name: &str,
) -> Result<Vec<HarnessReference>, TomeError> {
    // Load the three settings layers.
    let marker = super::list::load_project_marker_for_use(scope)?;
    let workspace_settings = super::list::load_workspace_settings_for_use(scope, paths)?;
    let global_settings = super::list::load_global_settings_for_use(paths)?;

    let provider = super::CentralDbScopeProvider::new(paths);
    let resolved = resolve_effective_list(
        marker.as_ref(),
        workspace_settings.as_ref(),
        &global_settings,
        &provider,
    );

    match resolved {
        Ok(list) => {
            let mut references: Vec<HarnessReference> = list
                .harnesses
                .into_iter()
                .filter(|h| h.name == name)
                .map(|h| {
                    let scope = h.source_chain.first().cloned().unwrap_or_default();
                    let via = if h.source_chain.len() > 1 {
                        h.source_chain.last().cloned()
                    } else {
                        None
                    };
                    HarnessReference { scope, via }
                })
                .collect();
            references.dedup();
            Ok(references)
        }
        Err(_) => {
            // Fall back to direct-declaration scanning when the
            // resolver can't run cleanly. This is the pre-C-B3 behaviour
            // generalised into the new wire shape.
            let mut found = Vec::new();
            if let Some(m) = marker.as_ref()
                && let Some(list) = m.harnesses.as_ref()
                && list.iter().any(|n| n == name)
            {
                found.push(HarnessReference {
                    scope: "project".to_string(),
                    via: None,
                });
            }
            if let Some(ws) = workspace_settings.as_ref()
                && let Some(list) = ws.harnesses.as_ref()
                && list.iter().any(|n| n == name)
            {
                found.push(HarnessReference {
                    scope: "workspace".to_string(),
                    via: None,
                });
            }
            if let Some(list) = global_settings.harnesses.as_ref()
                && list.iter().any(|n| n == name)
            {
                found.push(HarnessReference {
                    scope: "global".to_string(),
                    via: None,
                });
            }
            Ok(found)
        }
    }
}

fn emit_human(outcome: &HarnessInfoOutcome) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "Harness: {}", outcome.name)?;
    writeln!(out, "  Description:     {}", outcome.description)?;
    writeln!(
        out,
        "  Detected:        {} (probed {})",
        if outcome.detected { "yes" } else { "no" },
        outcome.detected_path.display(),
    )?;
    match &outcome.rules_target {
        Some(p) => writeln!(out, "  Rules-file:      {}", p.display())?,
        None => writeln!(out, "  Rules-file:      — (no project resolved)")?,
    }
    match &outcome.mcp_target {
        Some(p) => writeln!(out, "  MCP config:      {}", p.display())?,
        None => writeln!(out, "  MCP config:      — (no project resolved)")?,
    }
    match outcome.rules_block_present {
        Some(true) => writeln!(out, "  Rules block:     present")?,
        Some(false) => writeln!(out, "  Rules block:     absent")?,
        None => {}
    }
    match (outcome.mcp_entry_present, outcome.mcp_tome_owned) {
        (Some(true), Some(true)) => writeln!(out, "  MCP entry:       present (Tome-owned)")?,
        (Some(true), Some(false)) => writeln!(out, "  MCP entry:       present (user-owned)")?,
        (Some(false), _) => writeln!(out, "  MCP entry:       absent")?,
        _ => {}
    }
    if outcome.references.is_empty() {
        writeln!(out, "  References:      (none)")?;
    } else {
        for r in &outcome.references {
            match &r.via {
                None => writeln!(out, "  References:      {} (direct)", r.scope)?,
                Some(via) => writeln!(out, "  References:      {} via {}", r.scope, via)?,
            }
        }
    }
    // Phase 11 / US5 (T063): the paste-able MCP-config snippet. For a
    // manual-only harness (jetbrains-ai) this is how a user adds the Tome
    // server by hand; for every harness it is the self-heal paste target the
    // rules preamble points at.
    if let Some(snippet) = &outcome.mcp_snippet {
        writeln!(out)?;
        writeln!(out, "  MCP config — paste into {}:", outcome.name)?;
        writeln!(out)?;
        // Emit verbatim (already carries its own trailing newline).
        write!(out, "{snippet}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome_with_snippet(snippet: Option<String>) -> HarnessInfoOutcome {
        HarnessInfoOutcome {
            name: "jetbrains-ai".to_string(),
            description: "JetBrains AI Assistant".to_string(),
            detected: true,
            detected_path: PathBuf::from("/h/.aiassistant"),
            rules_target: None,
            mcp_target: None,
            rules_block_present: None,
            mcp_entry_present: None,
            mcp_tome_owned: None,
            references: Vec::new(),
            mcp_snippet: snippet,
        }
    }

    /// T063: `mcp_snippet` serialises LAST + is `skip_serializing_if`-gated so
    /// the byte-stable pre-Phase-11 `--json` pins don't move.
    #[test]
    fn mcp_snippet_is_appended_last_and_gated() {
        let with = serde_json::to_string(&outcome_with_snippet(Some("SNIP".to_string()))).unwrap();
        assert!(
            with.ends_with("\"mcp_snippet\":\"SNIP\"}"),
            "mcp_snippet must be the LAST field; got: {with}",
        );

        // Absent → the key is omitted entirely (skip_serializing_if).
        let without = serde_json::to_string(&outcome_with_snippet(None)).unwrap();
        assert!(
            !without.contains("mcp_snippet"),
            "absent snippet must omit the key; got: {without}",
        );
        // The prior-last field (`references`) remains last when snippet absent.
        assert!(without.ends_with("\"references\":[]}"));
    }
}
