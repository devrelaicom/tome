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
//! ## Bare `tome sync` outside a project (issue #303)
//!
//! When no project marker resolves and `--all` was not passed, `tome sync`
//! does NOT hard-error. It falls back to the `--all` fan-out over the resolved
//! workspace's bound projects (reusing [`sync_all`], so it inherits every bit
//! of the `--all` writer safety and forward-progress), printing a short note to
//! stderr in human mode so the user isn't surprised it acted outside the CWD.
//! `--json` output is byte-identical to `--all`. Only when the workspace has NO
//! bound projects does it stay an error — a detect-and-suggest [`TomeError::Usage`]
//! (exit 2) naming the concrete next step (`tome workspace use` / `tome sync --all`).
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
    /// The per-file actions behind `harness_changes`, for the HUMAN renderer's
    /// per-harness enumeration (issue #424). `serde(skip)`: the `--json` wire
    /// shape stays exactly as before — the same data already rides
    /// [`crate::harness::sync::SyncOutcome`] on the surfaces that emit it.
    #[serde(skip)]
    pub changes: Vec<ChangeLine>,
}

/// One enumerated file-level action for the human renderer: which harness,
/// what happened (`+`/`~`/`-`), and the touched path. Never serialised.
#[derive(Debug, Clone)]
pub struct ChangeLine {
    pub op: ChangeOp,
    pub harness: String,
    pub path: PathBuf,
}

/// What happened to one file: added (`+`), updated (`~`), or removed (`-`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeOp {
    Add,
    Update,
    Remove,
}

impl ChangeOp {
    fn glyph(self) -> char {
        match self {
            ChangeOp::Add => '+',
            ChangeOp::Update => '~',
            ChangeOp::Remove => '-',
        }
    }
}

/// One project the `--all` fan-out could not sync (issue #426). The error is
/// its display string (already credential-scrubbed at the source boundary).
#[derive(Debug, Serialize)]
pub struct ProjectFailure {
    pub project: PathBuf,
    pub error: String,
}

/// Aggregate of every project this `tome sync` invocation touched.
#[derive(Debug, Serialize)]
pub struct SyncReport {
    pub projects: Vec<ProjectOutcome>,
    /// Per-project failures from the `--all` fan-out (issue #426). Appended
    /// LAST and omitted when empty, so the wire shape is byte-identical to
    /// before unless a project actually failed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<ProjectFailure>,
    /// The first per-project error, preserved as the exit-code source (the
    /// pre-existing first-error semantics). Never serialised — `failures`
    /// carries the wire form.
    #[serde(skip)]
    pub first_error: Option<TomeError>,
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

    let mut report = if args.all {
        sync_all(&ws, &args, paths)?
    } else if let Some(project_root) = scope.project_root.as_deref() {
        // In a project: sync exactly that project, unchanged.
        let mut report = SyncReport {
            projects: Vec::new(),
            failures: Vec::new(),
            first_error: None,
        };
        report
            .projects
            .push(sync_one_project(&ws, project_root, &args, paths)?);
        report
    } else {
        // No project marker resolved and `--all` was not passed. Rather than
        // hard-error, fan out to the resolved workspace's bound projects —
        // exactly what `--all` does (issue #303). We REUSE the vetted `--all`
        // path (`sync_all`) so this inherits all of its writer safety (symlink
        // refusal, marker-bounded edits, atomic writes) and forward-progress
        // fan-out; it is NOT a re-implemented fan-out.
        match sync_all(&ws, &args, paths) {
            // Fanned out to at least one bound project. Print a short human
            // note so the user isn't surprised `tome sync` acted outside CWD;
            // `--json` stays byte-identical to what `--all` already emits.
            Ok(report) if !report.projects.is_empty() || !report.failures.is_empty() => {
                if mode == Mode::Human {
                    eprintln!(
                        "note: no project marker here; syncing every project bound to workspace `{}` (like --all)",
                        ws.as_str(),
                    );
                }
                report
            }
            // No bound projects to fan out to. `Ok(empty)` is the pre-bootstrap
            // / no-bindings case; `WorkspaceNotFound` is a resolved workspace
            // with no registry row (also "no bindings"). Both collapse to the
            // same detect-and-suggest usage error naming the concrete next
            // steps — never a bare `WorkspaceNotFound` (exit 13) here.
            Ok(_) | Err(TomeError::WorkspaceNotFound { .. }) => {
                return Err(TomeError::Usage(no_bindings_hint(&ws)));
            }
            // A pre-fan-out failure (unreadable central DB, …). Surface it so
            // the exit code reflects the real failure.
            Err(e) => return Err(e),
        }
    };

    // #426: emit the per-project report (successes AND failures) FIRST, then
    // surface the first per-project error so the exit code is preserved
    // exactly as before — partial progress is now visible instead of silent.
    let first_error = report.first_error.take();
    emit(&report, mode, args.dry_run)?;
    match first_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Detect-and-suggest message for a bare `tome sync` (no project marker,
/// no `--all`) run against a workspace with no bound projects. Names the two
/// concrete next steps rather than the bare "requires a project marker" of the
/// old branch (issue #303).
fn no_bindings_hint(ws: &WorkspaceName) -> String {
    format!(
        "`tome sync`: no project marker here and no projects bound to workspace `{}` — \
         run `tome workspace use {}` inside a project to bind it, or `tome sync --all` \
         once you have bound projects",
        ws.as_str(),
        ws.as_str(),
    )
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
        let classification = match crate::workspace::sync::sync_rules_to_project_with(
            &source,
            project_root,
            ws,
            args.dry_run,
        )? {
            RulesSync::Synced => "synced",
            RulesSync::Unchanged => "unchanged",
            RulesSync::MissingProjectDir => "missing",
        };
        Some(classification)
    };

    let (harness_changes, changes) = if args.rules_only {
        (0, Vec::new())
    } else {
        let home = crate::commands::harness::home_root()?;
        // `--force` is not exposed on `tome sync` (matching `tome harness
        // sync`); a clash is resolved by re-binding with
        // `tome workspace use --force`.
        let mut deps = crate::harness::sync::build_deps(paths, &home, ws, false);
        deps.only_harness = harness_filter_set(&args.harness);
        deps.dry_run = args.dry_run;
        let outcome = crate::harness::sync::sync_project(project_root, &deps)?;

        // Telemetry parity with the removed `tome harness sync`: emit one
        // `tome.harness_action{Sync}` per DISTINCT harness that actually had a
        // change. Best-effort, success-path only; a fully-idempotent reconcile
        // (no changes) emits nothing. Unmapped harness names are SKIPped by
        // `emit_harness_action`. A dry run changed nothing — emit nothing.
        if !args.dry_run {
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
        }

        let count = outcome.added.len() + outcome.updated.len() + outcome.removed.len();
        (count, change_lines(&outcome))
    };

    Ok(ProjectOutcome {
        project: project_root.to_path_buf(),
        rules,
        harness_changes,
        changes,
    })
}

/// Flatten a [`crate::harness::sync::SyncOutcome`]'s added/updated/removed
/// vectors into per-file [`ChangeLine`]s for the human renderer, grouped by
/// harness (lexicographic) with each harness's adds, then updates, then
/// removals in recorded order.
fn change_lines(outcome: &crate::harness::sync::SyncOutcome) -> Vec<ChangeLine> {
    let mut lines: Vec<ChangeLine> = Vec::new();
    for (op, set) in [
        (ChangeOp::Add, &outcome.added),
        (ChangeOp::Update, &outcome.updated),
        (ChangeOp::Remove, &outcome.removed),
    ] {
        for change in set {
            lines.push(ChangeLine {
                op,
                harness: change.harness.clone(),
                path: change.path.clone(),
            });
        }
    }
    // Group by harness for readability; the sort is stable, so within one
    // harness the add → update → remove order above is preserved.
    lines.sort_by(|a, b| a.harness.cmp(&b.harness));
    lines
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
/// (on `failures` for the report, on `first_error` for the exit code) and the
/// loop continues, so every reachable project is attempted. Only PRE-fan-out
/// failures (unreadable central DB, missing workspace row) return `Err`.
pub fn sync_all(
    ws: &WorkspaceName,
    args: &SyncArgs,
    paths: &Paths,
) -> Result<SyncReport, TomeError> {
    let mut report = SyncReport {
        projects: Vec::new(),
        failures: Vec::new(),
        first_error: None,
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

    for project_root in project_roots {
        match sync_one_project(ws, &project_root, args, paths) {
            Ok(outcome) => report.projects.push(outcome),
            Err(e) => {
                // Forward-progress: keep going so every reachable project is
                // attempted; surface the first failure for the exit code and
                // EVERY failure on the report (#426), not just the log.
                tracing::warn!(
                    workspace = ws.as_str(),
                    project = %project_root.display(),
                    error = %e,
                    "sync: project failed; continuing",
                );
                report.failures.push(ProjectFailure {
                    project: project_root,
                    error: e.to_string(),
                });
                if report.first_error.is_none() {
                    report.first_error = Some(e);
                }
            }
        }
    }

    Ok(report)
}

/// Run the SAME propagation `tome sync --all` performs — RULES.md write +
/// harness reconcile over every project bound to `ws` — as an inline follow-up
/// to a workspace-state change (`plugin enable`/`disable` with `--sync`).
///
/// This is the single shared entry point those `--sync` flags route through, so
/// the inline sync inherits ALL of `sync_project`'s writer safety
/// (structural-match-only removal, symlink refusal, marker-bounded edits,
/// atomic writes) and the forward-progress `first_error` fan-out — it is NOT a
/// second, hand-rolled sync path.
///
/// Scope: every bound project of the resolved workspace. This matches the scope
/// the RULES.md propagation already reaches on enable/disable (via
/// `regenerate_for_trigger` → `write_workspace_rules` →
/// `sync_workspace_rules_to_bound_projects`), so the harness-file reconcile
/// lands wherever the RULES.md write already lands — no broader, no narrower.
///
/// Ordering / failure contract: callers MUST run this AFTER the enable/disable
/// state change has committed. A sync failure surfaces the underlying
/// `sync_project` error (and its exit code, e.g. 43/44/45/19/7) but does NOT
/// undo the committed enable/disable — the caller reports the state change as
/// done and the sync as failed.
///
/// Why the FULL sync (both halves), not `harness_only: true`: the enable/disable
/// caller has already written RULES.md before this runs (`regenerate_for_trigger`
/// → `write_workspace_rules` fans RULES.md out to bound projects). So this
/// function's RULES.md half is a REDUNDANT-but-idempotent re-run — writing bytes
/// that already match is a no-op (`sync_rules_to_project` classifies it
/// `unchanged` and skips the write). It is deliberately kept, NOT optimised to
/// `harness_only: true`: routing through the ONE `sync_all` SSOT (the exact path
/// `tome sync --all` takes) guarantees this reaches EVERY project the walk finds
/// — including any that the trigger's fan-out reached. A future reader must NOT
/// "optimise away" the rules half; the duplication is free (idempotent) and the
/// single-SSOT guarantee is the point.
pub fn sync_bound_projects(ws: &WorkspaceName, paths: &Paths) -> Result<SyncReport, TomeError> {
    let args = SyncArgs {
        all: true,
        rules_only: false,
        harness_only: false,
        harness: Vec::new(),
        dry_run: false,
    };
    let mut report = sync_all(ws, &args, paths)?;
    // Preserve the pre-#426 contract for the inline `--sync` callers: a
    // per-project failure IS the call's failure (the enable/disable caller
    // reports the state change as done and the sync as failed).
    if let Some(e) = report.first_error.take() {
        return Err(e);
    }
    Ok(report)
}

/// Emit the report per output mode. Human mode prints the per-project
/// rendering from [`render_human`]; `--json` emits the wire-stable
/// [`SyncReport`] (dry-run or not — the flag was explicit, the report shape
/// is identical).
fn emit(report: &SyncReport, mode: Mode, dry_run: bool) -> Result<(), TomeError> {
    match mode {
        Mode::Json => write_json(report),
        Mode::Human => {
            let mut out = std::io::stdout().lock();
            out.write_all(render_human(report, dry_run).as_bytes())?;
            Ok(())
        }
    }
}

/// Render the human-mode report (issues #424 / #425 / #426).
///
/// Per project: when nothing changed, the pre-#424 one-liner
/// (`<path>: rules synced, 0 harness change(s)`); when files changed, the
/// count is replaced by a per-harness enumeration of file-level actions
/// (`+` added / `~` updated / `-` removed, paths relative to the project
/// where possible). A dry run gets a leading banner; failed projects (#426)
/// each get a one-line `FAILED` entry after the successes.
fn render_human(report: &SyncReport, dry_run: bool) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    if dry_run {
        out.push_str("Dry run: no files written. Actions below are what `tome sync` would do.\n");
    }
    if report.projects.is_empty() && report.failures.is_empty() {
        out.push_str("Sync: no bound projects\n");
        return out;
    }
    for p in &report.projects {
        let rules = p.rules.unwrap_or("skipped");
        if p.changes.is_empty() {
            // Nothing changed: collapse to the pre-#424 one-liner.
            let _ = writeln!(
                out,
                "{}: rules {}, {} harness change(s)",
                p.project.display(),
                rules,
                p.harness_changes,
            );
            continue;
        }
        let _ = writeln!(out, "{}: rules {}", p.project.display(), rules);
        let width = p.changes.iter().map(|c| c.harness.len()).max().unwrap_or(0);
        for c in &p.changes {
            // Paths under the project render relative; home-based sinks (e.g.
            // `~/.codex/config.toml`) keep their absolute form.
            let path = c.path.strip_prefix(&p.project).unwrap_or(&c.path);
            let _ = writeln!(
                out,
                "  {:<width$}  {} {}",
                c.harness,
                c.op.glyph(),
                path.display(),
            );
        }
    }
    for f in &report.failures {
        let _ = writeln!(out, "{}: FAILED — {}", f.project.display(), f.error);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(
        project: &str,
        rules: Option<&'static str>,
        changes: Vec<ChangeLine>,
    ) -> ProjectOutcome {
        ProjectOutcome {
            project: PathBuf::from(project),
            rules,
            harness_changes: changes.len(),
            changes,
        }
    }

    fn line(op: ChangeOp, harness: &str, path: &str) -> ChangeLine {
        ChangeLine {
            op,
            harness: harness.to_string(),
            path: PathBuf::from(path),
        }
    }

    /// #424: nothing changed → collapse to the pre-existing one-liner
    /// (`rules synced, 0 harness change(s)`), byte-identical to before.
    #[test]
    fn render_unchanged_project_keeps_the_one_liner() {
        let report = SyncReport {
            projects: vec![outcome("/proj", Some("unchanged"), Vec::new())],
            failures: Vec::new(),
            first_error: None,
        };
        assert_eq!(
            render_human(&report, false),
            "/proj: rules unchanged, 0 harness change(s)\n",
        );
    }

    /// #424: changed files are enumerated per harness with `+`/`~`/`-` ops,
    /// project-relative paths where possible, absolute otherwise.
    #[test]
    fn render_changed_project_enumerates_per_harness() {
        let report = SyncReport {
            projects: vec![outcome(
                "/proj",
                Some("synced"),
                vec![
                    line(ChangeOp::Add, "cursor", "/proj/.cursor/mcp.json"),
                    line(ChangeOp::Update, "cursor", "/proj/AGENTS.md"),
                    line(ChangeOp::Remove, "codex", "/home/u/.codex/config.toml"),
                ],
            )],
            failures: Vec::new(),
            first_error: None,
        };
        let rendered = render_human(&report, false);
        assert_eq!(
            rendered,
            "/proj: rules synced\n\
             \x20 cursor  + .cursor/mcp.json\n\
             \x20 cursor  ~ AGENTS.md\n\
             \x20 codex   - /home/u/.codex/config.toml\n",
        );
        // The count phrasing is REPLACED by the enumeration when files changed.
        assert!(!rendered.contains("harness change(s)"), "{rendered}");
    }

    /// #425: a dry run gets a leading banner; the body uses the SAME renderer.
    #[test]
    fn render_dry_run_prefixes_banner() {
        let report = SyncReport {
            projects: vec![outcome(
                "/proj",
                Some("synced"),
                vec![line(ChangeOp::Add, "cursor", "/proj/.cursor/mcp.json")],
            )],
            failures: Vec::new(),
            first_error: None,
        };
        let rendered = render_human(&report, true);
        assert!(
            rendered.starts_with("Dry run: no files written."),
            "{rendered}"
        );
        assert!(
            rendered.contains("  cursor  + .cursor/mcp.json\n"),
            "{rendered}"
        );
    }

    /// #426: failed projects each render one FAILED line after the successes.
    #[test]
    fn render_failures_one_line_per_project() {
        let report = SyncReport {
            projects: vec![outcome("/ok", Some("synced"), Vec::new())],
            failures: vec![ProjectFailure {
                project: PathBuf::from("/bad"),
                error: "io: permission denied".to_string(),
            }],
            first_error: None,
        };
        let rendered = render_human(&report, false);
        assert!(
            rendered.contains("/ok: rules synced, 0 harness change(s)\n"),
            "{rendered}"
        );
        assert!(
            rendered.ends_with("/bad: FAILED — io: permission denied\n"),
            "{rendered}"
        );
    }

    /// `change_lines` groups by harness (stable sort) while keeping each
    /// harness's add → update → remove order.
    #[test]
    fn change_lines_groups_by_harness_keeping_op_order() {
        use crate::harness::sync::{SyncChange, SyncOutcome, SyncSubsystem};
        let mk = |harness: &str, path: &str| SyncChange {
            harness: harness.to_string(),
            subsystem: SyncSubsystem::Rules,
            path: PathBuf::from(path),
        };
        let outcome = SyncOutcome {
            added: vec![mk("cursor", "/p/a"), mk("codex", "/p/b")],
            updated: vec![mk("cursor", "/p/c")],
            removed: vec![mk("codex", "/p/d")],
            leave_alones: 0,
            decisions: Vec::new(),
        };
        let lines = change_lines(&outcome);
        let flat: Vec<(String, ChangeOp)> =
            lines.iter().map(|l| (l.harness.clone(), l.op)).collect();
        assert_eq!(
            flat,
            vec![
                ("codex".to_string(), ChangeOp::Add),
                ("codex".to_string(), ChangeOp::Remove),
                ("cursor".to_string(), ChangeOp::Add),
                ("cursor".to_string(), ChangeOp::Update),
            ],
        );
    }

    /// #426 wire shape: `failures` is omitted when empty (byte-identical to
    /// the pre-#426 report) and appended LAST when present; `changes` and
    /// `first_error` never serialise.
    #[test]
    fn sync_report_json_failures_appended_last_and_omitted_when_empty() {
        let clean = SyncReport {
            projects: vec![outcome(
                "/p",
                Some("synced"),
                vec![line(ChangeOp::Add, "h", "/p/x")],
            )],
            failures: Vec::new(),
            first_error: None,
        };
        assert_eq!(
            serde_json::to_string(&clean).unwrap(),
            r#"{"projects":[{"project":"/p","rules":"synced","harness_changes":1}]}"#,
        );

        let failed = SyncReport {
            projects: Vec::new(),
            failures: vec![ProjectFailure {
                project: PathBuf::from("/bad"),
                error: "boom".to_string(),
            }],
            first_error: Some(TomeError::Usage("boom".to_string())),
        };
        assert_eq!(
            serde_json::to_string(&failed).unwrap(),
            r#"{"projects":[],"failures":[{"project":"/bad","error":"boom"}]}"#,
        );
    }

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
