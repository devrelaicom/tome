//! End-to-end `convert` pipeline tests (US2): a Claude Code plugin fixture →
//! native Tome plugin on disk, verifying the manifest cutover, harness-ism
//! rewrite, supporting-file copy, unsupported-component warnings, rename,
//! `--dry-run` (zero writes), and `--strict` abort.

use std::fs;
use std::path::{Path, PathBuf};

use tome::authoring::convert::{ConvertConfig, run};
use tome::authoring::detect::ArtifactLevel;
use tome::plugin::manifest::read_plugin_manifest;

/// Write a representative CC plugin under `<tmp>/src` and return its path.
fn cc_plugin_fixture(tmp: &Path) -> PathBuf {
    let src = tmp.join("src");
    fs::create_dir_all(src.join(".claude-plugin")).unwrap();
    fs::write(
        src.join(".claude-plugin/plugin.json"),
        br#"{"name":"demo","version":"2.1.0","description":"a demo","displayName":"Demo"}"#,
    )
    .unwrap();
    // A skill with a harness-ism + a tool restriction + a supporting script.
    fs::create_dir_all(src.join("skills/greet/scripts")).unwrap();
    fs::write(
        src.join("skills/greet/SKILL.md"),
        "---\nname: greet\ndescription: greets\nallowed-tools: Bash\n---\nRun ${CLAUDE_PLUGIN_ROOT}/scripts/run.sh\n",
    )
    .unwrap();
    fs::write(
        src.join("skills/greet/scripts/run.sh"),
        b"#!/bin/sh\necho hi\n",
    )
    .unwrap();
    // A command with a legacy positional.
    fs::create_dir(src.join("commands")).unwrap();
    fs::write(src.join("commands/say.md"), "---\nname: say\n---\nSay $1\n").unwrap();
    // An unsupported component dir.
    fs::create_dir(src.join("monitors")).unwrap();
    // An MCP server.
    fs::write(
        src.join(".mcp.json"),
        br#"{"mcpServers":{"svc":{"command":"node","args":["s.js"]}}}"#,
    )
    .unwrap();
    src
}

fn config(output_dir: PathBuf) -> ConvertConfig {
    ConvertConfig {
        level: ArtifactLevel::Plugin,
        from: None,
        new_name: None,
        strict: false,
        force: false,
        dry_run: false,
        output_dir,
    }
}

#[test]
fn converts_a_cc_plugin_to_a_native_tome_plugin() {
    let tmp = tempfile::tempdir().unwrap();
    let src = cc_plugin_fixture(tmp.path());
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();

    let outcome = run(&src, &config(out.clone())).unwrap();

    // Default rename: `<current>-tome`.
    assert_eq!(outcome.source_name, "demo");
    assert_eq!(outcome.final_name, "demo-tome");
    let target = out.join("demo-tome");
    assert_eq!(outcome.target, target);

    // The native manifest landed and parses, with the converted name.
    let manifest = read_plugin_manifest(&target).unwrap();
    assert_eq!(manifest.name, "demo-tome");
    assert_eq!(manifest.version, "2.1.0");
    assert_eq!(manifest.description.as_deref(), Some("a demo"));

    // The skill body had its harness-ism rewritten.
    let skill = fs::read_to_string(target.join("skills/greet/SKILL.md")).unwrap();
    assert!(
        skill.contains("${TOME_PLUGIN_DIR}/scripts/run.sh"),
        "{skill}"
    );
    assert!(!skill.contains("CLAUDE_PLUGIN_ROOT"));

    // Supporting file copied; command + MCP emitted.
    assert!(target.join("skills/greet/scripts/run.sh").exists());
    let cmd = fs::read_to_string(target.join("commands/say.md")).unwrap();
    assert!(cmd.contains("Say $0"), "legacy positional rewritten: {cmd}");
    assert!(target.join(".mcp.json").exists());

    // The report carries the unsupported-component + tool-restriction warnings.
    assert!(outcome.report.warnings >= 2);
    assert!(
        outcome
            .report
            .diagnostics
            .iter()
            .any(|d| d.rule_id == "convert/unsupported-component")
    );
    assert!(
        outcome
            .report
            .diagnostics
            .iter()
            .any(|d| d.rule_id == "convert/tool-restriction-dropped")
    );
}

#[test]
fn explicit_name_overrides_the_default() {
    let tmp = tempfile::tempdir().unwrap();
    let src = cc_plugin_fixture(tmp.path());
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();

    let mut cfg = config(out.clone());
    cfg.new_name = Some("custom".to_owned());
    let outcome = run(&src, &cfg).unwrap();

    assert_eq!(outcome.final_name, "custom");
    let manifest = read_plugin_manifest(&out.join("custom")).unwrap();
    assert_eq!(manifest.name, "custom");
}

#[test]
fn dry_run_writes_nothing_but_plans_files() {
    let tmp = tempfile::tempdir().unwrap();
    let src = cc_plugin_fixture(tmp.path());
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();

    let mut cfg = config(out.clone());
    cfg.dry_run = true;
    let outcome = run(&src, &cfg).unwrap();

    assert!(outcome.dry_run);
    assert!(!outcome.written.is_empty(), "dry-run still plans the files");
    assert!(
        !out.join("demo-tome").exists(),
        "dry-run must not create the target"
    );
}

#[test]
fn strict_aborts_on_an_unsupported_component_before_writing() {
    let tmp = tempfile::tempdir().unwrap();
    let src = cc_plugin_fixture(tmp.path()); // has monitors/
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();

    let mut cfg = config(out.clone());
    cfg.strict = true;
    let err = run(&src, &cfg).unwrap_err();
    assert_eq!(err.exit_code(), 84);
    assert!(
        !out.join("demo-tome").exists(),
        "strict abort must leave nothing on disk"
    );
}

#[test]
fn level_mismatch_is_a_usage_error() {
    // A bare skill source asked to convert as a plugin.
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("askill");
    fs::create_dir(&src).unwrap();
    fs::write(
        src.join("SKILL.md"),
        "---\nname: s\ndescription: d\n---\nbody\n",
    )
    .unwrap();
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();

    let err = run(&src, &config(out)).unwrap_err();
    assert_eq!(err.exit_code(), 2);
}
