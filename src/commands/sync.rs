//! `tome sync` — unified propagation of workspace state to bound projects.
//!
//! Composes the two formerly-separate sync surfaces:
//!
//!  * [`crate::workspace::sync::sync_rules_to_project`] writes the
//!    workspace's `RULES.md` into one project's `<project>/.tome/RULES.md`,
//!    and
//!  * [`crate::harness::sync::sync_project`] reconciles that project's
//!    harness files (rules sink, MCP config, hooks, agents).
//!
//! Defaults to the current project (the resolved scope's `project_root`);
//! `--all` fans out to every project bound to the resolved workspace in
//! `workspace_projects`.
//!
//! ## Why both halves, why this order
//!
//! The RULES.md write lands the workspace prose first; the harness
//! reconcile then renders harness-specific files that may incorporate it.
//! The two halves are independently skippable (`--rules-only` /
//! `--harness-only`, mutually exclusive at the clap layer).
//!
//! ## Forward-progress on `--all`
//!
//! Mirrors the project's `first_error` pattern: a per-project failure is
//! captured but does not abort the fan-out — every reachable project is
//! attempted, and the first captured error is returned at the end so the
//! exit code reflects a genuine failure while partial progress still lands.
//!
//! This command replaces the former `tome workspace sync` /
//! `tome harness sync` subcommands, which were removed pre-launch.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::cli::SyncArgs;
use crate::error::TomeError;
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::workspace::sync::RulesSync;
use crate::workspace::{ResolvedScope, WorkspaceName};

/// One project's sync outcome. `rules` is the classification string from
/// [`RulesSync`] (`"synced"` / `"unchanged"` / `"missing"`) or `None` when
/// the RULES.md write was skipped (`--harness-only`). `harness_changes` is
/// the total count of added + updated + removed harness files, or `0` when
/// the harness reconcile was skipped (`--rules-only`).
#[derive(Debug, Serialize)]
pub struct ProjectOutcome {
    pub project: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rules: Option<&'static str>,
    pub harness_changes: usize,
}

/// Aggregate of every project this `tome sync` invocation touched.
#[derive(Debug, Serialize)]
pub struct SyncReport {
    pub projects: Vec<ProjectOutcome>,
}

/// Dispatcher invoked by `main.rs`. Validates flags, resolves the active
/// workspace, fans out (or targets the current project), then emits.
pub fn run(
    args: SyncArgs,
    scope: &ResolvedScope,
    paths: &Paths,
    mode: Mode,
) -> Result<(), TomeError> {
    // Validate every `--harness <name>` eagerly so an unknown name fails fast
    // with the same error/exit code as `tome harness use` — but only when the
    // harness reconcile will actually run (a `--rules-only` run never touches
    // harnesses, so the values are simply ignored).
    if !args.rules_only {
        for name in &args.harness {
            if crate::harness::lookup(name).is_none() {
                return Err(TomeError::HarnessNotSupported { name: name.clone() });
            }
        }
    }

    // A `--harness` value under `--rules-only` is inert; say so once on stderr
    // rather than silently dropping it.
    if args.rules_only && !args.harness.is_empty() {
        eprintln!("note: --harness is ignored with --rules-only");
    }

    let ws = scope.scope.name().clone();

    let report = if args.all {
        sync_all(&ws, &args, paths)?
    } else {
        let Some(project_root) = scope.project_root.as_deref() else {
            return Err(TomeError::Usage(
                "`tome sync` requires a project marker — run inside a project bound via `tome workspace use`, or pass --all to sync every bound project"
                    .into(),
            ));
        };
        let mut report = SyncReport {
            projects: Vec::new(),
        };
        report
            .projects
            .push(sync_one_project(&ws, project_root, &args, paths)?);
        report
    };

    emit(&report, mode)
}

/// Sync ONE project: the per-project RULES.md write (unless `--harness-only`)
/// followed by the harness reconcile (unless `--rules-only`).
pub fn sync_one_project(
    ws: &WorkspaceName,
    project_root: &Path,
    args: &SyncArgs,
    paths: &Paths,
) -> Result<ProjectOutcome, TomeError> {
    let rules = if args.harness_only {
        None
    } else {
        // A missing central RULES.md is not an error — an empty source still
        // reconciles cleanly (mirrors `workspace::sync::sync_one`, which
        // treats source-absent as a no-op). `unwrap_or_default` collapses the
        // NotFound case to empty bytes.
        let source = std::fs::read(paths.workspace_rules_file(ws)).unwrap_or_default();
        let classification =
            match crate::workspace::sync::sync_rules_to_project(&source, project_root, ws)? {
                RulesSync::Synced => "synced",
                RulesSync::Unchanged => "unchanged",
                RulesSync::MissingProjectDir => "missing",
            };
        Some(classification)
    };

    let harness_changes = if args.rules_only {
        0
    } else {
        let home = crate::commands::harness::home_root()?;
        // `--force` is not exposed on `tome sync` (matching `tome harness
        // sync`); a clash is resolved by re-binding with
        // `tome workspace use --force`.
        let mut deps = crate::harness::sync::build_deps(paths, &home, ws, false);
        deps.only_harness = harness_filter_set(&args.harness);
        let outcome = crate::harness::sync::sync_project(project_root, &deps)?;

        // Telemetry parity with the removed `tome harness sync`: emit one
        // `tome.harness_action{Sync}` per DISTINCT harness that actually had a
        // change. Best-effort, success-path only; a fully-idempotent reconcile
        // (no changes) emits nothing. Unmapped harness names are SKIPped by
        // `emit_harness_action`.
        let mut seen: Vec<&str> = Vec::new();
        for change in outcome
            .added
            .iter()
            .chain(outcome.updated.iter())
            .chain(outcome.removed.iter())
        {
            if !seen.contains(&change.harness.as_str()) {
                seen.push(change.harness.as_str());
                crate::commands::harness::emit_harness_action(
                    &change.harness,
                    crate::telemetry::event::HarnessAction::Sync,
                );
            }
        }

        outcome.added.len() + outcome.updated.len() + outcome.removed.len()
    };

    Ok(ProjectOutcome {
        project: project_root.to_path_buf(),
        rules,
        harness_changes,
    })
}

/// Build the canonical-name `SyncDeps.only_harness` filter SET from the
/// repeated `--harness` values (Phase 11 / US6, T080).
///
/// Each value is alias-resolved (so `--harness antigravity-cli` filters the
/// `gemini` module) before insertion, then deduped by the set. An EMPTY list →
/// `None` → reconcile the full effective set (the default). A single
/// `--harness X` is identical to the former single-name behaviour — a
/// one-element set whose `set.contains(m.name())` matches exactly X.
pub(crate) fn harness_filter_set(harness: &[String]) -> Option<std::collections::HashSet<String>> {
    if harness.is_empty() {
        None
    } else {
        Some(
            harness
                .iter()
                .map(|n| crate::harness::resolve_alias(n).to_string())
                .collect(),
        )
    }
}

/// Fan out [`sync_one_project`] over every project bound to `ws` in
/// `workspace_projects`. Forward-progress: a per-project failure is captured
/// in `first_error` and the loop continues; the first error is returned after
/// every project has been attempted.
pub fn sync_all(
    ws: &WorkspaceName,
    args: &SyncArgs,
    paths: &Paths,
) -> Result<SyncReport, TomeError> {
    let mut report = SyncReport {
        projects: Vec::new(),
    };

    // No central DB → no bindings to walk. An empty report is the correct
    // pre-bootstrap answer (mirrors `workspace::sync::sync_one`).
    if !paths.index_db.is_file() {
        return Ok(report);
    }

    let conn = crate::index::open_read_only(&paths.index_db)?;
    // `resolve_id_required` is the correct loud-on-missing choice here: an
    // explicit `tome sync --all` against a workspace with no registry row is a
    // user error, not the silent-empty pre-bootstrap case handled above.
    let workspace_id = crate::index::workspaces::resolve_id_required(&conn, ws)?;

    // Shared SSOT with `workspace::sync::sync_one` — one `workspace_projects`
    // walk, not a duplicated SELECT.
    let project_roots = crate::workspace::sync::bound_project_roots(&conn, workspace_id)?;

    let mut first_error: Option<TomeError> = None;
    for project_root in project_roots {
        match sync_one_project(ws, &project_root, args, paths) {
            Ok(outcome) => report.projects.push(outcome),
            Err(e) => {
                // Forward-progress: keep going so every reachable project is
                // attempted; surface the first failure for the exit code.
                tracing::warn!(
                    workspace = ws.as_str(),
                    project = %project_root.display(),
                    error = %e,
                    "sync: project failed; continuing",
                );
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
    }

    match first_error {
        Some(e) => Err(e),
        None => Ok(report),
    }
}

/// Emit the report per output mode. Human mode prints one line per project;
/// `--json` emits the wire-stable [`SyncReport`].
fn emit(report: &SyncReport, mode: Mode) -> Result<(), TomeError> {
    match mode {
        Mode::Json => write_json(report),
        Mode::Human => {
            let mut out = std::io::stdout().lock();
            if report.projects.is_empty() {
                writeln!(out, "Sync: no bound projects")?;
                return Ok(());
            }
            for p in &report.projects {
                let rules = p.rules.unwrap_or("skipped");
                writeln!(
                    out,
                    "{}: rules {}, {} harness change(s)",
                    p.project.display(),
                    rules,
                    p.harness_changes,
                )?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::harness_filter_set;

    /// Empty `--harness` → `None` (no filter; reconcile the full effective set).
    #[test]
    fn empty_harness_list_is_no_filter() {
        assert!(harness_filter_set(&[]).is_none());
    }

    /// A single `--harness X` → a one-element set containing exactly X.
    #[test]
    fn single_harness_is_singleton_set() {
        let set = harness_filter_set(&["cursor".to_string()]).expect("some");
        assert_eq!(set.len(), 1);
        assert!(set.contains("cursor"));
    }

    /// Repeated `--harness a --harness b` → the set {a, b}.
    #[test]
    fn repeated_harness_builds_the_set() {
        let set = harness_filter_set(&["cursor".to_string(), "codex".to_string()]).expect("some");
        assert_eq!(set.len(), 2);
        assert!(set.contains("cursor"));
        assert!(set.contains("codex"));
    }

    /// Aliases resolve to their canonical name BEFORE the set membership check:
    /// `antigravity-cli` → `gemini`, and naming both collapses to one entry.
    #[test]
    fn aliases_resolve_and_dedupe_before_membership() {
        let set = harness_filter_set(&["antigravity-cli".to_string()]).expect("some");
        assert!(
            set.contains("gemini"),
            "antigravity-cli must resolve to gemini: {set:?}",
        );

        // antigravity-cli + gemini collapse to a single `gemini` entry.
        let collapsed = harness_filter_set(&["antigravity-cli".to_string(), "gemini".to_string()])
            .expect("some");
        assert_eq!(
            collapsed.len(),
            1,
            "alias + canonical collapse: {collapsed:?}"
        );
        assert!(collapsed.contains("gemini"));
    }
}
