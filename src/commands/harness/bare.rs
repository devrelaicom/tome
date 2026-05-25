//! `tome harness` (no subcommand) — enumerate every supported harness
//! in tabular form (FR-520).
//!
//! Columns: NAME / DETECTED / RULES-FILE / MCP-CONFIG. The DETECTED
//! column probes `<home>/.<harness>/` per FR-167 (existence-only). The
//! RULES-FILE and MCP-CONFIG columns surface what Tome would write to
//! in this project — when no project is resolved, both fall back to a
//! `—` placeholder.
//!
//! This subcommand is read-only and never errors short of `$HOME` being
//! unset.

use std::io::Write;
use std::path::PathBuf;

use serde::Serialize;

use crate::error::TomeError;
use crate::harness::with_effective_modules;
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::presentation::tables;
use crate::workspace::ResolvedScope;

use super::home_root;

/// JSON envelope: one row per supported harness, in lex order.
#[derive(Debug, Clone, Serialize)]
pub struct HarnessBareEntry {
    pub name: String,
    pub description: String,
    pub detected: bool,
    /// `None` when no project is resolved.
    pub rules_file: Option<PathBuf>,
    pub mcp_config: PathBuf,
}

pub fn run(scope: &ResolvedScope, _paths: &Paths, mode: Mode) -> Result<(), TomeError> {
    let home = home_root()?;
    let project_root = scope.project_root.as_deref();

    let entries = with_effective_modules(|mods| {
        mods.iter()
            .map(|m| {
                let rules = project_root.map(|p| m.rules_file_target(p));
                // MCP config wants both project and home; for the
                // bare report we use a placeholder project root when
                // no project is bound so the harness-specific helper
                // still composes a usable path (it'll be relative to
                // home for global harnesses, junk for project-local
                // ones — those collapse to `—` via the project_root
                // gate below).
                let mcp = match project_root {
                    Some(p) => m.mcp_config_path(p, &home),
                    None => {
                        // Pass home as both args; harnesses whose MCP
                        // config is global (Codex, Gemini) compose a
                        // valid path. Per-project ones produce a path
                        // we hide in the human form below.
                        m.mcp_config_path(&home, &home)
                    }
                };
                HarnessBareEntry {
                    name: m.name().to_string(),
                    description: m.description().to_string(),
                    detected: m.detect(&home),
                    rules_file: rules,
                    mcp_config: mcp,
                }
            })
            .collect::<Vec<_>>()
    });

    match mode {
        Mode::Human => emit_human(&entries, project_root.is_some()),
        Mode::Json => write_json(&entries),
    }
}

fn emit_human(entries: &[HarnessBareEntry], project_resolved: bool) -> Result<(), TomeError> {
    let mut table = tables::new_table();
    table.set_header(vec!["NAME", "DETECTED", "RULES-FILE", "MCP-CONFIG"]);
    for e in entries {
        let detected = if e.detected { "yes" } else { "no" };
        let rules = match &e.rules_file {
            Some(p) => display_path(p),
            None => "—".to_string(),
        };
        let mcp = if project_resolved {
            display_path(&e.mcp_config)
        } else {
            // Without a project root, only harnesses whose MCP config
            // is genuinely global (Codex, Gemini under $HOME) carry a
            // meaningful path. Detect that by checking whether the
            // composed path lives under $HOME — if not, render `—`.
            match home_root() {
                Ok(home) if e.mcp_config.starts_with(&home) => display_path(&e.mcp_config),
                _ => "—".to_string(),
            }
        };
        table.add_row(vec![e.name.clone(), detected.to_string(), rules, mcp]);
    }
    let mut out = std::io::stdout().lock();
    writeln!(out, "{table}")?;
    Ok(())
}

fn display_path(p: &std::path::Path) -> String {
    p.display().to_string()
}
