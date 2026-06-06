//! Bare `tome plugin` — interactive catalog → plugin → action browse flow.
//!
//! Spec: `contracts/plugin-commands.md` §"`tome plugin` (no subcommand —
//! interactive)"; FR-050 / FR-051 / FR-052.
//!
//! ## Shape
//!
//! ```text
//! catalog selector ── (Quit)──> exit 0
//!        │
//!        v
//! plugin browser ── (Back)──> catalog selector
//!        │
//!        v
//! plugin view (same as `plugin show`)
//!        │
//!        v
//! action prompt: Enable | Disable | Back
//!   - Enable  → calls enable::run; on completion, redraw plugin view
//!   - Disable → confirm, then lifecycle::disable; redraw plugin view
//!   - Back    → plugin browser
//! ```
//!
//! ## Exit semantics
//!
//! - **Quit menu item** at any level → clean exit (`Ok(())` → exit 0).
//! - **Esc / Ctrl-C** at any prompt → clean exit (`Ok(())` → exit 0). `inquire`
//!   surfaces these as `OperationCanceled` / `OperationInterrupted`, which
//!   `presentation::prompt` maps to [`TomeError::Interrupted`]; we trap that
//!   variant here and translate to `Ok(())` because the contract says the
//!   bare interactive form "always [exits] 0 on a clean exit".
//! - **Errors during enable / disable** propagate verbatim — same exit codes
//!   as the non-interactive subcommands, per the contract.
//!
//! ## Test-driving
//!
//! `inquire` has no public test backend. Slice 2 (T101 / T102) exercises this
//! flow via a pty harness against the CLI binary; this module therefore makes
//! no concession to test-injection — `presentation::prompt` calls are direct.
//! Decision recorded in `retro/P4.md` § Slice 1.

use std::fmt;
use std::io::Write;

use crate::cli::PluginEnableArgs;
use crate::error::TomeError;
use crate::index::workspace_catalogs;
use crate::output::{self, Mode};
use crate::paths::Paths;
use crate::plugin::PluginStatus;
use crate::plugin::components::count_components;
use crate::plugin::lifecycle;
use crate::plugin::manifest::read_plugin_manifest;
use crate::plugin::{PluginId, PluginRecord};
use crate::presentation::{colour, prompt};
use crate::workspace::ResolvedScope;

use super::{
    aggregate_for_plugin, human_relative, open_index_for_read, read_catalog_manifest,
    registry_seeds, resolve_plugin_dir,
};

pub fn run(scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    if mode == Mode::Json {
        return Err(TomeError::Usage(
            "--json is not valid for the bare `tome plugin` interactive flow; \
             pass an explicit subcommand (`tome plugin list --json`, \
             `tome plugin show <id> --json`)"
                .to_owned(),
        ));
    }

    // FR-051 — refuse without a TTY. We write the contract's pointer message
    // ahead of returning `NotATerminal` so the user gets the specific
    // guidance even though the harness will append the generic
    // `NotATerminal` description on its own line.
    if !(output::stdin_is_tty() && output::stdout_is_tty()) {
        let mut err = std::io::stderr().lock();
        let _ = writeln!(
            err,
            "This command requires a terminal. \
             Try `tome plugin list` or `tome plugin show <catalog>/<plugin>`."
        );
        return Err(TomeError::NotATerminal);
    }

    let paths = Paths::resolve()?;

    // FF2: catalog enrolment is sourced from the `workspace_catalogs` DB,
    // not `config.toml [catalogs]` (never written in production → the
    // interactive flow always reported "No catalogs registered" on a fresh
    // install even after `tome catalog add`).
    let conn = open_index_for_read(&paths, &scope.scope)?;
    let enrolments = workspace_catalogs::list_for_workspace(&conn, scope.scope.name().as_str())?;
    drop(conn);

    if enrolments.is_empty() {
        let mut out = std::io::stdout().lock();
        writeln!(
            out,
            "No catalogs registered. Add one with `tome catalog add <source>`."
        )?;
        return Ok(());
    }

    catalog_loop(&paths, scope)
}

// ---------------------------------------------------------------------------
// Levels — each loop is its own function so the control flow mirrors the
// menu hierarchy. Back = `return Ok(())` from the inner loop returns control
// to the enclosing one. Quit = bubble `QuitSignal` all the way up.
// ---------------------------------------------------------------------------

/// What a nested loop level signals back to its parent.
///
/// - `Ok(())` — Back: pop one level.
/// - `Err(LoopExit::Quit)` — Quit / Esc / Ctrl-C: unwind all levels and exit
///   cleanly. The top-level [`run`] maps this to `Ok(())` so the process
///   exits 0.
/// - `Err(LoopExit::Err(_))` — fatal error from a nested operation
///   (typically `enable` or `disable`). Propagated verbatim per contract.
enum LoopExit {
    Quit,
    Err(TomeError),
}

impl From<TomeError> for LoopExit {
    fn from(e: TomeError) -> Self {
        Self::Err(e)
    }
}

type LoopFlow = Result<(), LoopExit>;

fn catalog_loop(paths: &Paths, scope: &ResolvedScope) -> Result<(), TomeError> {
    loop {
        let menu = build_catalog_menu(paths, scope)?;
        let pick = match prompt_select("Pick a catalog", menu) {
            Ok(v) => v,
            Err(InteractiveExit::Quit) => return Ok(()),
            Err(InteractiveExit::Err(e)) => return Err(e),
        };
        match pick {
            CatalogChoice::Quit => return Ok(()),
            CatalogChoice::Catalog { name, .. } => match plugin_loop(paths, scope, &name) {
                Ok(()) => continue,
                Err(LoopExit::Quit) => return Ok(()),
                Err(LoopExit::Err(e)) => return Err(e),
            },
        }
    }
}

fn plugin_loop(paths: &Paths, scope: &ResolvedScope, catalog_name: &str) -> LoopFlow {
    loop {
        // Menu construction errors bubble up — same exit-code surface as
        // `tome plugin list`.
        let menu = build_plugin_menu(paths, scope, catalog_name)?;
        let pick = match prompt_select(&format!("Pick a plugin in `{catalog_name}`"), menu) {
            Ok(v) => v,
            Err(InteractiveExit::Quit) => return Err(LoopExit::Quit),
            Err(InteractiveExit::Err(e)) => return Err(LoopExit::Err(e)),
        };
        match pick {
            PluginChoice::Back => return Ok(()),
            PluginChoice::Plugin { id, .. } => view_loop(paths, scope, &id)?,
        }
    }
}

fn view_loop(paths: &Paths, scope: &ResolvedScope, id: &PluginId) -> LoopFlow {
    loop {
        // Render the plugin view and capture the resolved status so the
        // action menu can offer the right verb without re-querying.
        let status = render_plugin_view(paths, scope, id)?;

        let actions = build_action_menu(status);
        let pick = match prompt_select("Action", actions) {
            Ok(v) => v,
            Err(InteractiveExit::Quit) => return Err(LoopExit::Quit),
            Err(InteractiveExit::Err(e)) => return Err(LoopExit::Err(e)),
        };
        match pick {
            ActionChoice::Back => return Ok(()),
            // Errors propagate per contract — same exit codes as the
            // non-interactive forms. The `?` runs From<TomeError>.
            ActionChoice::Enable => {
                run_enable_action(scope, id)?;
            }
            ActionChoice::Disable => {
                run_disable_action(paths, scope, id)?;
            }
        }
        // After a successful enable / disable, fall through and redraw.
    }
}

// ---------------------------------------------------------------------------
// Menu construction
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum CatalogChoice {
    Catalog {
        name: String,
        plugin_count: usize,
        enabled_count: usize,
    },
    Quit,
}

impl fmt::Display for CatalogChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Catalog {
                name,
                plugin_count,
                enabled_count,
            } => write!(
                f,
                "{name}  ({enabled_count} enabled / {plugin_count} plugins)",
            ),
            Self::Quit => f.write_str("Quit"),
        }
    }
}

fn build_catalog_menu(
    paths: &Paths,
    scope: &ResolvedScope,
) -> Result<Vec<CatalogChoice>, TomeError> {
    let conn = open_index_for_read(paths, &scope.scope)?;
    let workspace_name = scope.scope.name().as_str();
    // FF2: enumerate enrolments from the `workspace_catalogs` DB; the
    // catalog clone root is the content-addressed `cache_dir_for(url)`.
    let enrolments = workspace_catalogs::list_for_workspace(&conn, workspace_name)?;
    let mut out: Vec<CatalogChoice> = Vec::with_capacity(enrolments.len() + 1);
    for enrolment in &enrolments {
        let clone_dir = paths.cache_dir_for(&enrolment.url);
        let manifest = read_catalog_manifest(&clone_dir);
        let plugin_count = manifest.as_ref().map(|m| m.plugins.len()).unwrap_or(0);
        let mut enabled_count = 0usize;
        if let Some(manifest) = &manifest {
            for plugin in &manifest.plugins {
                let agg = aggregate_for_plugin(
                    &conn,
                    workspace_name,
                    &enrolment.catalog_name,
                    &plugin.name,
                )?;
                if agg.total > 0 && agg.enabled > 0 {
                    enabled_count += 1;
                }
            }
        }
        out.push(CatalogChoice::Catalog {
            name: enrolment.catalog_name.clone(),
            plugin_count,
            enabled_count,
        });
    }
    out.push(CatalogChoice::Quit);
    Ok(out)
}

#[derive(Clone)]
enum PluginChoice {
    Plugin {
        id: PluginId,
        version: Option<String>,
        status: PluginStatus,
    },
    Back,
}

impl fmt::Display for PluginChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plugin {
                id,
                version,
                status,
            } => {
                let v = version.as_deref().unwrap_or("—");
                let s = match status {
                    PluginStatus::Enabled => format!("{} enabled", colour::success("✓")),
                    PluginStatus::Disabled => format!("{} disabled", colour::error("✗")),
                    PluginStatus::Unindexable => {
                        format!("{} unindexable", colour::warning("⚠"))
                    }
                };
                write!(f, "{}  v{v}  [{s}]", id.plugin)
            }
            Self::Back => f.write_str("Back"),
        }
    }
}

fn build_plugin_menu(
    paths: &Paths,
    scope: &ResolvedScope,
    catalog_name: &str,
) -> Result<Vec<PluginChoice>, TomeError> {
    let conn = open_index_for_read(paths, &scope.scope)?;
    let workspace_name = scope.scope.name().as_str();
    // FF2: resolve the catalog clone root from the DB enrolment URL.
    let enrolment = workspace_catalogs::find(&conn, workspace_name, catalog_name)?
        .ok_or_else(|| TomeError::CatalogNotFound(catalog_name.to_owned()))?;
    let clone_dir = paths.cache_dir_for(&enrolment.url);
    let manifest = read_catalog_manifest(&clone_dir);

    let mut out: Vec<PluginChoice> = Vec::new();
    if let Some(manifest) = manifest {
        for plugin in manifest.plugins {
            let id = PluginId {
                catalog: enrolment.catalog_name.clone(),
                plugin: plugin.name.clone(),
            };
            let plugin_dir = clone_dir.join(&plugin.source);
            let parsed = read_plugin_manifest(&plugin_dir).ok();
            let agg = aggregate_for_plugin(&conn, workspace_name, &id.catalog, &id.plugin)?;
            let status = match &parsed {
                None => PluginStatus::Unindexable,
                Some(_) => {
                    if agg.total > 0 && agg.enabled > 0 {
                        PluginStatus::Enabled
                    } else {
                        PluginStatus::Disabled
                    }
                }
            };
            let version = parsed.as_ref().map(|m| m.version.clone());
            out.push(PluginChoice::Plugin {
                id,
                version,
                status,
            });
        }
    }
    out.push(PluginChoice::Back);
    Ok(out)
}

#[derive(Clone, Copy)]
enum ActionChoice {
    Enable,
    Disable,
    Back,
}

impl fmt::Display for ActionChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Enable => "Enable",
            Self::Disable => "Disable",
            Self::Back => "Back",
        })
    }
}

fn build_action_menu(status: PluginStatus) -> Vec<ActionChoice> {
    let mut out = Vec::with_capacity(2);
    match status {
        PluginStatus::Enabled => out.push(ActionChoice::Disable),
        PluginStatus::Disabled => out.push(ActionChoice::Enable),
        // Unindexable plugins can't usefully be enabled or disabled — Back
        // is the only sensible action. The view will show the warning and
        // the user can pick a different plugin.
        PluginStatus::Unindexable => {}
    }
    out.push(ActionChoice::Back);
    out
}

// ---------------------------------------------------------------------------
// View rendering
// ---------------------------------------------------------------------------

/// Render the plugin view (same content as `tome plugin show`) and return the
/// resolved status so the action menu can pick the right verb without
/// re-querying the index.
fn render_plugin_view(
    paths: &Paths,
    scope: &ResolvedScope,
    id: &PluginId,
) -> Result<PluginStatus, TomeError> {
    // F11b: the catalog enrolment is read from the DB, so the read handle is
    // opened before resolving the plugin directory and reused for the
    // aggregate below. The surrounding menu builders read the same
    // `workspace_catalogs` enrolment since FF2.
    let conn = open_index_for_read(paths, &scope.scope)?;
    let plugin_dir = resolve_plugin_dir(id, &conn, scope.scope.name().as_str(), paths)?;
    let manifest = read_plugin_manifest(&plugin_dir);
    let component_counts = count_components(&plugin_dir);
    let agg = aggregate_for_plugin(&conn, scope.scope.name().as_str(), &id.catalog, &id.plugin)?;

    let status = match &manifest {
        Err(_) => PluginStatus::Unindexable,
        Ok(_) => {
            if agg.total > 0 && agg.enabled > 0 {
                PluginStatus::Enabled
            } else {
                PluginStatus::Disabled
            }
        }
    };

    let manifest_ok = manifest.as_ref().ok();
    let record = PluginRecord {
        id: id.clone(),
        version: manifest_ok.map(|m| m.version.clone()).unwrap_or_default(),
        author: manifest_ok.and_then(|m| m.author.as_ref().and_then(|a| a.display())),
        description: manifest_ok.and_then(|m| m.description.clone()),
        last_upstream_change: None,
        status,
        component_counts,
        last_indexed_at: agg.last_indexed_at.as_deref().and_then(|s| {
            use time::OffsetDateTime;
            use time::format_description::well_known::Rfc3339;
            OffsetDateTime::parse(s, &Rfc3339).ok()
        }),
    };

    write_plugin_view(&record, agg.last_indexed_at.as_deref())?;
    Ok(status)
}

fn write_plugin_view(record: &PluginRecord, last_indexed: Option<&str>) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    writeln!(out)?;
    writeln!(out, "Plugin:       {}", record.id)?;
    writeln!(
        out,
        "Version:      {}",
        if record.version.is_empty() {
            "—".to_owned()
        } else {
            record.version.clone()
        }
    )?;
    let status_line = match record.status {
        PluginStatus::Enabled => {
            let when = last_indexed
                .map(human_relative)
                .unwrap_or_else(|| "—".to_owned());
            format!("{} enabled (last indexed {})", colour::success("✓"), when)
        }
        PluginStatus::Disabled => format!("{} disabled", colour::error("✗")),
        PluginStatus::Unindexable => format!("{} unindexable", colour::warning("⚠")),
    };
    writeln!(out, "Status:       {status_line}")?;
    writeln!(
        out,
        "Author:       {}",
        record.author.clone().unwrap_or_else(|| "—".to_owned()),
    )?;
    if let Some(desc) = &record.description {
        writeln!(out, "Description:  {desc}")?;
    }
    writeln!(out)?;
    let counts = &record.component_counts;
    writeln!(
        out,
        "Components:   skills={} agents={} commands={} hooks={} mcp_servers={}",
        counts.skills, counts.agents, counts.commands, counts.hooks, counts.mcp_servers,
    )?;
    writeln!(out)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Action handlers
// ---------------------------------------------------------------------------

fn run_enable_action(scope: &ResolvedScope, id: &PluginId) -> Result<(), TomeError> {
    // Delegate to the existing CLI handler for parity with the
    // non-interactive form: this gives us the model-download prompt, the
    // banner, the spinner, the warnings, and the final summary line for
    // free. `--yes` is left false; the user is at a TTY and may decline.
    super::enable::run(
        PluginEnableArgs {
            id: id.to_string(),
            yes: false,
        },
        scope,
        Mode::Human,
    )
}

/// Prompt for confirmation, then call `lifecycle::disable` (force-equivalent
/// per the contract: "On Disable: prompt to confirm, then run `plugin
/// disable --force` equivalent"). Errors from disable propagate per
/// contract — same exit codes as the (future) non-interactive form.
fn run_disable_action(paths: &Paths, scope: &ResolvedScope, id: &PluginId) -> LoopFlow {
    let confirmed = match prompt::confirm(&format!("Disable {id}?"), false) {
        Ok(v) => v,
        Err(TomeError::Interrupted) | Err(TomeError::NotATerminal) => {
            return Err(LoopExit::Quit);
        }
        Err(e) => return Err(LoopExit::Err(e)),
    };
    if !confirmed {
        // Declined — bounce back to the view-loop redraw without erroring.
        return Ok(());
    }

    let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();
    let outcome = lifecycle::disable(
        id,
        paths,
        &scope.scope,
        embedder_seed,
        reranker_seed,
        summariser_seed,
    )?;
    let mut out = std::io::stdout().lock();
    let _ = writeln!(
        out,
        "{} disabled {} ({} skill records retained)",
        colour::success("✓"),
        id,
        outcome.skills_retained,
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Prompt helpers
// ---------------------------------------------------------------------------

/// Outcomes from a prompt that the loop functions need to discriminate.
enum InteractiveExit {
    /// Esc / Ctrl-C / EOF — clean exit of the whole interactive flow.
    Quit,
    /// Anything else — propagate as a normal error.
    Err(TomeError),
}

fn prompt_select<T: fmt::Display>(message: &str, options: Vec<T>) -> Result<T, InteractiveExit> {
    match prompt::select(message, options) {
        Ok(v) => Ok(v),
        Err(TomeError::Interrupted) | Err(TomeError::NotATerminal) => Err(InteractiveExit::Quit),
        Err(e) => Err(InteractiveExit::Err(e)),
    }
}
