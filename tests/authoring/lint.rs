//! Integration coverage for `lint` (US3): parse a native Tome artifact +
//! run the rule registry → verdict + exit codes. Read-only (the `--autofix`
//! application + command wrappers land in US3-2).
//!
//! Issue #326 adds the variadic-source command path
//! (`variadic_*`/`aggregate_*`/`never_halt_*` below), driven through
//! `tome::commands::lint::run` so the loop, aggregate exit code, and never-halt
//! forward-progress are exercised over the real command surface.

use std::fs;
use std::path::Path;

use tome::authoring::detect::ArtifactLevel;
use tome::authoring::lint::parse::parse_artifact;
use tome::authoring::lint::{rules, run};
use tome::cli::LintArgs;
use tome::output::Mode;
use tome::workspace::ResolvedScope;

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

// --- hooks-spec rule e2e coverage ------------------------------------------

#[test]
fn invalid_hooks_json_produces_a_hooks_spec_warning() {
    // A native plugin with a hooks/hooks.json that is not valid JSON must
    // produce a lint/hooks-spec warning. This is the signal that would otherwise
    // only surface at `harness sync` time (exit 43).
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("tome-plugin.toml"),
        "name = \"my-plugin\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    fs::create_dir_all(dir.join("hooks")).unwrap();
    fs::write(dir.join("hooks/hooks.json"), "{not json").unwrap();

    let artifact = parse_artifact(&dir).unwrap();
    let report = run(&artifact, &rules::all());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.rule_id == "lint/hooks-spec"),
        "expected lint/hooks-spec finding: {:?}",
        report.diagnostics
    );
}

#[test]
fn valid_hooks_json_produces_no_hooks_spec_finding() {
    // A native plugin with a hooks/hooks.json in the event-map form must not
    // produce any lint/hooks-spec finding.  The wrapped form {"hooks":{...}} IS
    // flagged (it would cause harness sync to exit 43); the event-map form is
    // the only accepted shape.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("tome-plugin.toml"),
        "name = \"my-plugin\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    fs::create_dir_all(dir.join("hooks")).unwrap();
    // Use the correct event-map form (not the wrapped {"hooks":{}} form).
    fs::write(dir.join("hooks/hooks.json"), r#"{"PreToolUse":[]}"#).unwrap();
    // A skill so the plugin is otherwise clean.
    fs::create_dir_all(dir.join("skills/foo")).unwrap();
    fs::write(
        dir.join("skills/foo/SKILL.md"),
        "---\nname: foo\ndescription: a skill\n---\nbody\n",
    )
    .unwrap();

    let artifact = parse_artifact(&dir).unwrap();
    let report = run(&artifact, &rules::all());
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|d| d.rule_id == "lint/hooks-spec"),
        "unexpected lint/hooks-spec finding: {:?}",
        report.diagnostics
    );
    assert_eq!(report.errors, 0, "{:?}", report.diagnostics);
}

#[test]
fn wrapped_hooks_json_produces_hooks_spec_finding() {
    // A native plugin whose hooks/hooks.json uses the wrapped form
    // ({"hooks":{...}}) must produce a lint/hooks-spec warning — this is the
    // shape that causes `harness sync` to exit 43.  The fix is to run
    // `tome catalog convert` which normalises the file to the event-map form.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("tome-plugin.toml"),
        "name = \"my-plugin\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    fs::create_dir_all(dir.join("hooks")).unwrap();
    // Wrapped form — discriminated by the top-level "hooks" key.
    fs::write(
        dir.join("hooks/hooks.json"),
        r#"{"hooks":{"PreToolUse":[{"type":"command","command":"run.sh"}]}}"#,
    )
    .unwrap();
    // A skill so the plugin is otherwise clean.
    fs::create_dir_all(dir.join("skills/foo")).unwrap();
    fs::write(
        dir.join("skills/foo/SKILL.md"),
        "---\nname: foo\ndescription: a skill\n---\nbody\n",
    )
    .unwrap();

    let artifact = parse_artifact(&dir).unwrap();
    let report = run(&artifact, &rules::all());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.rule_id == "lint/hooks-spec"),
        "wrapped form must produce lint/hooks-spec finding: {:?}",
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

// --- issue #326: variadic `lint` command path ------------------------------

/// A clean plugin at `<dir>/<name>`.
fn clean_plugin_at(dir: &Path, name: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    fs::create_dir(&p).unwrap();
    write_clean_plugin(&p);
    p
}

/// A plugin with an ERROR finding (invalid version + skill `name != dir`).
fn erroring_plugin_at(dir: &Path, name: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    fs::create_dir(&p).unwrap();
    fs::write(
        p.join("tome-plugin.toml"),
        "name = \"p\"\nversion = \"notsemver\"\n",
    )
    .unwrap();
    fs::create_dir_all(p.join("skills/realdir")).unwrap();
    fs::write(
        p.join("skills/realdir/SKILL.md"),
        "---\nname: wrong\ndescription: a skill\n---\nbody\n",
    )
    .unwrap();
    p
}

/// A plugin with a WARNING-only finding (a skill missing its description).
fn warning_plugin_at(dir: &Path, name: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    fs::create_dir(&p).unwrap();
    fs::write(
        p.join("tome-plugin.toml"),
        "name = \"p\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    fs::create_dir_all(p.join("skills/foo")).unwrap();
    fs::write(p.join("skills/foo/SKILL.md"), "---\nname: foo\n---\nbody\n").unwrap();
    p
}

fn lint_args(sources: Vec<std::path::PathBuf>, strict: bool) -> LintArgs {
    LintArgs {
        sources,
        autofix: false,
        dry_run: false,
        strict,
    }
}

/// Run the plugin-level `lint` command path (JSON mode so no human noise) and
/// return the boundary result. `Ok(())` = exit 0; the `Err`'s `exit_code()` is
/// the aggregate exit code (85/86/2/...).
fn run_plugin_lint(args: LintArgs) -> Result<(), i32> {
    let scope = ResolvedScope::global_fallback();
    tome::commands::lint::run(args, &scope, Mode::Json, ArtifactLevel::Plugin)
        .map_err(|e| e.exit_code())
}

#[test]
fn variadic_two_clean_sources_exit_zero() {
    let tmp = tempfile::tempdir().unwrap();
    let a = clean_plugin_at(tmp.path(), "a");
    let b = clean_plugin_at(tmp.path(), "b");
    assert_eq!(run_plugin_lint(lint_args(vec![a, b], false)), Ok(()));
}

#[test]
fn aggregate_exit_is_85_when_any_source_has_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let good = clean_plugin_at(tmp.path(), "good");
    let bad = erroring_plugin_at(tmp.path(), "bad");
    // Good source first, erroring second: the aggregate is still 85 (worst-of),
    // and the good source was NOT skipped by the bad one (order-independent).
    assert_eq!(run_plugin_lint(lint_args(vec![good, bad], false)), Err(85));
}

#[test]
fn aggregate_exit_is_86_when_strict_and_only_warnings() {
    let tmp = tempfile::tempdir().unwrap();
    let clean = clean_plugin_at(tmp.path(), "clean");
    let warn = warning_plugin_at(tmp.path(), "warn");
    // Without --strict, warnings pass (exit 0).
    assert_eq!(
        run_plugin_lint(lint_args(vec![clean.clone(), warn.clone()], false)),
        Ok(())
    );
    // With --strict, the warning-only source promotes the aggregate to 86.
    assert_eq!(run_plugin_lint(lint_args(vec![clean, warn], true)), Err(86));
}

#[test]
fn never_halt_a_bad_source_does_not_abort_the_rest_and_dominates() {
    let tmp = tempfile::tempdir().unwrap();
    // A non-existent / unparsable source sandwiched between two clean ones.
    let good1 = clean_plugin_at(tmp.path(), "good1");
    let missing = tmp.path().join("does-not-exist");
    let good2 = clean_plugin_at(tmp.path(), "good2");

    // The missing source is a pre-report failure; in multi-source mode it is
    // captured (never-halt) and drives the aggregate to errors → 85, while the
    // two clean sources are still linted (no early abort). If the loop had
    // halted on the missing source, `good2` would never be reached — the fact
    // that we get a deterministic 85 (not a propagated Usage/2) proves both the
    // never-halt AND the aggregate-worst-of behaviour.
    assert_eq!(
        run_plugin_lint(lint_args(vec![good1, missing, good2], false)),
        Err(85)
    );
}

#[test]
fn single_source_propagates_a_parse_error_as_usage_not_85() {
    // Back-compat: a SINGLE non-Tome/non-existent source keeps the pre-#326
    // behaviour — the parse `Usage`(2) propagates verbatim, NOT the aggregate
    // 85. (The multi path converts it to 85; the single path must not.)
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("nope");
    assert_eq!(run_plugin_lint(lint_args(vec![missing], false)), Err(2));
}

#[test]
fn single_source_clean_and_error_exit_codes_are_unchanged() {
    let tmp = tempfile::tempdir().unwrap();
    let clean = clean_plugin_at(tmp.path(), "clean");
    let bad = erroring_plugin_at(tmp.path(), "bad");
    assert_eq!(run_plugin_lint(lint_args(vec![clean], false)), Ok(()));
    assert_eq!(run_plugin_lint(lint_args(vec![bad], false)), Err(85));
}

// --- Gap 1: mcp-spec rule integration tests ---------------------------------

#[test]
fn mcp_json_invalid_json_flags_mcp_spec() {
    // A native plugin with a malformed .mcp.json must produce a lint/mcp-spec
    // warning — this is the signal that would otherwise only surface at
    // `harness sync` time.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("tome-plugin.toml"),
        "name = \"my-plugin\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    fs::write(dir.join(".mcp.json"), "{not valid json").unwrap();

    let artifact = parse_artifact(&dir).unwrap();
    let report = run(&artifact, &rules::all());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.rule_id == "lint/mcp-spec"),
        "expected lint/mcp-spec finding: {:?}",
        report.diagnostics
    );
}

#[test]
fn mcp_json_valid_object_passes() {
    // A native plugin with a valid .mcp.json (a JSON object) must not produce
    // any lint/mcp-spec finding.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("tome-plugin.toml"),
        "name = \"my-plugin\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    fs::write(dir.join(".mcp.json"), r#"{"mcpServers":{}}"#).unwrap();
    // A skill so the plugin is otherwise clean.
    fs::create_dir_all(dir.join("skills/foo")).unwrap();
    fs::write(
        dir.join("skills/foo/SKILL.md"),
        "---\nname: foo\ndescription: a skill\n---\nbody\n",
    )
    .unwrap();

    let artifact = parse_artifact(&dir).unwrap();
    let report = run(&artifact, &rules::all());
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|d| d.rule_id == "lint/mcp-spec"),
        "unexpected lint/mcp-spec finding: {:?}",
        report.diagnostics
    );
    assert_eq!(report.errors, 0, "{:?}", report.diagnostics);
}

// --- Gap 2: agent-spec rule integration tests --------------------------------

#[test]
fn agent_spec_wrong_tools_type_flags() {
    // An agent entry with `tools: 7` (scalar not list) must produce a
    // lint/agent-spec warning — this is what would fail harness sync at exit 45.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("tome-plugin.toml"),
        "name = \"my-plugin\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    fs::create_dir(dir.join("agents")).unwrap();
    fs::write(
        dir.join("agents/helper.md"),
        "---\nname: helper\ndescription: helps\ntools: 7\n---\nbody\n",
    )
    .unwrap();

    let artifact = parse_artifact(&dir).unwrap();
    let report = run(&artifact, &rules::all());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.rule_id == "lint/agent-spec"),
        "expected lint/agent-spec finding: {:?}",
        report.diagnostics
    );
}

// --- Gap 3: hooks-spec non-array values integration test --------------------

#[test]
fn hooks_spec_non_array_event_value_flags() {
    // A hooks.json where an event key maps to a non-array value (e.g. a string)
    // must produce a lint/hooks-spec warning — harness sync deserialises as
    // HashMap<String, Vec<HookEntry>> which would fail on a non-array value.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("tome-plugin.toml"),
        "name = \"my-plugin\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    fs::create_dir_all(dir.join("hooks")).unwrap();
    fs::write(
        dir.join("hooks/hooks.json"),
        r#"{"PreToolUse": "not-an-array"}"#,
    )
    .unwrap();

    let artifact = parse_artifact(&dir).unwrap();
    let report = run(&artifact, &rules::all());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.rule_id == "lint/hooks-spec"),
        "expected lint/hooks-spec finding for non-array value: {:?}",
        report.diagnostics
    );
}
