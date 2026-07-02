//! End-to-end `create` tests (US4): scaffold each artifact level → emit to disk
//! → parse back → lint, asserting the contract's "a freshly-created artifact
//! MUST pass lint with zero findings" invariant, plus the skill naming rules
//! (`<plugin>:<name>`), `--bare`, `name == dir`, and symlink refusal.

use std::fs;
use std::path::Path;

use tome::authoring::detect::ArtifactLevel;
use tome::authoring::emit::{EmitOptions, emit};
use tome::authoring::lint::parse::parse_artifact;
use tome::authoring::lint::{rules, run};
use tome::authoring::scaffold::{CreateParams, create_artifact};

fn params(name: &str) -> CreateParams {
    CreateParams {
        name: name.to_owned(),
        plugin_name: None,
        description: None,
        author_name: None,
        date: "2026-06-08".to_owned(),
        bare: false,
    }
}

/// Scaffold `level` from `params`, emit under `<tmp>/<dir>`, and return the
/// landed root for inspection + linting.
fn scaffold_to_disk(tmp: &Path, level: ArtifactLevel, p: &CreateParams) -> std::path::PathBuf {
    let (artifact, name) = create_artifact(level, p).expect("scaffold");
    let target = tmp.join(&name);
    emit(&artifact, &target, EmitOptions::default()).expect("emit");
    target
}

/// Parse the artifact at `root` and assert it lints with zero findings.
fn assert_lints_clean(root: &Path) {
    let artifact = parse_artifact(root).expect("parse scaffolded artifact");
    let report = run(&artifact, &rules::all());
    assert_eq!(report.errors, 0, "errors: {:?}", report.diagnostics);
    assert_eq!(report.warnings, 0, "warnings: {:?}", report.diagnostics);
    assert_eq!(report.infos, 0, "infos: {:?}", report.diagnostics);
}

#[test]
fn catalog_scaffold_lints_clean() {
    let tmp = tempfile::tempdir().unwrap();
    let root = scaffold_to_disk(tmp.path(), ArtifactLevel::Catalog, &params("my-catalog"));
    assert!(root.join("tome-catalog.toml").is_file());
    assert_lints_clean(&root);
}

#[test]
fn plugin_scaffold_lints_clean() {
    let tmp = tempfile::tempdir().unwrap();
    let root = scaffold_to_disk(tmp.path(), ArtifactLevel::Plugin, &params("toolkit"));
    assert!(root.join("tome-plugin.toml").is_file());
    assert!(root.join("skills/toolkit/SKILL.md").is_file());
    assert_lints_clean(&root);
}

#[test]
fn plugin_scaffold_is_readable_by_the_strict_cutover_reader() {
    // T-MAJOR-3 (phase-wide): "lints clean" goes through the LENIENT parser;
    // this proves a scaffolded plugin also satisfies the STRICT cutover reader
    // (read_plugin_manifest, deny_unknown_fields) — i.e. `tome plugin enable`
    // would accept it (connects US4 create to US1 cutover, as convert already
    // does).
    let tmp = tempfile::tempdir().unwrap();
    let root = scaffold_to_disk(tmp.path(), ArtifactLevel::Plugin, &params("toolkit"));
    let manifest = tome::plugin::manifest::read_plugin_manifest(&root).unwrap();
    assert_eq!(manifest.name, "toolkit");
    assert_eq!(manifest.version, "0.1.0");

    // Same for the default plugin-wrapped skill scaffold.
    let skill_root = scaffold_to_disk(tmp.path(), ArtifactLevel::Skill, &params("review"));
    let m2 = tome::plugin::manifest::read_plugin_manifest(&skill_root).unwrap();
    assert_eq!(m2.name, "review");
}

#[test]
fn default_skill_scaffold_is_plugin_wrapped_and_lints_clean() {
    // `skill create review` → plugin "review" + skills/review/SKILL.md (review:review).
    let tmp = tempfile::tempdir().unwrap();
    let root = scaffold_to_disk(tmp.path(), ArtifactLevel::Skill, &params("review"));
    assert_eq!(root.file_name().unwrap(), "review", "name == dir");
    assert!(root.join("tome-plugin.toml").is_file());
    assert!(root.join("skills/review/SKILL.md").is_file());
    assert_lints_clean(&root);
}

#[test]
fn plugin_name_gives_the_full_name_and_dir() {
    // `skill create review --plugin-name qa` → dir "qa", full name qa:review.
    let tmp = tempfile::tempdir().unwrap();
    let mut p = params("review");
    p.plugin_name = Some("qa".to_owned());
    let root = scaffold_to_disk(tmp.path(), ArtifactLevel::Skill, &p);
    assert_eq!(root.file_name().unwrap(), "qa", "dir is the plugin name");
    assert!(root.join("skills/review/SKILL.md").is_file());
    assert_lints_clean(&root);
}

#[test]
fn bare_skill_scaffold_lints_clean() {
    let tmp = tempfile::tempdir().unwrap();
    let mut p = params("review");
    p.bare = true;
    let root = scaffold_to_disk(tmp.path(), ArtifactLevel::Skill, &p);
    assert!(root.join("SKILL.md").is_file());
    assert!(!root.join("tome-plugin.toml").exists());
    assert_lints_clean(&root);
}

#[test]
fn re_emitting_into_an_existing_dir_without_force_is_output_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let root = scaffold_to_disk(tmp.path(), ArtifactLevel::Plugin, &params("toolkit"));
    // A second emit into the same dir without --force → OutputExists (81).
    let (artifact, _) = create_artifact(ArtifactLevel::Plugin, &params("toolkit")).unwrap();
    let err = emit(&artifact, &root, EmitOptions::default()).unwrap_err();
    assert_eq!(err.exit_code(), 81);
}

#[test]
fn a_non_kebab_name_is_a_usage_error() {
    let err = create_artifact(ArtifactLevel::Skill, &params("Not_Kebab")).unwrap_err();
    assert_eq!(err.exit_code(), 2);
}

#[test]
fn description_and_author_land_and_still_lint_clean() {
    // #325: a scaffold given --description + --author carries both into the
    // emitted files AND still satisfies the "lint-clean by construction"
    // invariant (a supplied author does not introduce an `owner-missing` or
    // author-email finding).
    let tmp = tempfile::tempdir().unwrap();
    let mut p = params("toolkit");
    p.description = Some("QA helpers".to_owned());
    p.author_name = Some("Acme".to_owned());
    let root = scaffold_to_disk(tmp.path(), ArtifactLevel::Plugin, &p);

    let manifest = fs::read_to_string(root.join("tome-plugin.toml")).unwrap();
    assert!(
        manifest.contains("description = \"QA helpers\""),
        "manifest description: {manifest}"
    );
    assert!(
        manifest.contains("name = \"Acme\""),
        "manifest [author]: {manifest}"
    );
    let skill = fs::read_to_string(root.join("skills/toolkit/SKILL.md")).unwrap();
    assert!(skill.contains("QA helpers"), "skill body: {skill}");

    assert_lints_clean(&root);
}

#[test]
fn blank_author_emits_no_author_table_byte_identical_to_omitting_it() {
    // #325 review Minor: `plugin create x --author ""` (and whitespace-only)
    // must emit NO `[author]` table — byte-identical to omitting the flag —
    // never a lint-tripping `name = ""`.
    let baseline_tmp = tempfile::tempdir().unwrap();
    let baseline = scaffold_to_disk(
        baseline_tmp.path(),
        ArtifactLevel::Plugin,
        &params("toolkit"),
    );
    let baseline_manifest = fs::read_to_string(baseline.join("tome-plugin.toml")).unwrap();
    assert!(
        !baseline_manifest.contains("[author]"),
        "sanity: omitted author has no [author] table"
    );

    for blank in ["", "   ", "\t"] {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = params("toolkit");
        p.author_name = Some(blank.to_owned());
        let root = scaffold_to_disk(tmp.path(), ArtifactLevel::Plugin, &p);
        let manifest = fs::read_to_string(root.join("tome-plugin.toml")).unwrap();
        assert_eq!(
            manifest, baseline_manifest,
            "blank author {blank:?} must be byte-identical to omitting --author"
        );
        assert_lints_clean(&root);
    }
}

#[test]
fn catalog_author_sets_the_owner_and_lints_clean() {
    // #325: --author on a catalog replaces the `Your Name` owner placeholder
    // and the result still lints clean.
    let tmp = tempfile::tempdir().unwrap();
    let mut p = params("my-catalog");
    p.author_name = Some("Acme".to_owned());
    let root = scaffold_to_disk(tmp.path(), ArtifactLevel::Catalog, &p);

    let manifest = fs::read_to_string(root.join("tome-catalog.toml")).unwrap();
    assert!(manifest.contains("name = \"Acme\""), "owner: {manifest}");
    assert!(!manifest.contains("Your Name"), "placeholder: {manifest}");

    assert_lints_clean(&root);
}

#[cfg(unix)]
#[test]
fn emit_refuses_a_symlinked_target_parent() {
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir().unwrap();
    // A real outside dir, and a symlink "link" inside tmp pointing at it.
    let outside = tmp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let link = tmp.path().join("link");
    symlink(&outside, &link).unwrap();

    // Emitting "through" the symlinked component must be refused.
    let (artifact, name) = create_artifact(ArtifactLevel::Plugin, &params("toolkit")).unwrap();
    let target = link.join(&name);
    let err = emit(&artifact, &target, EmitOptions::default()).unwrap_err();
    // Symlink refusal surfaces as an Io error (exit 7), and nothing landed in
    // the real outside dir.
    assert_eq!(err.exit_code(), 7, "symlinked parent must be refused");
    assert!(
        !outside.join(&name).exists(),
        "no write escaped through the symlink"
    );
}
