//! Phase 9 / US4 meta-skill DRIFT doctor surface: a read-only projection over
//! the SAME (harness × scope) candidate locations the `tome meta` installer
//! targets, plus the `--fix` repair.
//!
//! Mirrors the [`crate::doctor::cutover`] read-only-check + repair shape: a
//! pure enumeration that returns data ([`check`]), and a repair that re-runs
//! the idempotent shared writer ([`repair`]). It reuses the shared
//! [`crate::authoring::meta`] helpers — `all()` for the embedded registry,
//! `drift_probe` for the bounded UTF-8-fail-closed read, and `install_skill`
//! for the atomic, symlink-checked write — so there is NO duplicated drift
//! logic and NO bespoke writer (contract `doctor-meta-drift.md` §Invariants).
//!
//! ## Read-only check (FR-031/031a/031b)
//! - Candidate locations are re-derived the SAME way the installer targets
//!   (installs are not indexed — NFR-003): [`candidates`] enumerates the
//!   effective harness registry and detects via `HarnessModule::detect`, exactly
//!   like [`crate::commands::meta`]'s `resolve_targets` (true SSOT — see the
//!   doc comment on [`candidates`]).
//! - GLOBAL: a row is emitted only for a **detected** harness (gated on the
//!   harness's own `detect(&home)`) — the expectation source for
//!   `missing-but-expected` is a detected harness, never a guess (FR-031a).
//! - PROJECT: the `scope.project_root` IS the detection signal (we are inside
//!   that project), so a row is emitted per skill-capable harness with a
//!   resolvable project skill dir.
//! - Each candidate is probed via `drift_probe`, classifying it
//!   `up-to-date` | `stale` | `missing-but-expected`. Read is mtime-stable —
//!   `drift_probe` only `read`s (FR-124).
//!
//! ## Emit policy — stale + missing-but-expected ONLY (NOT up-to-date)
//! The contract heads this surface a "meta-skill **drift** check" and §Read-only
//! says it "**Surfaces in the report**" the drift classes; `up-to-date` is the
//! ABSENCE of drift. Emitting only the two drift classes means a clean system
//! yields an empty `Vec`, which (with the report field's
//! `skip_serializing_if = "Vec::is_empty"`) keeps the existing byte-stable
//! `doctor_json` wire-shape pin unchanged. Rows are sorted deterministically by
//! `(skill_id, harness, scope)` for the `--json` wire-shape pins (contract
//! §Read-only "BTreeMap-ordered").

use std::path::{Path, PathBuf};

use crate::authoring::meta::{self, DriftState};
use crate::commands::meta::{Scope, skill_targets_for_scope};
use crate::doctor::report::MetaSkillDrift;
use crate::error::TomeError;
use crate::workspace::ResolvedScope;

/// Wire string for the project scope.
const SCOPE_PROJECT: &str = "project";
/// Wire string for the global scope.
const SCOPE_GLOBAL: &str = "global";

/// One resolved candidate location: `(skill_id, harness, scope, dir)`. The dir
/// is the harness `skills/` root the installer would write under.
struct Candidate {
    skill_id: &'static str,
    harness: &'static str,
    scope: &'static str,
    dir: PathBuf,
}

/// Re-derive the installer's candidate set for the active scope (FR-031a).
///
/// The candidate set is derived through the **one** shared enumeration helper
/// [`skill_targets_for_scope`] — the true SSOT with the installer's
/// `resolve_targets`, so the two surfaces can never diverge on which
/// (harness × scope × dir) locations are in play. The helper:
///
/// - iterates the effective registry (so a test-injected
///   `HARNESS_MODULES_OVERRIDE` is honoured) and skips any harness that does not
///   consume native skills;
/// - gates the all-default case on the harness's OWN `detect(&home)` — the same
///   mechanism `resolve_targets` uses for the all-detected default;
/// - resolves the per-scope skills dir (`skill_dir` for project /
///   `skill_dir_global` for global).
///
/// Doctor passes **no explicit harness list** (it always surveys the
/// all-detected set) and surveys both scopes the installer can target across
/// invocations:
///
/// - **GLOBAL** candidates are gated on `detect(&home)` (the all-default path).
/// - **PROJECT** candidates are produced only when `scope.project_root` is
///   `Some` (doctor never invents a project root — it surveys the one it was
///   given) AND the harness is detected. The project root is **necessary but
///   not sufficient**: the harness must ALSO be detected under `home`, matching
///   the installer exactly (FR-031a). Before this routed through the shared
///   helper, the project branch gated on the project root alone — so
///   `doctor --fix` could write into an UNDETECTED harness's project dir, more
///   broadly than `meta add` ever would. The shared helper closes that.
///
/// `meta_skills` is verdict-neutral (it never feeds `overall`), so surveying
/// both scopes never trips `degraded`.
fn candidates(home: &Path, scope: &ResolvedScope) -> Vec<Candidate> {
    // Doctor surveys the all-detected set: no explicit harness selection.
    const NO_EXPLICIT: &[String] = &[];

    let mut out = Vec::new();
    for skill in meta::all() {
        // GLOBAL: detect-gated under `home`.
        for (harness, dir) in skill_targets_for_scope(home, Scope::Global, None, NO_EXPLICIT) {
            out.push(Candidate {
                skill_id: skill.id,
                harness,
                scope: SCOPE_GLOBAL,
                dir,
            });
        }
        // PROJECT: only when a project root was surveyed, AND still detect-gated
        // under `home` (the project root is necessary, not sufficient).
        if let Some(project_root) = scope.project_root.as_deref() {
            for (harness, dir) in
                skill_targets_for_scope(home, Scope::Project, Some(project_root), NO_EXPLICIT)
            {
                out.push(Candidate {
                    skill_id: skill.id,
                    harness,
                    scope: SCOPE_PROJECT,
                    dir,
                });
            }
        }
    }

    out
}

/// Read-only meta-skill drift projection (FR-031). For every candidate
/// location, probe `<dir>/<skill-id>/SKILL.md` via the shared
/// [`meta::drift_probe`] and classify. Emits only `stale` +
/// `missing-but-expected` rows (see module docs — `up-to-date` is the absence
/// of drift), sorted by `(skill_id, harness, scope)` for deterministic wire
/// shape. Makes NO changes (FR-124); the read path is mtime-stable.
pub fn check(home: &Path, scope: &ResolvedScope) -> Vec<MetaSkillDrift> {
    let mut rows: Vec<MetaSkillDrift> = candidates(home, scope)
        .into_iter()
        .filter_map(|c| {
            let state = match meta::drift_probe(c.skill_id, &c.dir) {
                // The absence of drift is not surfaced (keeps a clean system's
                // wire shape empty + byte-stable; see module docs).
                DriftState::UpToDate => return None,
                DriftState::Stale { .. } => "stale",
                DriftState::MissingButExpected => "missing-but-expected",
            };
            Some(MetaSkillDrift {
                skill_id: c.skill_id.to_owned(),
                harness: c.harness.to_owned(),
                scope: c.scope.to_owned(),
                dir: c.dir.display().to_string(),
                state: state.to_owned(),
            })
        })
        .collect();

    // Deterministic order for the `--json` wire-shape pins (contract §Read-only
    // "BTreeMap-ordered"). The tuple sort is total over the three string keys.
    rows.sort_by(|a, b| {
        (&a.skill_id, &a.harness, &a.scope).cmp(&(&b.skill_id, &b.harness, &b.scope))
    });
    rows
}

/// `--fix` repair (FR-032): re-run the idempotent [`meta::install_skill`] for
/// every `stale` / `missing-but-expected` row by re-deriving the candidate set
/// (the SAME safe, atomic, symlink-checked path — NOT a bespoke writer).
///
/// Forward-progress: a per-location failure is recorded and the loop continues;
/// the first error is returned for the caller's exit-code precedence (mirrors
/// the `reconcile_<sink>` template). The caller re-projects [`check`] afterward
/// (gated on "the repair ran"), so the post-repair report reflects on-disk
/// state.
///
/// Returns `Ok(count_installed)` when every targeted location was (re)installed,
/// or `Err(first_error)` after attempting them all.
pub fn repair(home: &Path, scope: &ResolvedScope) -> Result<usize, TomeError> {
    let mut installed = 0usize;
    let mut first_error: Option<TomeError> = None;

    for c in candidates(home, scope) {
        // Only repair the two drift classes; an up-to-date install is left
        // untouched (idempotent install would replace in place, but skipping
        // it avoids needless writes and keeps mtime stable for healthy rows).
        match meta::drift_probe(c.skill_id, &c.dir) {
            DriftState::UpToDate => continue,
            DriftState::Stale { .. } | DriftState::MissingButExpected => {}
        }
        match meta::install_skill(c.skill_id, &c.dir) {
            Ok(_) => installed += 1,
            Err(e) => {
                tracing::warn!(
                    skill = c.skill_id,
                    harness = c.harness,
                    scope = c.scope,
                    error = %e,
                    "doctor --fix: meta-skill (re)install failed; continuing",
                );
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
    }

    match first_error {
        Some(e) => Err(e),
        None => Ok(installed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{Scope, ScopeSource, WorkspaceName};
    use tempfile::TempDir;

    fn global_scope() -> ResolvedScope {
        ResolvedScope {
            scope: Scope(WorkspaceName::global()),
            source: ScopeSource::GlobalFallback,
            project_root: None,
        }
    }

    fn project_scope(project_root: &Path) -> ResolvedScope {
        ResolvedScope {
            scope: Scope(WorkspaceName::global()),
            source: ScopeSource::ProjectMarker,
            project_root: Some(project_root.to_path_buf()),
        }
    }

    #[test]
    fn global_undetected_harness_emits_nothing() {
        // Empty HOME → no harness detected → no candidates → empty projection.
        let home = TempDir::new().unwrap();
        let rows = check(home.path(), &global_scope());
        assert!(rows.is_empty(), "no detected harness ⇒ no rows: {rows:?}");
    }

    #[test]
    fn global_detected_harness_with_no_install_is_missing_but_expected() {
        let home = TempDir::new().unwrap();
        // Make claude-code "detected" — its dot-dir exists (existence-only).
        std::fs::create_dir_all(home.path().join(".claude")).unwrap();

        let rows = check(home.path(), &global_scope());
        let cc: Vec<_> = rows.iter().filter(|r| r.harness == "claude-code").collect();
        assert!(!cc.is_empty(), "detected claude-code must produce rows");
        assert!(
            cc.iter().all(|r| r.state == "missing-but-expected"),
            "no install yet ⇒ missing-but-expected: {cc:?}",
        );
        assert!(cc.iter().all(|r| r.scope == "global"));
    }

    #[test]
    fn global_up_to_date_install_is_omitted() {
        let home = TempDir::new().unwrap();
        std::fs::create_dir_all(home.path().join(".claude")).unwrap();
        let skill_dir = home.path().join(".claude/skills");
        std::fs::create_dir_all(&skill_dir).unwrap();
        meta::install_skill("convert-marketplace", &skill_dir).unwrap();

        let rows = check(home.path(), &global_scope());
        // The freshly-installed claude-code/global row is up-to-date → omitted.
        assert!(
            !rows
                .iter()
                .any(|r| r.harness == "claude-code" && r.scope == "global"),
            "up-to-date claude-code/global row must be omitted: {rows:?}",
        );
    }

    #[test]
    fn rows_match_hand_written_sort_order() {
        let home = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        // Detect claude-code + cursor at GLOBAL scope; the surveyed project root
        // adds PROJECT-scope rows for both. With one embedded skill, the
        // hand-written `(skill_id, harness, scope)` order is fully determined:
        // claude-code before cursor, and `global` before `project` per harness.
        std::fs::create_dir_all(home.path().join(".claude")).unwrap();
        std::fs::create_dir_all(home.path().join(".cursor")).unwrap();

        let rows = check(home.path(), &project_scope(project.path()));
        let observed: Vec<(&str, &str, &str)> = rows
            .iter()
            .filter(|r| r.harness == "claude-code" || r.harness == "cursor")
            .map(|r| (r.skill_id.as_str(), r.harness.as_str(), r.scope.as_str()))
            .collect();

        // A LITERAL expected sequence — catches a comparator change, not just a
        // missing sort call (a self-sort would pass any comparator).
        assert_eq!(
            observed,
            [
                ("convert-marketplace", "claude-code", "global"),
                ("convert-marketplace", "claude-code", "project"),
                ("convert-marketplace", "cursor", "global"),
                ("convert-marketplace", "cursor", "project"),
            ],
        );
    }

    #[test]
    fn project_scope_emits_per_detected_skill_capable_harness() {
        let home = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        // FIX A: the project branch is now detect-gated like the installer — the
        // project root is necessary but NOT sufficient; the harness must ALSO be
        // detected under `home`. Detect claude-code so its project row appears.
        std::fs::create_dir_all(home.path().join(".claude")).unwrap();

        // No on-disk install → every detected project candidate is
        // missing-but-expected.
        let rows = check(home.path(), &project_scope(project.path()));
        let project_rows: Vec<_> = rows.iter().filter(|r| r.scope == "project").collect();
        assert!(
            project_rows.iter().any(|r| r.harness == "claude-code"),
            "detected claude-code (skill-capable) must appear in project rows: {rows:?}",
        );
        assert!(
            !rows.iter().any(|r| r.harness == "gemini"),
            "gemini does not support native skills ⇒ no rows",
        );
    }

    #[test]
    fn project_scope_undetected_harness_emits_no_project_row() {
        // FIX A: a surveyed project root with NO detected harness under `home`
        // produces NO rows — doctor must not write into an undetected harness's
        // project dir (the divergence FR-031a closes).
        let home = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        let rows = check(home.path(), &project_scope(project.path()));
        assert!(
            rows.is_empty(),
            "an undetected harness yields no project candidates: {rows:?}",
        );
    }

    #[test]
    fn repair_installs_missing_then_reprobe_is_up_to_date() {
        let home = TempDir::new().unwrap();
        std::fs::create_dir_all(home.path().join(".claude")).unwrap();

        // Pre-repair: claude-code/global is missing-but-expected.
        let before = check(home.path(), &global_scope());
        assert!(
            before
                .iter()
                .any(|r| r.harness == "claude-code" && r.state == "missing-but-expected"),
        );

        let installed = repair(home.path(), &global_scope()).expect("repair ok");
        assert!(
            installed >= 1,
            "at least the claude-code/global install ran"
        );

        // Post-repair: claude-code/global drops out of the drift projection.
        let after = check(home.path(), &global_scope());
        assert!(
            !after
                .iter()
                .any(|r| r.harness == "claude-code" && r.scope == "global"),
            "claude-code/global must be up-to-date (omitted) after repair: {after:?}",
        );
    }
}
