//! `tome harness list [<workspace>]` — report a harness list.
//!
//! Two modes:
//!
//! - **No argument**: report the *effective* harness list for the
//!   current project, computed via the layered settings walk +
//!   composition expansion. Each entry annotated with the contributing
//!   scope chain; `!`-prefixed exclusions reported separately.
//! - **`<workspace>` argument**: report that workspace's directly-
//!   declared harness list verbatim (no composition expansion).

use std::io::Write;

use serde::Serialize;

use crate::cli::HarnessListArgs;
use crate::error::TomeError;
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::presentation::tables;
use crate::settings::parser::{parse_global, parse_project_marker, parse_workspace};
use crate::settings::resolver::{EffectiveHarness, resolve_effective_list};
use crate::settings::{GlobalSettings, ProjectMarkerConfig, WorkspaceSettings};
use crate::workspace::{ResolvedScope, WorkspaceName};

use super::CentralDbScopeProvider;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum HarnessListOutcome {
    Effective {
        harnesses: Vec<EffectiveEntry>,
        excluded: Vec<String>,
    },
    AsWritten {
        workspace: String,
        harnesses: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct EffectiveEntry {
    pub name: String,
    /// Mixed-notation chain per `contracts/settings-composition.md` —
    /// each element is either a plain scope name (`"project"` /
    /// `"workspace"` / `"global"`) or a bracketed reference
    /// (`"[workspaces.<name>]"`, `"[workspace]"`, `"[global]"`).
    pub source_chain: Vec<String>,
}

pub fn run(
    args: HarnessListArgs,
    scope: &ResolvedScope,
    paths: &Paths,
    mode: Mode,
) -> Result<(), TomeError> {
    let outcome = match args.workspace.as_deref() {
        Some(raw) => list_as_written(raw, paths)?,
        None => list_effective(scope, paths)?,
    };
    match mode {
        Mode::Human => emit_human(&outcome),
        Mode::Json => write_json(&outcome),
    }
}

fn list_as_written(raw: &str, paths: &Paths) -> Result<HarnessListOutcome, TomeError> {
    let name = WorkspaceName::parse(raw)?;

    // C-M2: confirm the workspace exists in the central registry before
    // reading its settings file. Otherwise a typo like
    // `tome harness list demoo` would silently return an empty list
    // instead of exit 13. The `global` workspace is always present
    // (bootstrap-seeded); fall back to that invariant when the DB has
    // not yet been created.
    if !workspace_is_registered(&name, paths) {
        return Err(TomeError::WorkspaceNotFound {
            name: name.as_str().to_owned(),
        });
    }

    let path = paths.workspace_settings_file(&name);
    let body = match std::fs::read_to_string(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // No settings file → no declared list. Report empty.
            return Ok(HarnessListOutcome::AsWritten {
                workspace: name.as_str().to_owned(),
                harnesses: Vec::new(),
            });
        }
        Err(e) => return Err(TomeError::Io(e)),
    };
    let ws = parse_workspace(&body).map_err(|e| TomeError::WorkspaceMalformed {
        path: path.clone(),
        reason: format!("parse workspace settings: {e}"),
    })?;
    Ok(HarnessListOutcome::AsWritten {
        workspace: name.as_str().to_owned(),
        harnesses: ws.harnesses.unwrap_or_default(),
    })
}

/// Check whether `name` exists in the central `workspaces` table.
/// Mirrors [`CentralDbScopeProvider::workspace_is_registered`] but
/// inlined here to avoid making that helper public outside the
/// `commands::harness` module.
fn workspace_is_registered(name: &WorkspaceName, paths: &Paths) -> bool {
    if !paths.index_db.exists() {
        return name.as_str() == WorkspaceName::global().as_str();
    }
    let Ok(conn) = crate::index::open_read_only(&paths.index_db) else {
        return name.as_str() == WorkspaceName::global().as_str();
    };
    conn.query_row(
        "SELECT 1 FROM workspaces WHERE name = ?1",
        rusqlite::params![name.as_str()],
        |_| Ok(()),
    )
    .is_ok()
}

fn list_effective(scope: &ResolvedScope, paths: &Paths) -> Result<HarnessListOutcome, TomeError> {
    let marker = load_project_marker(scope)?;
    let workspace_settings = load_workspace_settings(scope, paths)?;
    let global_settings = load_global_settings(paths)?;
    let provider = CentralDbScopeProvider::new(paths);

    let resolved = resolve_effective_list(
        marker.as_ref(),
        workspace_settings.as_ref(),
        &global_settings,
        &provider,
    )
    .map_err(TomeError::from)?;

    let harnesses: Vec<EffectiveEntry> = resolved
        .harnesses
        .into_iter()
        .map(|h: EffectiveHarness| EffectiveEntry {
            name: h.name,
            source_chain: h.source_chain,
        })
        .collect();
    Ok(HarnessListOutcome::Effective {
        harnesses,
        excluded: resolved.excluded,
    })
}

pub(crate) fn load_project_marker_for_use(
    scope: &ResolvedScope,
) -> Result<Option<ProjectMarkerConfig>, TomeError> {
    load_project_marker(scope)
}

pub(crate) fn load_workspace_settings_for_use(
    scope: &ResolvedScope,
    paths: &Paths,
) -> Result<Option<WorkspaceSettings>, TomeError> {
    load_workspace_settings(scope, paths)
}

pub(crate) fn load_global_settings_for_use(paths: &Paths) -> Result<GlobalSettings, TomeError> {
    load_global_settings(paths)
}

fn load_project_marker(scope: &ResolvedScope) -> Result<Option<ProjectMarkerConfig>, TomeError> {
    let Some(project_root) = scope.project_root.as_deref() else {
        return Ok(None);
    };
    let path = Paths::project_marker_config(project_root);
    let body = match std::fs::read_to_string(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(TomeError::Io(e)),
    };
    let pm = parse_project_marker(&body).map_err(|e| TomeError::WorkspaceMalformed {
        path: path.clone(),
        reason: format!("parse project marker: {e}"),
    })?;
    Ok(Some(pm))
}

fn load_workspace_settings(
    scope: &ResolvedScope,
    paths: &Paths,
) -> Result<Option<WorkspaceSettings>, TomeError> {
    let path = paths.workspace_settings_file(scope.scope.name());
    let body = match std::fs::read_to_string(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(TomeError::Io(e)),
    };
    let ws = parse_workspace(&body).map_err(|e| TomeError::WorkspaceMalformed {
        path: path.clone(),
        reason: format!("parse workspace settings: {e}"),
    })?;
    Ok(Some(ws))
}

fn load_global_settings(paths: &Paths) -> Result<GlobalSettings, TomeError> {
    let path = &paths.global_settings_file;
    let body = match std::fs::read_to_string(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(GlobalSettings::default()),
        Err(e) => return Err(TomeError::Io(e)),
    };
    parse_global(&body).map_err(|e| TomeError::WorkspaceMalformed {
        path: path.clone(),
        reason: format!("parse global settings: {e}"),
    })
}

fn emit_human(outcome: &HarnessListOutcome) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    match outcome {
        HarnessListOutcome::Effective {
            harnesses,
            excluded,
        } => {
            let mut table = tables::new_table();
            table.set_header(vec!["NAME", "SOURCE_CHAIN"]);
            if harnesses.is_empty() {
                writeln!(out, "No harnesses declared in any settings layer.")?;
            } else {
                for h in harnesses {
                    let chain = h.source_chain.join(" → ");
                    table.add_row(vec![h.name.clone(), chain]);
                }
                writeln!(out, "{table}")?;
            }
            if !excluded.is_empty() {
                writeln!(out, "\nExcluded: {}", excluded.join(", "))?;
            }
        }
        HarnessListOutcome::AsWritten {
            workspace,
            harnesses,
        } => {
            writeln!(out, "Workspace `{workspace}` declares:")?;
            if harnesses.is_empty() {
                writeln!(out, "  (no harnesses declaration)")?;
            } else {
                for h in harnesses {
                    writeln!(out, "  {h}")?;
                }
            }
        }
    }
    Ok(())
}
