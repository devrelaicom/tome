//! `tome meta {list,add,remove}` — install / list / remove Tome's bundled
//! meta skills across the user's harnesses.
//!
//! Mirrors the project's silent-compute / emit-wrapper split: the per-location
//! work (`install_to_targets` / `remove_from_targets`) builds a report with
//! **forward-progress `first_error`** (a failure at one location never rolls back
//! the others, exactly like `reconcile_agents`), and the thin `*_run` wrappers
//! emit per `mode` then surface the highest-precedence failure as the process
//! exit code. Reports land on **stdout**; the error summary lands on **stderr**
//! (via `main.rs::write_error`), so a partial-failure JSON report stays a single
//! well-formed object on stdout.
//!
//! Target resolution: explicit `--harness` (repeatable) names win; otherwise
//! every **detected** harness that consumes native skills (existence-only probe,
//! FR-008a). Project scope writes under the resolved project root (or CWD when
//! not inside a `.tome` project); `--global` writes under the user home.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::authoring::meta::{self as meta_skill, DriftState, RemoveOutcome};
use crate::cli::{MetaAddArgs, MetaCommand, MetaListArgs, MetaRemoveArgs};
use crate::commands::harness::home_root;
use crate::error::TomeError;
use crate::harness::with_effective_modules;
use crate::output::{Mode, write_json};
use crate::workspace::ResolvedScope;

/// Subcommand dispatcher invoked by `main.rs`.
pub fn run(cmd: MetaCommand, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    match cmd {
        MetaCommand::List(args) => list_run(args, scope, mode),
        MetaCommand::Add(args) => add_run(args, scope, mode),
        MetaCommand::Remove(args) => remove_run(args, scope, mode),
    }
}

// ---------------------------------------------------------------------------
// Scope + target resolution
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    Project,
    Global,
}

impl Scope {
    fn as_str(self) -> &'static str {
        match self {
            Scope::Project => "project",
            Scope::Global => "global",
        }
    }
}

/// One resolved install/remove location.
struct Target {
    harness: String,
    scope: Scope,
    /// The harness's skills root (e.g. `<project>/.claude/skills`).
    dir: PathBuf,
}

/// Names of every supported harness that consumes native skills.
fn native_skill_harness_names() -> Vec<String> {
    with_effective_modules(|mods| {
        mods.iter()
            .filter(|m| m.supports_native_skills())
            .map(|m| m.name().to_string())
            .collect()
    })
}

/// Enumerate the `(harness_name, skills-root dir)` install targets for a SINGLE
/// scope — the **one** SSOT every (harness × scope × dir) enumeration routes
/// through (the installer `resolve_targets`, the `list` projection, and the
/// doctor `meta_drift::candidates`). Promoted here at its second consumer rather
/// than letting each surface hand-roll the gating + dir-resolution and drift.
///
/// Gating matches the installer exactly:
/// - `explicit` non-empty → select the named harnesses (validated upstream as
///   skill-capable; an unknown name simply matches nothing here);
/// - `explicit` empty → select every harness the installer would itself detect
///   via `m.detect(home)` (existence-only probe, FR-008a).
///
/// A harness that does not consume native skills (`!supports_native_skills()`)
/// is always skipped. The skills root is `m.skill_dir(project_root)` for
/// [`Scope::Project`] (requires a `project_root`) or `m.skill_dir_global(home)`
/// for [`Scope::Global`]; a harness with no resolvable dir for the scope is
/// dropped. Returned in effective-registry iteration order.
pub(crate) fn skill_targets_for_scope(
    home: &Path,
    scope: Scope,
    project_root: Option<&Path>,
    explicit: &[String],
) -> Vec<(&'static str, PathBuf)> {
    let by_name = !explicit.is_empty();
    with_effective_modules(|mods| {
        let mut out = Vec::new();
        for m in mods {
            if !m.supports_native_skills() {
                continue;
            }
            let selected = if by_name {
                explicit.iter().any(|h| h == m.name())
            } else {
                m.detect(home)
            };
            if !selected {
                continue;
            }
            let dir = match scope {
                Scope::Project => project_root.and_then(|p| m.skill_dir(p)),
                Scope::Global => m.skill_dir_global(home),
            };
            if let Some(dir) = dir {
                out.push((m.name(), dir));
            }
        }
        out
    })
}

/// Resolve the project root used for project-scope installs: the resolved
/// `.tome` project root when inside one, otherwise the current directory (so
/// `tome meta add` works anywhere, landing `./.<harness>/skills/`).
fn project_root_for(scope: &ResolvedScope) -> Result<PathBuf, TomeError> {
    match &scope.project_root {
        Some(root) => Ok(root.clone()),
        None => std::env::current_dir().map_err(TomeError::Io),
    }
}

/// Resolve the (harness, scope, dir) targets for an add/remove run.
///
/// - explicit `--harness` names → those harnesses (validated; an unknown or
///   non-skill harness is a usage error, exit 2);
/// - otherwise every detected native-skill harness (existence-only probe).
///   An empty all-detected set → [`TomeError::NoHarnessDetected`] (89).
fn resolve_targets(
    harnesses: &[String],
    global: bool,
    scope: &ResolvedScope,
) -> Result<Vec<Target>, TomeError> {
    let home = home_root()?;
    let install_scope = if global {
        Scope::Global
    } else {
        Scope::Project
    };
    let project_root = if global {
        // Global scope never consults the project root.
        None
    } else {
        Some(project_root_for(scope)?)
    };
    let explicit = !harnesses.is_empty();

    // Validate explicit harness names up front (exit 2, before any write).
    if explicit {
        let known = native_skill_harness_names();
        for h in harnesses {
            if !known.iter().any(|k| k == h) {
                return Err(TomeError::Usage(format!(
                    "`{h}` is not a harness that consumes native skills (choose from: {})",
                    known.join(", ")
                )));
            }
        }
    }

    // Build the targets from the SSOT enumeration helper (same gating the
    // doctor `meta_drift::candidates` now uses).
    let targets: Vec<Target> =
        skill_targets_for_scope(&home, install_scope, project_root.as_deref(), harnesses)
            .into_iter()
            .map(|(harness, dir)| Target {
                harness: harness.to_string(),
                scope: install_scope,
                dir,
            })
            .collect();

    if !explicit && targets.is_empty() {
        return Err(TomeError::NoHarnessDetected);
    }
    Ok(targets)
}

// ---------------------------------------------------------------------------
// `meta add`
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct LocationOutcome {
    harness: String,
    scope: &'static str,
    dir: String,
    /// `installed` | `already-current` | `removed` | `not-present` | `failed`.
    result: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct ActionReport {
    skill_id: String,
    locations: Vec<LocationOutcome>,
    #[serde(skip)]
    first_error: Option<TomeError>,
}

fn add_run(args: MetaAddArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    // Unknown skill id fails closed before any target resolution (87).
    if meta_skill::find(&args.skill_id).is_none() {
        // OUTCOME-bearing: emit `Failed` even on a pre-report hard fail.
        emit_meta_telemetry(crate::telemetry::event::MetaAction::Add, None);
        return Err(meta_skill::not_found(&args.skill_id));
    }
    let targets = match resolve_targets(&args.harnesses, args.global, scope) {
        Ok(t) => t,
        Err(e) => {
            emit_meta_telemetry(crate::telemetry::event::MetaAction::Add, None);
            return Err(e);
        }
    };
    let report = install_to_targets(&args.skill_id, &targets, args.force);
    emit_meta_telemetry(crate::telemetry::event::MetaAction::Add, Some(&report));
    emit_action(&report, mode)?;
    match report.first_error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

/// Emit one `tome.meta_action` event with the forward-progress outcome:
/// `None` report (a pre-report hard fail) → `Failed`; a report with no
/// `first_error` → `Ok`; a report with a `first_error` but at least one
/// succeeding location → `Partial`; otherwise `Failed`. One infallible
/// `enqueue`; never alters control flow.
fn emit_meta_telemetry(action: crate::telemetry::event::MetaAction, report: Option<&ActionReport>) {
    use crate::telemetry::event::{MetaActionEvent, Outcome};
    let outcome = match report {
        None => Outcome::Failed,
        Some(r) if r.first_error.is_none() => Outcome::Ok,
        Some(r) => {
            // Some failed (first_error set). Partial iff at least one location
            // did NOT fail; otherwise every location failed → Failed.
            let any_ok = r.locations.iter().any(|l| l.result != "failed");
            if any_ok {
                Outcome::Partial
            } else {
                Outcome::Failed
            }
        }
    };
    crate::telemetry::emit(MetaActionEvent { action, outcome });
}

/// Install `skill_id` into every target, forward-progress. An up-to-date
/// location is a no-op unless `force` (NFR-010, avoid churn).
///
/// `first_error` is the FIRST failure in registry-iteration order. The contract
/// asks for the "highest-precedence" code, and that holds trivially here: an
/// unknown skill (87) and no-harness (89) are rejected BEFORE this loop, so
/// every in-loop failure is a `MetaInstallFailed` (88) — there is only ever one
/// failure code in play, making first-error ≡ highest-precedence.
fn install_to_targets(skill_id: &str, targets: &[Target], force: bool) -> ActionReport {
    let mut locations = Vec::with_capacity(targets.len());
    let mut first_error: Option<TomeError> = None;
    for t in targets {
        let outcome = if !force
            && matches!(
                meta_skill::drift_probe(skill_id, &t.dir),
                DriftState::UpToDate
            ) {
            LocationOutcome {
                harness: t.harness.clone(),
                scope: t.scope.as_str(),
                dir: t.dir.display().to_string(),
                result: "already-current",
                revision: meta_skill::find(skill_id).map(|s| s.revision.to_string()),
                error: None,
            }
        } else {
            match meta_skill::install_skill(skill_id, &t.dir) {
                Ok(at) => LocationOutcome {
                    harness: t.harness.clone(),
                    scope: t.scope.as_str(),
                    dir: t.dir.display().to_string(),
                    result: "installed",
                    revision: Some(at.revision),
                    error: None,
                },
                Err(e) => {
                    let location = LocationOutcome {
                        harness: t.harness.clone(),
                        scope: t.scope.as_str(),
                        dir: t.dir.display().to_string(),
                        result: "failed",
                        revision: None,
                        error: Some(e.to_string()),
                    };
                    if first_error.is_none() {
                        first_error = Some(e);
                    }
                    location
                }
            }
        };
        locations.push(outcome);
    }
    ActionReport {
        skill_id: skill_id.to_owned(),
        locations,
        first_error,
    }
}

// ---------------------------------------------------------------------------
// `meta remove`
// ---------------------------------------------------------------------------

fn remove_run(args: MetaRemoveArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    if meta_skill::find(&args.skill_id).is_none() {
        emit_meta_telemetry(crate::telemetry::event::MetaAction::Remove, None);
        return Err(meta_skill::not_found(&args.skill_id));
    }
    let targets = match resolve_targets(&args.harnesses, args.global, scope) {
        Ok(t) => t,
        Err(e) => {
            emit_meta_telemetry(crate::telemetry::event::MetaAction::Remove, None);
            return Err(e);
        }
    };
    let report = remove_from_targets(&args.skill_id, &targets);
    emit_meta_telemetry(crate::telemetry::event::MetaAction::Remove, Some(&report));
    emit_action(&report, mode)?;
    match report.first_error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

fn remove_from_targets(skill_id: &str, targets: &[Target]) -> ActionReport {
    let mut locations = Vec::with_capacity(targets.len());
    let mut first_error: Option<TomeError> = None;
    for t in targets {
        let outcome = match meta_skill::remove_skill(skill_id, &t.dir) {
            Ok(RemoveOutcome::Removed) => location(t, "removed", None, None),
            Ok(RemoveOutcome::NotPresent) => location(t, "not-present", None, None),
            Err(e) => {
                let l = location(t, "failed", None, Some(e.to_string()));
                if first_error.is_none() {
                    first_error = Some(e);
                }
                l
            }
        };
        locations.push(outcome);
    }
    ActionReport {
        skill_id: skill_id.to_owned(),
        locations,
        first_error,
    }
}

fn location(
    t: &Target,
    result: &'static str,
    revision: Option<String>,
    error: Option<String>,
) -> LocationOutcome {
    LocationOutcome {
        harness: t.harness.clone(),
        scope: t.scope.as_str(),
        dir: t.dir.display().to_string(),
        result,
        revision,
        error,
    }
}

fn emit_action(report: &ActionReport, mode: Mode) -> Result<(), TomeError> {
    match mode {
        Mode::Json => write_json(report),
        Mode::Human => {
            let mut out = std::io::stdout().lock();
            if report.locations.is_empty() {
                writeln!(out, "{}: no target locations", report.skill_id)?;
            }
            for l in &report.locations {
                let tail = match (l.revision.as_deref(), l.error.as_deref()) {
                    (Some(rev), _) => format!(" ({rev})"),
                    (None, Some(err)) => format!(" — {err}"),
                    _ => String::new(),
                };
                writeln!(
                    out,
                    "  {} {}/{} {}{}",
                    symbol(l.result),
                    l.harness,
                    l.scope,
                    l.dir,
                    tail
                )?;
            }
            Ok(())
        }
    }
}

fn symbol(result: &str) -> &'static str {
    match result {
        "installed" => "+",
        "already-current" => "=",
        "removed" => "-",
        "not-present" => "·",
        _ => "✗",
    }
}

// ---------------------------------------------------------------------------
// `meta list`
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct ListReport {
    skills: Vec<SkillStatus>,
}

#[derive(Debug, Serialize)]
struct SkillStatus {
    id: String,
    summary: String,
    revision: String,
    /// `harness` → `scope` → `up-to-date | stale | not-installed`.
    status: BTreeMap<String, BTreeMap<String, String>>,
}

fn list_run(_args: MetaListArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let home = home_root()?;
    // Read-only listing degrades a missing/unreadable CWD to global-only status
    // (`.ok()`), unlike the write paths (`add`/`remove`) which propagate the
    // `current_dir()` failure — a pure projection should still report what it can.
    let project_root = project_root_for(scope).ok();

    // Resolve each native-skill harness's project + global skills root once.
    let dirs: Vec<(String, Option<PathBuf>, Option<PathBuf>)> = with_effective_modules(|mods| {
        mods.iter()
            .filter(|m| m.supports_native_skills())
            .map(|m| {
                let proj = project_root.as_deref().and_then(|p| m.skill_dir(p));
                let glob = m.skill_dir_global(&home);
                (m.name().to_string(), proj, glob)
            })
            .collect()
    });

    let mut skills = Vec::new();
    for skill in meta_skill::all() {
        let mut status: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        for (harness, proj, glob) in &dirs {
            let mut by_scope = BTreeMap::new();
            if let Some(dir) = proj {
                by_scope.insert("project".to_string(), drift_label(skill.id, dir));
            }
            if let Some(dir) = glob {
                by_scope.insert("global".to_string(), drift_label(skill.id, dir));
            }
            status.insert(harness.clone(), by_scope);
        }
        skills.push(SkillStatus {
            id: skill.id.to_string(),
            summary: skill.summary.to_string(),
            revision: skill.revision.to_string(),
            status,
        });
    }
    let report = ListReport { skills };

    match mode {
        Mode::Json => write_json(&report),
        Mode::Human => emit_list_human(&report),
    }
}

fn drift_label(skill_id: &str, dir: &Path) -> String {
    match meta_skill::drift_probe(skill_id, dir) {
        DriftState::UpToDate => "up-to-date",
        DriftState::Stale { .. } => "stale",
        DriftState::MissingButExpected => "not-installed",
    }
    .to_string()
}

fn emit_list_human(report: &ListReport) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    if report.skills.is_empty() {
        writeln!(out, "no bundled meta skills")?;
        return Ok(());
    }
    for s in &report.skills {
        writeln!(out, "{} — {}  [{}]", s.id, s.summary, s.revision)?;
        for (harness, by_scope) in &s.status {
            for (scope, state) in by_scope {
                writeln!(out, "    {harness}/{scope}: {state}")?;
            }
        }
    }
    Ok(())
}
