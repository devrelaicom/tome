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
//! - Candidate locations are re-derived from the supported-harness set (installs
//!   are not indexed — NFR-003): for every embedded skill, every skill-capable
//!   harness, at the requested scope.
//! - GLOBAL: a row is emitted only for a **detected** harness (its dot-dir is
//!   present via the existence-only [`crate::doctor::harness_detect::probe`]) —
//!   the expectation source for `missing-but-expected` is a detected harness,
//!   never a guess (FR-031a).
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

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::authoring::meta::{self, DriftState};
use crate::doctor::harness_detect;
use crate::doctor::report::MetaSkillDrift;
use crate::error::TomeError;
use crate::harness::{HarnessModule, SUPPORTED_HARNESSES};
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
/// PROJECT candidates are produced only when `scope.project_root` is `Some`;
/// the project root's existence IS the detection signal. GLOBAL candidates are
/// gated on the harness being detected under `home` (existence-only probe).
fn candidates(home: &Path, scope: &ResolvedScope) -> Vec<Candidate> {
    let mut out = Vec::new();

    // The set of harnesses detected on this machine, by name (existence-only).
    // Used as the `missing-but-expected` expectation source for GLOBAL scope.
    let detected: BTreeSet<String> = harness_detect::probe(home)
        .into_iter()
        .filter(|p| p.present)
        .map(|p| p.name)
        .collect();

    for skill in meta::all() {
        for module in SUPPORTED_HARNESSES {
            if !module.supports_native_skills() {
                continue;
            }
            let harness = module.name();

            // PROJECT scope: the project root is the detection signal.
            if let Some(project_root) = scope.project_root.as_deref()
                && let Some(dir) = project_skill_dir(*module, project_root)
            {
                out.push(Candidate {
                    skill_id: skill.id,
                    harness,
                    scope: SCOPE_PROJECT,
                    dir,
                });
            }

            // GLOBAL scope: emit only for a DETECTED harness (FR-031a).
            if detected.contains(harness)
                && let Some(dir) = global_skill_dir(*module, home)
            {
                out.push(Candidate {
                    skill_id: skill.id,
                    harness,
                    scope: SCOPE_GLOBAL,
                    dir,
                });
            }
        }
    }

    out
}

/// Project skills root for a skill-capable harness (`None` if it has none).
fn project_skill_dir(module: &dyn HarnessModule, project_root: &Path) -> Option<PathBuf> {
    module.skill_dir(project_root)
}

/// Global/user skills root for a skill-capable harness (`None` if it has none).
fn global_skill_dir(module: &dyn HarnessModule, home: &Path) -> Option<PathBuf> {
    module.skill_dir_global(home)
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
    fn rows_are_sorted_by_skill_harness_scope() {
        let home = TempDir::new().unwrap();
        // Detect every skill-capable harness so we get multiple rows to order.
        for d in [".claude", ".cursor", ".codex", ".opencode"] {
            std::fs::create_dir_all(home.path().join(d)).unwrap();
        }
        let rows = check(home.path(), &global_scope());
        let mut sorted = rows.clone();
        sorted.sort_by(|a, b| {
            (&a.skill_id, &a.harness, &a.scope).cmp(&(&b.skill_id, &b.harness, &b.scope))
        });
        assert_eq!(rows, sorted, "rows must be deterministically sorted");
    }

    #[test]
    fn project_scope_emits_per_skill_capable_harness() {
        let home = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        // No on-disk install → every project candidate is missing-but-expected.
        let rows = check(home.path(), &project_scope(project.path()));
        assert!(rows.iter().all(|r| r.scope == "project"));
        assert!(
            rows.iter().any(|r| r.harness == "claude-code"),
            "claude-code (skill-capable) must appear in project rows: {rows:?}",
        );
        assert!(
            !rows.iter().any(|r| r.harness == "gemini"),
            "gemini does not support native skills ⇒ no rows",
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
