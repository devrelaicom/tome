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
    BlockBodyStyle, McpConfigFormat, RulesFileStrategy, mcp_config, rules_file,
    with_effective_modules,
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
}

/// Per-harness snapshot captured outside the registry's read guard.
struct ModuleSnapshot {
    name: String,
    description: String,
    rules_strategy: RulesFileStrategy,
    mcp_format: McpConfigFormat,
    mcp_parent_key: &'static str,
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
                mcp_format: m.mcp_config_format(),
                mcp_parent_key: m.mcp_parent_key(),
                detected: m.detect(&home),
                detected_path: home.join(format!(".{}", m.name())),
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
                let entry = mcp_config::read_entry(mcp_path, snap.mcp_format, snap.mcp_parent_key)?;
                let entry_present = entry.is_some();
                let entry_tome_owned = entry.as_ref().map(mcp_config::is_tome_owned);
                (Some(block_present), Some(entry_present), entry_tome_owned)
            }
            _ => (None, None, None),
        };

    let references = collect_references(scope, paths, &snap.name)?;

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
            let body = match std::fs::read_to_string(target) {
                Ok(s) => s,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(e) => return Err(TomeError::Io(e)),
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
    Ok(())
}
