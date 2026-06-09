//! Phase 9 / US4 — `tome doctor` meta-skill DRIFT surface (integration).
//!
//! Drives the PUBLIC library API (`doctor::assemble_report` + the
//! `doctor::meta_drift` repair) end-to-end against on-disk installs under an
//! isolated temp `home`, mirroring the `doctor_p6` style (StubEmbedder-free —
//! the meta-skill surface is independent of model health). Covers:
//!
//! 1. install-then-corrupt-then-fix round trip (up-to-date → stale → repaired),
//! 2. read-only default mutates nothing (mtime-stable, FR-124),
//! 3. a malformed on-disk SKILL.md classifies `stale` (never a panic, FR-031b),
//! 4. a detected skill-capable harness with NO install → `missing-but-expected`.

mod common;

use std::path::{Path, PathBuf};

use tempfile::TempDir;
use tome::authoring::meta;
use tome::doctor;
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

use crate::common::{fabricate_models, lifecycle_paths};

const SKILL: &str = "convert-marketplace";

/// A global-fallback scope (no project root). The meta-skill global candidates
/// are gated on harness DETECTION under `home`.
fn global_scope() -> ResolvedScope {
    ResolvedScope {
        scope: Scope(WorkspaceName::global()),
        source: ScopeSource::GlobalFallback,
        project_root: None,
    }
}

/// Build an isolated `(tome_root_paths, home_dir)` pair with all registry
/// models fabricated so `assemble_report` classifies a healthy-enough report.
/// `home` is a SEPARATE temp dir under which `~/.claude` etc. are probed.
fn setup() -> (TempDir, TempDir, tome::paths::Paths) {
    let root = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let paths = lifecycle_paths(root.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);
    (root, home, paths)
}

/// Make claude-code "detected" (existence-only) and return its global skills
/// root `<home>/.claude/skills`.
fn detect_claude_code(home: &Path) -> PathBuf {
    let skills = home.join(".claude/skills");
    std::fs::create_dir_all(&skills).unwrap();
    skills
}

/// The claude-code/global drift row from a fresh `assemble_report`, if any.
fn cc_global_row(report: &tome::doctor::DoctorReport) -> Option<&tome::doctor::MetaSkillDrift> {
    report
        .meta_skills
        .iter()
        .find(|r| r.harness == "claude-code" && r.scope == "global")
}

#[test]
fn install_corrupt_then_fix_round_trip() {
    let (_root, home, paths) = setup();
    let skills = detect_claude_code(home.path());

    // Install at the embedded revision via the shared writer.
    meta::install_skill(SKILL, &skills).expect("install");

    // up-to-date ⇒ NOT surfaced as drift (the row is absent).
    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert!(
        cc_global_row(&report).is_none(),
        "up-to-date install must be absent from the drift projection: {:?}",
        report.meta_skills,
    );

    // Corrupt the on-disk revision stamp → drift_probe reads a mismatch → stale.
    let skill_md = skills.join(SKILL).join("SKILL.md");
    let original = std::fs::read_to_string(&skill_md).unwrap();
    let bogus = original.replace(
        &meta::find(SKILL).unwrap().revision.to_string(),
        "deadbeefdeadbeef",
    );
    assert_ne!(bogus, original, "the corruption must change the stamp");
    std::fs::write(&skill_md, &bogus).unwrap();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    let row = cc_global_row(&report).expect("stale row present after corruption");
    assert_eq!(row.state, "stale");
    assert_eq!(row.skill_id, SKILL);

    // Repair via the shared idempotent install path, then re-probe.
    let installed = doctor::meta_drift::repair(home.path(), &global_scope()).expect("repair");
    assert!(installed >= 1, "claude-code/global re-install ran");

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert!(
        cc_global_row(&report).is_none(),
        "after --fix the claude-code/global row is up-to-date (absent): {:?}",
        report.meta_skills,
    );
    // And the on-disk stamp is back to the embedded revision.
    let after = std::fs::read_to_string(&skill_md).unwrap();
    assert!(after.contains(&meta::find(SKILL).unwrap().revision.to_string()));
}

#[test]
fn read_only_default_mutates_nothing() {
    let (_root, home, paths) = setup();
    let skills = detect_claude_code(home.path());
    meta::install_skill(SKILL, &skills).expect("install");

    let skill_md = skills.join(SKILL).join("SKILL.md");
    let before = std::fs::metadata(&skill_md).unwrap().modified().unwrap();

    // A read-only assemble (no fix) must not touch the installed file.
    let _ = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();

    let after = std::fs::metadata(&skill_md).unwrap().modified().unwrap();
    assert_eq!(before, after, "read-only doctor must not rewrite SKILL.md");
}

#[test]
fn malformed_on_disk_file_classifies_stale_not_panic() {
    let (_root, home, paths) = setup();
    let skills = detect_claude_code(home.path());

    // Plant a malformed (truncated frontmatter, non-UTF-8 tail) SKILL.md under
    // the owned folder — no marker, invalid bytes → refreshable `stale`.
    let dir = skills.join(SKILL);
    std::fs::create_dir_all(&dir).unwrap();
    let mut bytes = b"---\nname: convert-marketplace\n".to_vec();
    bytes.extend_from_slice(&[0xff, 0xfe, 0x00, 0x01]); // invalid UTF-8 tail
    std::fs::write(dir.join("SKILL.md"), &bytes).unwrap();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    let row = cc_global_row(&report).expect("malformed install surfaces as a row");
    assert_eq!(
        row.state, "stale",
        "malformed/non-UTF-8 ⇒ stale (refreshable)"
    );
}

#[test]
fn detected_harness_with_no_install_is_missing_but_expected() {
    let (_root, home, paths) = setup();
    // Detect claude-code but install NOTHING.
    detect_claude_code(home.path());

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    let row = cc_global_row(&report).expect("detected harness must produce a row");
    assert_eq!(row.state, "missing-but-expected");
    assert_eq!(row.scope, "global");
}

#[test]
fn undetected_harness_emits_no_rows() {
    let (_root, home, paths) = setup();
    // Empty home → no harness detected → no global candidates at all.
    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert!(
        report.meta_skills.is_empty(),
        "no detected harness ⇒ empty drift projection (keeps wire shape stable): {:?}",
        report.meta_skills,
    );
}
