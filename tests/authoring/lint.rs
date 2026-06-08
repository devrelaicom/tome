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

// --- closeout-added coverage (US3 4-reviewer pass) -------------------------

#[test]
fn catalog_with_an_escaping_plugin_source_is_refused_not_followed() {
    // SEC-1: a `plugins[].source` that escapes the catalog root must be refused
    // (so it never reaches an --autofix write), reported as a finding.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("cat");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("tome-catalog.toml"),
        "name = \"c\"\nversion = \"1.0.0\"\ndescription = \"d\"\n\n[owner]\nname = \"o\"\nemail = \"o@x.io\"\n\n[[plugins]]\nname = \"evil\"\nsource = \"../escape\"\n",
    )
    .unwrap();
    // A real plugin OUTSIDE the catalog root that an unvalidated join would reach.
    fs::create_dir_all(tmp.path().join("escape")).unwrap();
    fs::write(
        tmp.path().join("escape/tome-plugin.toml"),
        "name = \"escape\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();

    let artifact = parse_artifact(&dir).unwrap();
    let report = run(&artifact, &rules::all());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.rule_id == "lint/catalog-plugin-source-invalid"),
        "escaping source must be a finding: {:?}",
        report.diagnostics
    );
}

#[test]
fn malformed_manifest_is_a_finding_not_an_abort() {
    // The lenient-parse promise: a TOML syntax error is reported, and other
    // findings still surface.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("tome-plugin.toml"), "name = \nversion =").unwrap();
    // A skill with a real issue, to prove the run continues past the bad manifest.
    fs::create_dir_all(dir.join("skills/foo")).unwrap();
    fs::write(
        dir.join("skills/foo/SKILL.md"),
        "---\nname: foo\n---\nbody\n",
    )
    .unwrap();

    let artifact = parse_artifact(&dir).unwrap();
    let report = run(&artifact, &rules::all());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.rule_id == "lint/manifest-invalid"),
        "{:?}",
        report.diagnostics
    );
    // The skill's missing-description finding is still reported.
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.rule_id == "lint/description-missing"),
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn malformed_entry_is_a_finding_not_an_abort() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("tome-plugin.toml"),
        "name = \"p\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    fs::create_dir_all(dir.join("skills/bad")).unwrap();
    // No frontmatter delimiters → ENTRY_INVALID, not an abort.
    fs::write(dir.join("skills/bad/SKILL.md"), "no frontmatter here").unwrap();

    let artifact = parse_artifact(&dir).unwrap();
    let report = run(&artifact, &rules::all());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.rule_id == "lint/entry-invalid"),
        "{:?}",
        report.diagnostics
    );
}

#[cfg(unix)]
#[test]
fn lint_refuses_a_symlinked_skill_child_and_does_not_read_out_of_tree() {
    // SEC-MED-1 (phase-wide): the lint parse path must NOT follow a symlinked
    // component (consistent with the convert read boundary). A malicious native
    // plugin with a real `skills/` holding `evil -> /outside-skill` must not
    // disclose the out-of-tree SKILL.md into a finding — the listing is refused
    // and reported as `lint/unsafe-path`.
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir().unwrap();

    // An out-of-tree skill dir whose description is a recognizable secret.
    let outside = tmp.path().join("outside-skill");
    fs::create_dir_all(&outside).unwrap();
    fs::write(
        outside.join("SKILL.md"),
        "---\nname: evil\ndescription: TOP-SECRET-EXFIL\n---\nbody\n",
    )
    .unwrap();

    // The artifact under lint: a real plugin with a real `skills/` dir whose
    // child `evil` is a symlink to the out-of-tree skill dir.
    let dir = tmp.path().join("p");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("tome-plugin.toml"),
        "name = \"p\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    fs::create_dir(dir.join("skills")).unwrap();
    symlink(&outside, dir.join("skills/evil")).unwrap();

    let artifact = parse_artifact(&dir).unwrap();
    let report = run(&artifact, &rules::all());

    // The symlinked child makes the listing refuse → reported, not followed.
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.rule_id == "lint/unsafe-path"),
        "expected a lint/unsafe-path finding: {:?}",
        report.diagnostics
    );
    // The out-of-tree skill content never reached a finding (no disclosure).
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("TOP-SECRET-EXFIL")),
        "out-of-tree content leaked into findings: {:?}",
        report.diagnostics
    );
}

#[test]
fn findings_order_is_deterministic_across_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("tome-plugin.toml"),
        "name = \"p\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    // Three skills created out of alpha order, each missing a description.
    for name in ["gamma", "alpha", "beta"] {
        fs::create_dir_all(dir.join(format!("skills/{name}"))).unwrap();
        fs::write(
            dir.join(format!("skills/{name}/SKILL.md")),
            format!("---\nname: {name}\n---\nbody\n"),
        )
        .unwrap();
    }
    let seq = |r: &tome::authoring::lint::LintReport| {
        r.diagnostics
            .iter()
            .map(|x| {
                (
                    x.rule_id,
                    x.location.as_ref().map(|l| l.file.display().to_string()),
                )
            })
            .collect::<Vec<_>>()
    };
    let a = run(&parse_artifact(&dir).unwrap(), &rules::all());
    let b = run(&parse_artifact(&dir).unwrap(), &rules::all());
    assert_eq!(seq(&a), seq(&b), "findings order must be stable");
}
