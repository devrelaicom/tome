//! Integration coverage for `lint` (US3): parse a native Tome artifact +
//! run the rule registry → verdict + exit codes. Read-only (the `--autofix`
//! application + command wrappers land in US3-2).

use std::fs;
use std::path::Path;

use tome::authoring::lint::parse::parse_artifact;
use tome::authoring::lint::{rules, run};

fn write_clean_plugin(dir: &Path) {
    fs::write(
        dir.join("tome-plugin.toml"),
        "name = \"my-plugin\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    fs::create_dir_all(dir.join("skills/foo")).unwrap();
    fs::write(
        dir.join("skills/foo/SKILL.md"),
        "---\nname: foo\ndescription: a skill\n---\nbody\n",
    )
    .unwrap();
}

#[test]
fn clean_plugin_lints_with_no_findings() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    fs::create_dir(&dir).unwrap();
    write_clean_plugin(&dir);

    let artifact = parse_artifact(&dir).unwrap();
    let report = run(&artifact, &rules::all());
    assert_eq!(report.errors, 0, "{:?}", report.diagnostics);
    assert_eq!(report.warnings, 0, "{:?}", report.diagnostics);
    assert!(report.into_result(false).is_ok());
}

#[test]
fn reports_every_finding_in_one_run_and_errors_exit_85() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    fs::create_dir(&dir).unwrap();
    // Invalid version + a skill whose name != dir + no description + a residual
    // harness-ism + an unsupported component dir.
    fs::write(
        dir.join("tome-plugin.toml"),
        "name = \"p\"\nversion = \"notsemver\"\n",
    )
    .unwrap();
    fs::create_dir_all(dir.join("skills/realdir")).unwrap();
    fs::write(
        dir.join("skills/realdir/SKILL.md"),
        "---\nname: wrong\n---\nUse ${CLAUDE_PLUGIN_ROOT}/x\n",
    )
    .unwrap();
    fs::create_dir(dir.join("monitors")).unwrap();

    let artifact = parse_artifact(&dir).unwrap();
    let report = run(&artifact, &rules::all());
    // version-invalid + name-not-dir.
    assert!(report.errors >= 2, "{:?}", report.diagnostics);
    // description-missing + residual-harness-ism + unsupported-component.
    assert!(report.warnings >= 3, "{:?}", report.diagnostics);
    assert!(matches!(report.into_result(false), Err(e) if e.exit_code() == 85));
}

#[test]
fn warnings_only_pass_without_strict_and_fail_86_with_strict() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("tome-plugin.toml"),
        "name = \"p\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    // A skill missing only a description → one warning, no errors.
    fs::create_dir_all(dir.join("skills/foo")).unwrap();
    fs::write(
        dir.join("skills/foo/SKILL.md"),
        "---\nname: foo\n---\nbody\n",
    )
    .unwrap();

    let artifact = parse_artifact(&dir).unwrap();
    let report = run(&artifact, &rules::all());
    assert_eq!(report.errors, 0, "{:?}", report.diagnostics);
    assert!(report.warnings >= 1);
    assert!(
        report.into_result(false).is_ok(),
        "warnings pass without --strict"
    );
    assert!(matches!(report.into_result(true), Err(e) if e.exit_code() == 86));
}

#[test]
fn a_non_tome_directory_is_a_usage_error() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("notome");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("README.md"), b"hi").unwrap();
    let err = parse_artifact(&dir).unwrap_err();
    assert_eq!(err.exit_code(), 2);
}

#[test]
fn catalog_lints_its_vendored_plugins_and_name_mismatch() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("cat");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("tome-catalog.toml"),
        "name = \"c\"\nversion = \"1.0.0\"\ndescription = \"d\"\n\n[owner]\nname = \"o\"\nemail = \"o@x.io\"\n\n[[plugins]]\nname = \"declared\"\nsource = \"alpha\"\n",
    )
    .unwrap();
    // The vendored plugin's own name differs from the catalog's declaration.
    fs::create_dir(dir.join("alpha")).unwrap();
    fs::write(
        dir.join("alpha/tome-plugin.toml"),
        "name = \"alpha\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();

    let artifact = parse_artifact(&dir).unwrap();
    let report = run(&artifact, &rules::all());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.rule_id == "lint/catalog-name-mismatch"),
        "{:?}",
        report.diagnostics
    );
}
