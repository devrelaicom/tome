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
//! 4. a detected skill-capable harness with NO install → NOT drift (option A),
//! 5. a positive `--json` wire-shape pin for a stale row (all five fields incl. `dir`),
//! 6. repair forward-progress: one symlink-refused harness with a stale install,
//!    the other still lands (first_error surfaced, no escape),
//! 7. a hand-written sort order over ≥2 detected harnesses + both scopes,
//! 8. a project-scope install → stale → `--fix` → up-to-date round trip.

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

/// A project-marker scope carrying `project_root` — the only shape under which
/// the doctor surveys PROJECT-scope candidates (it never invents a root).
fn project_scope(project_root: &Path) -> ResolvedScope {
    ResolvedScope {
        scope: Scope(WorkspaceName::global()),
        source: ScopeSource::ProjectMarker,
        project_root: Some(project_root.to_path_buf()),
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
fn detected_harness_with_no_install_is_not_drift() {
    // Option A (smoke-test regression): a detected harness with NO install is
    // simply "not installed" — `tome meta list` is that surface, not doctor.
    let (_root, home, paths) = setup();
    // Detect claude-code but install NOTHING.
    detect_claude_code(home.path());

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert!(
        cc_global_row(&report).is_none(),
        "missing is never drift — detected harness with no install must not produce a drift row: {:?}",
        report.meta_skills,
    );
    assert!(
        report.meta_skills.is_empty(),
        "no stale install ⇒ empty drift projection: {:?}",
        report.meta_skills,
    );
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

/// Make cursor "detected" (existence-only, default `.cursor` dot-dir) and return
/// its global skills root `<home>/.cursor/skills`.
fn detect_cursor(home: &Path) -> PathBuf {
    let skills = home.join(".cursor/skills");
    std::fs::create_dir_all(&skills).unwrap();
    skills
}

/// A POPULATED `meta_skills` row freezes the `--json` wire shape — exactly the
/// five keys, the expected values, and the `.../skills` `dir` suffix for the
/// (harness, scope) pair. Uses a stale (present-but-unstamped) install so the
/// row appears under the new stale-only emit policy (option A: missing is not
/// drift).
#[test]
fn populated_row_json_wire_shape_is_pinned() {
    let (_root, home, paths) = setup();
    let skills = detect_claude_code(home.path());

    // Plant a stale (present-but-unstamped) SKILL.md — drift_probe reads Stale.
    std::fs::create_dir_all(skills.join(SKILL)).unwrap();
    std::fs::write(
        skills.join(SKILL).join("SKILL.md"),
        "---\nname: convert-marketplace\n---\nold body\n",
    )
    .unwrap();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    let row = cc_global_row(&report).expect("stale install must produce a row");

    let value = serde_json::to_value(row).expect("row serialises");
    let obj = value.as_object().expect("row is a JSON object");

    // EXACTLY the five documented keys — no more, no fewer.
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["dir", "harness", "scope", "skill_id", "state"],
        "the wire shape is exactly the five keys: {value}",
    );

    assert_eq!(obj["skill_id"], serde_json::json!("convert-marketplace"));
    assert_eq!(obj["harness"], serde_json::json!("claude-code"));
    assert_eq!(obj["scope"], serde_json::json!("global"));
    assert_eq!(obj["state"], serde_json::json!("stale"));

    // `dir` is the claude-code/global skills root — ends with `.../.claude/skills`.
    let dir = obj["dir"].as_str().expect("dir is a string");
    let expected_suffix = Path::new(".claude").join("skills");
    assert!(
        dir.ends_with(expected_suffix.to_str().unwrap()),
        "dir must end with the claude-code/global skills suffix: {dir}",
    );
}

/// Repair forward-progress. TWO detected skill-capable harnesses both have
/// STALE installs; one harness's skills root traverses a SYMLINK so its
/// `install_skill` is refused (MetaInstallFailed/88) while the other refreshes
/// cleanly. `repair` surfaces the first error AND lands the healthy one (one
/// failure never aborts the rest), and the symlinked one does not escape.
/// Under option A (missing-is-not-drift) both harnesses must have a stale
/// on-disk copy to be drift candidates in the first place.
#[cfg(unix)]
#[test]
fn repair_forward_progress_one_symlink_refused_other_lands() {
    use std::os::unix::fs::symlink;

    let (_root, home_dir, _paths) = setup();
    // Canonicalise so the symlink-guard compares stable absolute components.
    let home = home_dir.path().canonicalize().unwrap();

    // cursor is "detected" + healthy → plant a stale install so it is a drift
    // candidate (missing-is-not-drift; cursor must have a copy to be repaired).
    let cursor_skills = detect_cursor(&home);
    std::fs::create_dir_all(cursor_skills.join(SKILL)).unwrap();
    std::fs::write(
        cursor_skills.join(SKILL).join("SKILL.md"),
        "---\nname: convert-marketplace\n---\nold body\n",
    )
    .unwrap();

    // claude-code is "detected" but its `.claude` dot-dir is a SYMLINK to an
    // out-of-tree dir, so the global skills root traverses a symlinked
    // component → probe reads Stale (symlink-refused read degrades), and
    // the repair write is also refused — no escape.
    let outside = TempDir::new().unwrap();
    symlink(outside.path(), home.join(".claude")).unwrap();

    let scope = global_scope();

    // Pre-repair: both harnesses are stale drift candidates.
    // claude-code is stale (symlink-refused read degrades to Stale).
    // cursor is stale (unstamped SKILL.md).
    let before = doctor::meta_drift::check(&home, &scope);
    assert!(
        before
            .iter()
            .any(|r| r.harness == "claude-code" && r.state == "stale"),
        "claude-code/global is a (symlink-refused, refreshable-stale) candidate: {before:?}",
    );
    assert!(
        before
            .iter()
            .any(|r| r.harness == "cursor" && r.state == "stale"),
        "cursor/global is a stale drift candidate: {before:?}",
    );

    // Repair: the claude-code symlink is refused (88), cursor still refreshes.
    let err = doctor::meta_drift::repair(&home, &scope)
        .expect_err("the symlinked harness must surface a first_error");
    assert_eq!(err.exit_code(), 88, "first_error is MetaInstallFailed (88)");

    // (b) forward-progress: the HEALTHY cursor skill was refreshed on disk.
    assert!(
        home.join(".cursor/skills/convert-marketplace/SKILL.md")
            .is_file(),
        "cursor/global must refresh despite the claude-code failure",
    );

    // (c) the symlinked claude-code write did NOT escape the home tree.
    assert!(
        !outside.path().join("skills/convert-marketplace").exists(),
        "no write escaped through the .claude symlink",
    );
}

/// A HAND-WRITTEN expected order over ≥2 detected harnesses AND both scopes —
/// catches a comparator change, not just a missing `sort` call. With a single
/// embedded skill (`convert-marketplace`), the rows sort `(skill_id, harness,
/// scope)`, so claude-code precedes cursor and, within a harness, `global`
/// precedes `project`. Under option A (missing-is-not-drift) all four candidate
/// locations must carry a STALE install to appear as rows.
#[test]
fn rows_match_hand_written_sort_order() {
    let (_root, home, paths) = setup();
    let project = TempDir::new().unwrap();
    // Detect both claude-code and cursor at GLOBAL scope, plus a surveyed
    // project root → PROJECT-scope candidates for the same two harnesses.
    detect_claude_code(home.path());
    detect_cursor(home.path());

    // Plant a stale (unstamped) SKILL.md at all four candidate locations so
    // they surface as drift rows under the stale-only emit policy.
    let stale = "---\nname: convert-marketplace\n---\nold body\n";
    // Global dirs.
    let cc_global = home.path().join(".claude/skills");
    let cursor_global = home.path().join(".cursor/skills");
    for dir in [&cc_global, &cursor_global] {
        std::fs::create_dir_all(dir.join(SKILL)).unwrap();
        std::fs::write(dir.join(SKILL).join("SKILL.md"), stale).unwrap();
    }
    // Project dirs (claude-code and cursor under the project root).
    let cc_project = project.path().join(".claude/skills");
    let cursor_project = project.path().join(".cursor/skills");
    for dir in [&cc_project, &cursor_project] {
        std::fs::create_dir_all(dir.join(SKILL)).unwrap();
        std::fs::write(dir.join(SKILL).join("SKILL.md"), stale).unwrap();
    }

    let report =
        doctor::assemble_report(&project_scope(project.path()), &paths, home.path(), false)
            .unwrap();

    // Restrict to the two harnesses under test, in the row order as emitted.
    let observed: Vec<(&str, &str, &str)> = report
        .meta_skills
        .iter()
        .filter(|r| r.harness == "claude-code" || r.harness == "cursor")
        .map(|r| (r.skill_id.as_str(), r.harness.as_str(), r.scope.as_str()))
        .collect();

    let expected = [
        ("convert-marketplace", "claude-code", "global"),
        ("convert-marketplace", "claude-code", "project"),
        ("convert-marketplace", "cursor", "global"),
        ("convert-marketplace", "cursor", "project"),
    ];
    assert_eq!(
        observed, expected,
        "rows must match the hand-written (skill_id, harness, scope) order",
    );
}

/// FIX 6 / Test Minor #9: a PROJECT-scope on-disk round trip mirroring the
/// global one — install into the project skills dir, corrupt the stamp →
/// `check` reports `stale` for the project row → `repair` → re-probe up-to-date.
#[test]
fn project_scope_install_corrupt_fix_round_trip() {
    let (_root, home, paths) = setup();
    let project = TempDir::new().unwrap();
    let scope = project_scope(project.path());

    // FIX A: the doctor's PROJECT branch is now detect-gated like the installer —
    // the harness must be detected under `home` for its project row to surface.
    // Detect claude-code under `home` so `<project>/.claude/skills` is surveyed.
    detect_claude_code(home.path());

    // claude-code's PROJECT skills root is `<project>/.claude/skills`.
    let skills = project.path().join(".claude/skills");
    std::fs::create_dir_all(&skills).unwrap();
    meta::install_skill(SKILL, &skills).expect("install into project scope");

    // up-to-date ⇒ no claude-code/project drift row.
    let report = doctor::assemble_report(&scope, &paths, home.path(), false).unwrap();
    assert!(
        !report
            .meta_skills
            .iter()
            .any(|r| r.harness == "claude-code" && r.scope == "project"),
        "fresh project install is up-to-date (absent): {:?}",
        report.meta_skills,
    );

    // Corrupt the stamped revision → stale.
    let skill_md = skills.join(SKILL).join("SKILL.md");
    let original = std::fs::read_to_string(&skill_md).unwrap();
    let bogus = original.replace(
        &meta::find(SKILL).unwrap().revision.to_string(),
        "deadbeefdeadbeef",
    );
    assert_ne!(bogus, original, "the corruption must change the stamp");
    std::fs::write(&skill_md, &bogus).unwrap();

    let report = doctor::assemble_report(&scope, &paths, home.path(), false).unwrap();
    let row = report
        .meta_skills
        .iter()
        .find(|r| r.harness == "claude-code" && r.scope == "project")
        .expect("stale project row present after corruption");
    assert_eq!(row.state, "stale");
    assert_eq!(row.skill_id, SKILL);

    // Repair, then re-probe up-to-date (absent).
    let installed = doctor::meta_drift::repair(home.path(), &scope).expect("project repair");
    assert!(installed >= 1, "claude-code/project re-install ran");

    let report = doctor::assemble_report(&scope, &paths, home.path(), false).unwrap();
    assert!(
        !report
            .meta_skills
            .iter()
            .any(|r| r.harness == "claude-code" && r.scope == "project"),
        "after --fix the project row is up-to-date (absent): {:?}",
        report.meta_skills,
    );
    let after = std::fs::read_to_string(&skill_md).unwrap();
    assert!(after.contains(&meta::find(SKILL).unwrap().revision.to_string()));
}

/// FIX A (Rust MAJOR, FR-031a): the doctor PROJECT branch is now detect-gated
/// EXACTLY like the installer. A surveyed project root with NO detected harness
/// under `home` must yield NO project drift row — doctor must never write into
/// an undetected harness's project dir (broader than `meta add` would). This is
/// the SSOT-divergence the shared enumeration helper closes.
#[test]
fn project_scope_undetected_harness_emits_no_project_row() {
    let (_root, home, paths) = setup();
    // `home` is empty → claude-code (and every harness) is UNDETECTED, even
    // though a project root is surveyed.
    let project = TempDir::new().unwrap();

    let report =
        doctor::assemble_report(&project_scope(project.path()), &paths, home.path(), false)
            .unwrap();
    assert!(
        !report.meta_skills.iter().any(|r| r.scope == "project"),
        "an undetected harness must not produce a project drift row: {:?}",
        report.meta_skills,
    );
    // And with nothing detected at all, the projection is entirely empty.
    assert!(
        report.meta_skills.is_empty(),
        "no detected harness ⇒ empty drift projection: {:?}",
        report.meta_skills,
    );
}
