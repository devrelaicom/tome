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
use crate::workspace::ResolvedScope;

use super::home_root;

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
    /// Which of the three settings scopes (project/workspace/global)
    /// directly declare this harness in their `harnesses` array.
    /// Composition references that *would* pull this harness in are
    /// NOT reported here — we only surface direct declarations to keep
    /// the report deterministic on `[workspaces.<name>]` references
    /// against absent workspaces.
    pub direct_scopes: Vec<String>,
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

    let direct_scopes = collect_direct_scopes(scope, paths, &snap.name)?;

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
        direct_scopes,
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

fn collect_direct_scopes(
    scope: &ResolvedScope,
    paths: &Paths,
    name: &str,
) -> Result<Vec<String>, TomeError> {
    let mut found = Vec::new();
    // Project marker
    if let Some(marker) = super::list::load_project_marker_for_use(scope)?
        && let Some(list) = marker.harnesses.as_ref()
        && list.iter().any(|n| n == name)
    {
        found.push("project".to_string());
    }
    // Workspace settings
    if let Some(ws) = super::list::load_workspace_settings_for_use(scope, paths)?
        && let Some(list) = ws.harnesses.as_ref()
        && list.iter().any(|n| n == name)
    {
        found.push("workspace".to_string());
    }
    // Global settings
    let global = super::list::load_global_settings_for_use(paths)?;
    if let Some(list) = global.harnesses.as_ref()
        && list.iter().any(|n| n == name)
    {
        found.push("global".to_string());
    }
    Ok(found)
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
    if outcome.direct_scopes.is_empty() {
        writeln!(out, "  Direct declares: (none)")?;
    } else {
        writeln!(
            out,
            "  Direct declares: {}",
            outcome.direct_scopes.join(", "),
        )?;
    }
    Ok(())
}
