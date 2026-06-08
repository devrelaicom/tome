//! `lint --autofix` — apply the autofixable findings to a native Tome artifact.
//!
//! Each autofixable [`Fix`](crate::authoring::ir::Fix) is a whole-file
//! replacement computed from the file's *current* content, so two fixes that
//! target the same file (e.g. `name == dir` + a harness-ism) would conflict if
//! applied in one pass. Instead this runs a **fixpoint loop**: parse → lint →
//! apply *one* fix per file (atomic-replace) → re-parse → repeat until no
//! autofixable finding remains (or a pass-cap is hit). Re-linting after each
//! pass recomputes the next fix from the updated file, so the fixes compose.
//!
//! Within a pass, writes use `first_error` forward-progress: a failed write is
//! recorded as an error finding and the remaining files are still fixed; the
//! failure surfaces in the final report (non-zero exit), never halting (FR-021).
//!
//! `--dry-run` reports the would-be fixes of the first pass and writes nothing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::parse::parse_artifact;
use super::rules;
use super::{LintReport, run};
use crate::authoring::ir::{Diagnostic, Fix};
use crate::catalog::store::write_atomic;
use crate::error::TomeError;

/// Pass-cap guarding against a pathological non-converging rule set.
const MAX_PASSES: usize = 32;

const AUTOFIX_WRITE_FAILED: &str = "lint/autofix-write-failed";

/// Outcome of an `--autofix` run.
pub struct AutofixOutcome {
    /// Number of fixes applied (would-be, under `--dry-run`).
    pub fixed: usize,
    /// The final report: residual findings after fixing, plus any write errors.
    pub report: LintReport,
    pub dry_run: bool,
}

/// Apply autofixable findings to the artifact at `source` to a fixpoint.
pub fn autofix(source: &Path, dry_run: bool) -> Result<AutofixOutcome, TomeError> {
    let mut fixed = 0usize;
    let mut write_errors: Vec<Diagnostic> = Vec::new();

    for _ in 0..MAX_PASSES {
        let artifact = parse_artifact(source)?;
        let report = run(&artifact, &rules::all());
        let fixes = first_fix_per_path(&report);

        if fixes.is_empty() {
            return Ok(finish(fixed, report, write_errors, dry_run));
        }
        if dry_run {
            // Report the would-be fixes of this pass; change nothing on disk.
            return Ok(finish(fixes.len(), report, write_errors, dry_run));
        }

        let mut applied_this_pass = 0;
        for fix in fixes {
            match write_atomic(&fix.path, fix.replacement.as_bytes()) {
                Ok(()) => {
                    fixed += 1;
                    applied_this_pass += 1;
                }
                Err(e) => write_errors.push(Diagnostic::error(
                    AUTOFIX_WRITE_FAILED,
                    format!("could not apply autofix to {}: {e}", fix.path.display()),
                )),
            }
        }
        // No forward progress this pass (every pending write failed) — stop
        // rather than re-attempting (and re-erroring) the same fixes.
        if applied_this_pass == 0 {
            let report = run(&parse_artifact(source)?, &rules::all());
            return Ok(finish(fixed, report, write_errors, dry_run));
        }
    }

    // Pass-cap reached: report the current state.
    let report = run(&parse_artifact(source)?, &rules::all());
    Ok(finish(fixed, report, write_errors, dry_run))
}

/// One fix per file (the first autofixable finding per path), ordered by path
/// for deterministic application.
fn first_fix_per_path(report: &LintReport) -> Vec<Fix> {
    let mut by_path: BTreeMap<PathBuf, Fix> = BTreeMap::new();
    for d in report.autofixable() {
        if let Some(fix) = &d.autofix {
            by_path
                .entry(fix.path.clone())
                .or_insert_with(|| fix.clone());
        }
    }
    by_path.into_values().collect()
}

/// Fold any write-error diagnostics into the final report and recount.
fn finish(
    fixed: usize,
    report: LintReport,
    write_errors: Vec<Diagnostic>,
    dry_run: bool,
) -> AutofixOutcome {
    let report = if write_errors.is_empty() {
        report
    } else {
        let mut all = report.diagnostics;
        all.extend(write_errors);
        LintReport::from_diagnostics(all)
    };
    AutofixOutcome {
        fixed,
        report,
        dry_run,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn plugin_with_fixable_issues(dir: &Path) {
        fs::write(
            dir.join("tome-plugin.toml"),
            "name = \"p\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        // A skill whose frontmatter name != dir AND a rewritable harness-ism —
        // two fixes on the SAME file, which the fixpoint must compose.
        fs::create_dir_all(dir.join("skills/realdir")).unwrap();
        fs::write(
            dir.join("skills/realdir/SKILL.md"),
            "---\nname: wrong\ndescription: d\n---\nUse ${CLAUDE_PLUGIN_ROOT}/x\n",
        )
        .unwrap();
    }

    #[test]
    fn fixpoint_composes_two_fixes_on_one_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("p");
        fs::create_dir(&dir).unwrap();
        plugin_with_fixable_issues(&dir);

        let outcome = autofix(&dir, false).unwrap();
        assert!(outcome.fixed >= 2, "both fixes applied: {}", outcome.fixed);

        let skill = fs::read_to_string(dir.join("skills/realdir/SKILL.md")).unwrap();
        // BOTH fixes landed: name == dir, and the harness-ism rewritten.
        assert!(skill.contains("name: realdir"), "{skill}");
        assert!(skill.contains("${TOME_PLUGIN_DIR}/x"), "{skill}");
        assert!(!skill.contains("CLAUDE_PLUGIN_ROOT"));
        // The autofixable findings are resolved (only the manual ones remain).
        assert_eq!(outcome.report.errors, 0, "{:?}", outcome.report.diagnostics);
    }

    #[test]
    fn dry_run_reports_fixes_but_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("p");
        fs::create_dir(&dir).unwrap();
        plugin_with_fixable_issues(&dir);
        let before = fs::read_to_string(dir.join("skills/realdir/SKILL.md")).unwrap();

        let outcome = autofix(&dir, true).unwrap();
        assert!(outcome.dry_run);
        assert!(outcome.fixed >= 1, "would-be fixes reported");
        let after = fs::read_to_string(dir.join("skills/realdir/SKILL.md")).unwrap();
        assert_eq!(before, after, "--dry-run must not write");
    }

    #[test]
    fn clean_artifact_applies_no_fixes() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("p");
        fs::create_dir(&dir).unwrap();
        fs::write(
            dir.join("tome-plugin.toml"),
            "name = \"p\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        fs::create_dir_all(dir.join("skills/foo")).unwrap();
        fs::write(
            dir.join("skills/foo/SKILL.md"),
            "---\nname: foo\ndescription: d\n---\nbody\n",
        )
        .unwrap();

        let outcome = autofix(&dir, false).unwrap();
        assert_eq!(outcome.fixed, 0);
        assert_eq!(outcome.report.errors, 0);
        assert_eq!(outcome.report.warnings, 0);
    }

    #[test]
    fn autofix_resolves_fixables_and_leaves_manual_findings() {
        // Autofixable (name!=dir + harness-ism) AND a non-autofixable (missing
        // description). After autofix the fixables are gone; the manual remains.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("p");
        fs::create_dir(&dir).unwrap();
        fs::write(
            dir.join("tome-plugin.toml"),
            "name = \"p\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        fs::create_dir_all(dir.join("skills/realdir")).unwrap();
        // No `description` (manual) + name!=dir + harness-ism (both autofixable).
        fs::write(
            dir.join("skills/realdir/SKILL.md"),
            "---\nname: wrong\n---\nUse ${CLAUDE_PLUGIN_ROOT}/x\n",
        )
        .unwrap();

        let outcome = autofix(&dir, false).unwrap();
        // Every autofixable finding is resolved.
        assert_eq!(
            outcome.report.autofixable().count(),
            0,
            "{:?}",
            outcome.report.diagnostics
        );
        // The manual missing-description finding survives.
        assert!(
            outcome
                .report
                .diagnostics
                .iter()
                .any(|d| d.rule_id == "lint/description-missing"),
            "{:?}",
            outcome.report.diagnostics
        );
    }

    #[cfg(unix)]
    #[test]
    fn write_failure_surfaces_a_finding_and_does_not_hang() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("p");
        fs::create_dir(&dir).unwrap();
        fs::write(
            dir.join("tome-plugin.toml"),
            "name = \"p\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        let skills = dir.join("skills/realdir");
        fs::create_dir_all(&skills).unwrap();
        fs::write(
            skills.join("SKILL.md"),
            "---\nname: wrong\ndescription: d\n---\nbody\n",
        )
        .unwrap();
        // Make the skill dir read-only so the atomic temp-file write fails.
        fs::set_permissions(&skills, fs::Permissions::from_mode(0o555)).unwrap();

        let outcome = autofix(&dir, false).unwrap(); // forward-progress: returns Ok
        // Restore perms so the tempdir can be cleaned up.
        fs::set_permissions(&skills, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(
            outcome
                .report
                .diagnostics
                .iter()
                .any(|d| d.rule_id == "lint/autofix-write-failed"),
            "a write failure must surface as a finding: {:?}",
            outcome.report.diagnostics
        );
    }
}
