//! End-to-end `convert` pipeline tests (US2): a Claude Code plugin fixture →
//! native Tome plugin on disk, verifying the manifest cutover, harness-ism
//! rewrite, supporting-file copy, unsupported-component warnings, rename,
//! `--dry-run` (zero writes), and `--strict` abort.

use std::fs;
use std::path::{Path, PathBuf};

use tome::authoring::convert::{ConvertConfig, run};
use tome::authoring::detect::{ArtifactLevel, SourceHarness};
use tome::authoring::lint::parse::parse_artifact;
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
    // Hooks subtree — passed through verbatim.
    fs::create_dir_all(src.join("hooks")).unwrap();
    fs::write(
        src.join("hooks/hooks.json"),
        br#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"${CLAUDE_PLUGIN_ROOT}/hooks/run.sh"}]}]}}"#,
    )
    .unwrap();
    fs::write(src.join("hooks/run.sh"), b"#!/bin/sh\necho hooked\n").unwrap();
    src
}

fn config(output_dir: PathBuf) -> ConvertConfig {
    ConvertConfig {
        level: ArtifactLevel::Plugin,
        from: None,
        new_name: None,
        strict: false,
        allow: Vec::new(),
        force: false,
        dry_run: false,
        fetch_remote: true,
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
fn converted_plugin_relints_clean_from_disk() {
    // T-MAJOR-1 (phase-wide): convert lints the in-memory IR; this proves the
    // EMITTED on-disk tree re-parses and re-lints clean — the convert→lint
    // composition the quickstart headlines (and that `create` already pins).
    // Uses a minimal CLEAN source (one skill w/ name+description+rewritable
    // harness-ism; no unsupported dirs, no description-less command).
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(src.join(".claude-plugin")).unwrap();
    fs::write(
        src.join(".claude-plugin/plugin.json"),
        br#"{"name":"kit","version":"1.0.0","description":"a kit"}"#,
    )
    .unwrap();
    fs::create_dir_all(src.join("skills/greet")).unwrap();
    fs::write(
        src.join("skills/greet/SKILL.md"),
        "---\nname: greet\ndescription: greets the user\n---\nRun ${CLAUDE_PLUGIN_ROOT}/x\n",
    )
    .unwrap();

    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();
    let outcome = run(&src, &config(out.clone())).unwrap();
    let target = out.join(&outcome.final_name);

    // Re-parse + re-lint the EMITTED tree through the lenient lint parser.
    let artifact = parse_artifact(&target).unwrap();
    let report = tome::authoring::lint::run(&artifact, &tome::authoring::lint::rules::all());
    assert_eq!(report.errors, 0, "errors: {:?}", report.diagnostics);
    assert_eq!(report.warnings, 0, "warnings: {:?}", report.diagnostics);
    // And the strict cutover reader accepts the emitted manifest.
    assert_eq!(
        read_plugin_manifest(&target).unwrap().name,
        outcome.final_name
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
fn strict_reports_all_blocking_rule_ids_not_just_the_first() {
    // The CC fixture has TWO distinct strict-blocking findings:
    // `convert/unsupported-component` (monitors/) and
    // `convert/tool-restriction-dropped` (allowed-tools: Bash). The strict error
    // must name BOTH and count the findings, not stop at the first.
    let tmp = tempfile::tempdir().unwrap();
    let src = cc_plugin_fixture(tmp.path());
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();

    let mut cfg = config(out.clone());
    cfg.strict = true;
    let err = run(&src, &cfg).unwrap_err();
    assert_eq!(err.exit_code(), 84);
    let msg = err.to_string();
    assert!(
        msg.contains("convert/unsupported-component"),
        "names the component rule: {msg}"
    );
    assert!(
        msg.contains("convert/tool-restriction-dropped"),
        "names the tool-restriction rule: {msg}"
    );
    assert!(!out.join("demo-tome").exists(), "still writes nothing");
}

#[test]
fn strict_with_allow_demotes_a_rule_and_succeeds() {
    // Allowing BOTH blocking rule ids lets `--strict` convert successfully; the
    // demoted findings still appear in the report as warnings (not blocking).
    let tmp = tempfile::tempdir().unwrap();
    let src = cc_plugin_fixture(tmp.path());
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();

    let mut cfg = config(out.clone());
    cfg.strict = true;
    cfg.allow = vec![
        "convert/unsupported-component".to_owned(),
        "convert/tool-restriction-dropped".to_owned(),
    ];
    let outcome = run(&src, &cfg).expect("strict convert succeeds once both rules allowed");

    // Output was written (all-or-nothing, but nothing blocked).
    assert!(out.join("demo-tome").exists(), "converted output landed");
    assert!(!outcome.written.is_empty());

    // The demoted diagnostics are STILL present in the normal report.
    assert!(
        outcome
            .report
            .diagnostics
            .iter()
            .any(|d| d.rule_id == "convert/unsupported-component"),
        "demoted rule still emitted as a diagnostic"
    );
    assert!(
        outcome
            .report
            .diagnostics
            .iter()
            .any(|d| d.rule_id == "convert/tool-restriction-dropped"),
        "demoted rule still emitted as a diagnostic"
    );
}

#[test]
fn strict_with_partial_allow_still_aborts_on_the_remaining_rule() {
    // Allowing only ONE of the two blocking rules still aborts on the other.
    let tmp = tempfile::tempdir().unwrap();
    let src = cc_plugin_fixture(tmp.path());
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();

    let mut cfg = config(out.clone());
    cfg.strict = true;
    cfg.allow = vec!["convert/unsupported-component".to_owned()];
    let err = run(&src, &cfg).unwrap_err();
    assert_eq!(err.exit_code(), 84);
    let msg = err.to_string();
    assert!(
        msg.contains("convert/tool-restriction-dropped"),
        "aborts on the un-allowed rule: {msg}"
    );
    assert!(
        !msg.contains("convert/unsupported-component"),
        "the allowed rule is not reported as blocking: {msg}"
    );
    assert!(!out.join("demo-tome").exists(), "still writes nothing");
}

#[test]
fn allow_of_unknown_or_non_blocking_rule_has_no_effect() {
    // An `--allow` naming a rule id that is neither blocking nor real is a
    // harmless no-op: the strict abort proceeds exactly as if it were absent.
    let tmp = tempfile::tempdir().unwrap();
    let src = cc_plugin_fixture(tmp.path());
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();

    let mut cfg = config(out.clone());
    cfg.strict = true;
    cfg.allow = vec![
        "convert/does-not-exist".to_owned(),
        "convert/missing-version".to_owned(), // a real but non-blocking rule id
    ];
    let err = run(&src, &cfg).unwrap_err();
    assert_eq!(
        err.exit_code(),
        84,
        "no-op allow does not change the verdict"
    );
    let msg = err.to_string();
    assert!(msg.contains("convert/unsupported-component"), "{msg}");
    assert!(msg.contains("convert/tool-restriction-dropped"), "{msg}");
    assert!(!out.join("demo-tome").exists(), "still writes nothing");
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

fn skill_config(output_dir: PathBuf, from: Option<SourceHarness>) -> ConvertConfig {
    ConvertConfig {
        level: ArtifactLevel::Skill,
        from,
        new_name: None,
        strict: false,
        allow: Vec::new(),
        force: false,
        dry_run: false,
        fetch_remote: true,
        output_dir,
    }
}

#[test]
fn converts_a_native_skill_to_a_naked_tome_skill() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("greeter");
    fs::create_dir(&src).unwrap();
    fs::write(
        src.join("SKILL.md"),
        "---\nname: greeter\ndescription: greets\n---\nUse ${CLAUDE_SKILL_DIR}/x\n",
    )
    .unwrap();
    fs::create_dir(src.join("references")).unwrap();
    fs::write(src.join("references/r.md"), b"ref").unwrap();
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();

    let outcome = run(&src, &skill_config(out.clone(), None)).unwrap();
    assert_eq!(outcome.final_name, "greeter-tome");

    let target = out.join("greeter-tome");
    let skill = fs::read_to_string(target.join("SKILL.md")).unwrap();
    assert!(skill.contains("${TOME_SKILL_DIR}/x"), "{skill}");
    // name == dir preserved through the rename.
    assert!(skill.contains("name: greeter-tome"), "{skill}");
    assert!(target.join("references/r.md").exists());
}

#[test]
fn cline_skill_remaps_docs_to_references() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("clineskill");
    fs::create_dir(&src).unwrap();
    fs::write(
        src.join("SKILL.md"),
        "---\nname: clineskill\ndescription: d\n---\nbody\n",
    )
    .unwrap();
    fs::create_dir(src.join("docs")).unwrap();
    fs::write(src.join("docs/guide.md"), b"g").unwrap();
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();

    let outcome = run(&src, &skill_config(out.clone(), Some(SourceHarness::Cline))).unwrap();
    let target = out.join(&outcome.final_name);
    assert!(target.join("references/guide.md").exists());
    assert!(
        !target.join("docs").exists(),
        "cline `docs/` must be remapped"
    );
}

#[test]
fn converts_a_codex_project_to_a_synthesized_plugin() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("proj");
    fs::create_dir(&src).unwrap();
    fs::create_dir_all(src.join(".agents/skills/helper")).unwrap();
    fs::write(
        src.join(".agents/skills/helper/SKILL.md"),
        "---\nname: helper\ndescription: h\n---\nbody\n",
    )
    .unwrap();
    fs::write(
        src.join("config.toml"),
        "[mcp_servers.local]\ncommand = \"node\"\n",
    )
    .unwrap();
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();

    // `plugin convert` over a Codex project synthesizes a Tome plugin.
    let outcome = run(&src, &config(out.clone())).unwrap();
    assert_eq!(outcome.harness.as_str(), "codex");
    assert_eq!(outcome.final_name, "proj-tome");

    let target = out.join("proj-tome");
    let manifest = read_plugin_manifest(&target).unwrap();
    assert_eq!(manifest.name, "proj-tome");
    assert_eq!(manifest.version, "0.0.0");
    assert!(target.join("skills/helper/SKILL.md").exists());
    assert!(target.join(".mcp.json").exists());
}

/// Build a CC marketplace with one relative-path plugin + one remote plugin.
///
/// The `alpha` plugin includes a `hooks/` subtree (with a `${CLAUDE_PLUGIN_ROOT}`
/// token in `hooks.json`) to verify that hooks land NAMESPACED under the vendored
/// plugin directory (`alpha/hooks/`), never flat at the catalog root.
fn cc_marketplace_fixture(tmp: &Path) -> PathBuf {
    let src = tmp.join("mkt");
    fs::create_dir_all(src.join(".claude-plugin")).unwrap();
    fs::write(
        src.join(".claude-plugin/marketplace.json"),
        br#"{"name":"mkt","owner":{"name":"Owner","email":"o@x.io"},
             "plugins":[{"name":"alpha","source":"./alpha"},
                        {"name":"beta","source":{"source":"github","repo":"x/y"}}]}"#,
    )
    .unwrap();
    // The relative-path plugin `alpha`.
    fs::create_dir_all(src.join("alpha/.claude-plugin")).unwrap();
    fs::write(
        src.join("alpha/.claude-plugin/plugin.json"),
        br#"{"name":"alpha","version":"1.2.0"}"#,
    )
    .unwrap();
    fs::create_dir_all(src.join("alpha/skills/s")).unwrap();
    fs::write(
        src.join("alpha/skills/s/SKILL.md"),
        "---\nname: s\ndescription: d\n---\nbody\n",
    )
    .unwrap();
    // hooks/ subtree for the namespaced-placement test.
    fs::create_dir_all(src.join("alpha/hooks")).unwrap();
    fs::write(
        src.join("alpha/hooks/hooks.json"),
        br#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"${CLAUDE_PLUGIN_ROOT}/run.sh"}]}]}}"#,
    )
    .unwrap();
    src
}

fn catalog_config(output_dir: PathBuf, strict: bool) -> ConvertConfig {
    ConvertConfig {
        level: ArtifactLevel::Catalog,
        from: None,
        new_name: None,
        strict,
        allow: Vec::new(),
        force: false,
        dry_run: false,
        fetch_remote: true,
        output_dir,
    }
}

#[test]
fn converts_a_marketplace_vendoring_relative_plugins_and_skipping_remote() {
    // The fixture uses a github remote that cannot be fetched in tests; we
    // disable fetching so it degrades to the hermetic warn-and-skip path.
    let tmp = tempfile::tempdir().unwrap();
    let src = cc_marketplace_fixture(tmp.path());
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();

    let mut cfg = catalog_config(out.clone(), false);
    cfg.fetch_remote = false;
    let outcome = run(&src, &cfg).unwrap();
    assert_eq!(outcome.final_name, "mkt-tome");

    let target = out.join("mkt-tome");
    // The catalog manifest + the vendored relative plugin landed.
    let cat = fs::read_to_string(target.join("tome-catalog.toml")).unwrap();
    assert!(cat.contains("name = \"mkt-tome\""), "{cat}");
    assert!(read_plugin_manifest(&target.join("alpha")).is_ok());
    assert!(target.join("alpha/skills/s/SKILL.md").exists());
    // Under --no-fetch the remote plugin was skipped + warned, not vendored.
    assert!(!target.join("beta").exists());
    assert!(
        outcome
            .report
            .diagnostics
            .iter()
            .any(|d| d.rule_id == "convert/remote-plugin-skipped")
    );
}

#[test]
fn strict_marketplace_hard_fails_on_a_remote_plugin() {
    // With fetch disabled, a github remote produces remote-plugin-skipped which
    // is strict-blocking → exit 84.
    let tmp = tempfile::tempdir().unwrap();
    let src = cc_marketplace_fixture(tmp.path());
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();

    let mut cfg = catalog_config(out.clone(), true);
    cfg.fetch_remote = false;
    let err = run(&src, &cfg).unwrap_err();
    assert_eq!(err.exit_code(), 84);
    assert!(!out.join("mkt-tome").exists(), "strict abort lands nothing");
}

#[test]
fn vendored_catalog_plugin_hooks_land_namespaced_not_flat() {
    // Hooks must be emitted under `<catalog>/<plugin>/hooks/`, not flat at the
    // catalog root (`<catalog>/hooks/`). The cc_marketplace_fixture alpha plugin
    // has a hooks/hooks.json with a ${CLAUDE_PLUGIN_ROOT} token.
    let tmp = tempfile::tempdir().unwrap();
    let src = cc_marketplace_fixture(tmp.path());
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();

    let mut cfg = catalog_config(out.clone(), false);
    cfg.fetch_remote = false; // beta is a github remote, skip it
    let outcome = run(&src, &cfg).unwrap();
    let root = out.join(&outcome.final_name);

    // hooks.json lands namespaced under the alpha plugin directory.
    assert!(
        root.join("alpha/hooks/hooks.json").is_file(),
        "hooks must land under alpha/hooks/, not at the catalog root"
    );
    // No flat hooks/ directory at the catalog root.
    assert!(
        !root.join("hooks").exists(),
        "hooks must NOT appear flat at the catalog root"
    );
    // Token is intact (verbatim pass-through, no harness-ism rewrite at convert time).
    let text = fs::read_to_string(root.join("alpha/hooks/hooks.json")).unwrap();
    assert!(
        text.contains("${CLAUDE_PLUGIN_ROOT}"),
        "token must survive verbatim: {text}"
    );
}

/// Run a git subcommand in `dir`, asserting success (identity injected so CI
/// never prompts).
fn git_cmd(args: &[&str], dir: &Path) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Tome Test")
        .env("GIT_AUTHOR_EMAIL", "tests@tome.invalid")
        .env("GIT_COMMITTER_NAME", "Tome Test")
        .env("GIT_COMMITTER_EMAIL", "tests@tome.invalid")
        .status()
        .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
    assert!(status.success(), "git {args:?} exited {status}");
}

/// A minimal CC plugin repo committed to git, returning its `file://` URL.
///
/// Includes a `hooks/` subtree (with a `${CLAUDE_PLUGIN_ROOT}` token in
/// `hooks.json`) to verify the FetchContext keepalive contract: the temp clone
/// must stay alive across emit so the hooks files can be copied from it.
fn remote_plugin_repo(tmp: &Path, name: &str) -> String {
    let repo = tmp.join(name);
    fs::create_dir_all(repo.join(".claude-plugin")).unwrap();
    fs::write(
        repo.join(".claude-plugin/plugin.json"),
        format!(r#"{{"name":"{name}","version":"1.0.0","description":"d"}}"#),
    )
    .unwrap();
    fs::create_dir_all(repo.join("skills/hello")).unwrap();
    fs::write(
        repo.join("skills/hello/SKILL.md"),
        "---\nname: hello\ndescription: says hello\n---\nHello.\n",
    )
    .unwrap();
    fs::create_dir_all(repo.join("hooks")).unwrap();
    fs::write(
        repo.join("hooks/hooks.json"),
        br#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"${CLAUDE_PLUGIN_ROOT}/hooks/run.sh"}]}]}}"#,
    )
    .unwrap();
    fs::write(repo.join("hooks/run.sh"), b"#!/bin/sh\n").unwrap();
    git_cmd(&["init", "-q", "-b", "main"], &repo);
    git_cmd(&["add", "-A"], &repo);
    git_cmd(&["commit", "-q", "-m", "init"], &repo);
    format!("file://{}", repo.display())
}

/// A marketplace whose plugins mix a vendored relative source, a fetchable
/// `url` source, an unreachable `url` source, and an unfetchable npm source.
fn marketplace_with_remotes(tmp: &Path, good_url: &str, bad_url: &str) -> PathBuf {
    let src = tmp.join("market");
    fs::create_dir_all(src.join(".claude-plugin")).unwrap();
    fs::write(
        src.join(".claude-plugin/marketplace.json"),
        format!(
            r#"{{"name":"mixed","version":"1.0.0","description":"d",
                 "owner":{{"name":"o","email":"o@x.io"}},
                 "plugins":[
                   {{"name":"local-one","source":"./local-one"}},
                   {{"name":"fetched-one","source":{{"source":"url","url":"{good_url}"}}}},
                   {{"name":"broken-one","source":{{"source":"url","url":"{bad_url}"}}}},
                   {{"name":"npm-one","source":{{"source":"npm","package":"x"}}}}
                 ]}}"#
        ),
    )
    .unwrap();
    fs::create_dir_all(src.join("local-one/.claude-plugin")).unwrap();
    fs::write(
        src.join("local-one/.claude-plugin/plugin.json"),
        br#"{"name":"local-one","version":"1.0.0"}"#,
    )
    .unwrap();
    src
}

#[test]
fn catalog_convert_fetches_remote_plugins_and_skips_failures() {
    tome::authoring::import::claude_code::ALLOW_FILE_URLS_FOR_TESTS
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let tmp = tempfile::tempdir().unwrap();
    let good = remote_plugin_repo(tmp.path(), "fetched-one");
    let bad = format!("file://{}/nonexistent", tmp.path().display());
    let src = marketplace_with_remotes(tmp.path(), &good, &bad);
    let out = tmp.path().join("out");

    let outcome = run(&src, &catalog_config(out.clone(), false)).expect("convert");
    let root = out.join(&outcome.final_name);

    // The relative plugin AND the fetched remote plugin are vendored.
    assert!(root.join("local-one/tome-plugin.toml").is_file());
    assert!(root.join("fetched-one/tome-plugin.toml").is_file());
    assert!(root.join("fetched-one/skills/hello/SKILL.md").is_file());
    // Both are registered in the catalog manifest.
    let manifest = fs::read_to_string(root.join("tome-catalog.toml")).unwrap();
    assert!(manifest.contains("name = \"fetched-one\""), "{manifest}");

    // Forward-progress: the unreachable URL warned, did not abort.
    assert!(outcome.report.diagnostics.iter().any(|d| {
        d.rule_id == "convert/remote-plugin-fetch-failed" && d.message.contains("broken-one")
    }));
    // npm stays skipped under the existing rule.
    assert!(outcome.report.diagnostics.iter().any(|d| {
        d.rule_id == "convert/remote-plugin-skipped" && d.message.contains("npm-one")
    }));
    // The fetched plugin is reported.
    assert!(outcome.report.diagnostics.iter().any(|d| {
        d.rule_id == "convert/remote-plugin-fetched" && d.message.contains("fetched-one")
    }));
    assert!(!root.join("broken-one").exists());

    // Fetched-plugin hooks: Copy sources live in the temp clone, which the
    // FetchContext keepalive holds alive across emit — the hooks must land
    // namespaced under the vendored plugin with the token intact.
    let hooks = fs::read_to_string(root.join("fetched-one/hooks/hooks.json")).unwrap();
    assert!(hooks.contains("${CLAUDE_PLUGIN_ROOT}"));
    assert!(root.join("fetched-one/hooks/run.sh").is_file());
}

#[test]
fn catalog_convert_no_fetch_restores_hermetic_skip() {
    tome::authoring::import::claude_code::ALLOW_FILE_URLS_FOR_TESTS
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let tmp = tempfile::tempdir().unwrap();
    let good = remote_plugin_repo(tmp.path(), "fetched-one");
    let bad = format!("file://{}/nonexistent", tmp.path().display());
    let src = marketplace_with_remotes(tmp.path(), &good, &bad);
    let out = tmp.path().join("out");

    let mut cfg = catalog_config(out.clone(), false);
    cfg.fetch_remote = false;
    let outcome = run(&src, &cfg).expect("convert");
    let root = out.join(&outcome.final_name);

    assert!(root.join("local-one/tome-plugin.toml").is_file());
    assert!(
        !root.join("fetched-one").exists(),
        "--no-fetch must not clone"
    );
    // Every remote (fetchable or not) warned under the skip rule.
    let skips = outcome
        .report
        .diagnostics
        .iter()
        .filter(|d| d.rule_id == "convert/remote-plugin-skipped")
        .count();
    assert_eq!(skips, 3, "fetched-one + broken-one + npm-one all skipped");
}

#[test]
fn strict_aborts_on_a_remote_fetch_failure() {
    tome::authoring::import::claude_code::ALLOW_FILE_URLS_FOR_TESTS
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let tmp = tempfile::tempdir().unwrap();
    let good = remote_plugin_repo(tmp.path(), "fetched-one");
    let bad = format!("file://{}/nonexistent", tmp.path().display());
    let src = marketplace_with_remotes(tmp.path(), &good, &bad);
    let out = tmp.path().join("out");

    let err = run(&src, &catalog_config(out.clone(), true)).unwrap_err();
    assert_eq!(err.exit_code(), 84, "strict fetch failure aborts: {err}");
    assert!(!out.exists(), "strict abort writes nothing");
}

#[test]
fn marketplace_with_a_broken_relative_plugin_is_all_or_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("mkt");
    fs::create_dir_all(src.join(".claude-plugin")).unwrap();
    fs::write(
        src.join(".claude-plugin/marketplace.json"),
        br#"{"name":"mkt","owner":{"name":"O","email":"o@x.io"},
             "plugins":[{"name":"broken","source":"./broken"}]}"#,
    )
    .unwrap();
    // `broken` has an invalid plugin.json → import fails → whole catalog aborts.
    fs::create_dir_all(src.join("broken/.claude-plugin")).unwrap();
    fs::write(src.join("broken/.claude-plugin/plugin.json"), b"{not json").unwrap();
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();

    let err = run(&src, &catalog_config(out.clone(), false)).unwrap_err();
    assert_eq!(err.exit_code(), 2);
    assert!(
        !out.join("mkt-tome").exists(),
        "a single plugin failure must land nothing (all-or-nothing)"
    );
}

// --- closeout-added coverage (US2 4-reviewer pass) -------------------------

#[test]
fn every_unsupported_component_type_warns_exactly_once() {
    // SC-002: a warning for EVERY unsupported component type, exact count.
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(src.join(".claude-plugin")).unwrap();
    fs::write(
        src.join(".claude-plugin/plugin.json"),
        br#"{"name":"p","version":"1.0.0"}"#,
    )
    .unwrap();
    for d in [
        "monitors",
        "themes",
        "lsp",
        "output-styles",
        "channels",
        "bin",
    ] {
        fs::create_dir(src.join(d)).unwrap();
    }
    fs::write(src.join("settings.json"), b"{}").unwrap();
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();

    let outcome = run(&src, &config(out)).unwrap();
    let count = outcome
        .report
        .diagnostics
        .iter()
        .filter(|d| d.rule_id == "convert/unsupported-component")
        .count();
    // 6 component dirs + settings.json (hooks/ is now a verbatim pass-through,
    // not an unsupported component).
    assert_eq!(count, 7, "{:?}", outcome.report.diagnostics);
}

#[test]
fn skill_convert_with_non_utf8_body_is_named_error_and_emits_nothing() {
    // SC-012 / FR-011a: a non-UTF-8 body is a named, fail-closed error and
    // produces no output (never lossy-decode-then-rewrite).
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("badskill");
    fs::create_dir(&src).unwrap();
    fs::write(src.join("SKILL.md"), [0xff, 0xfe, 0x00, 0x66]).unwrap();
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();

    let err = run(&src, &skill_config(out.clone(), None)).unwrap_err();
    assert_eq!(err.exit_code(), 7);
    assert!(
        !out.join("badskill-tome").exists(),
        "a non-UTF-8 body must leave nothing on disk"
    );
}

#[test]
fn convert_normalises_wrapped_hooks_json_and_keeps_token() {
    // The fixture's hooks.json uses the wrapped form {"hooks":{...}};
    // convert must unwrap it to the event-map form that `harness sync` expects,
    // while preserving the ${CLAUDE_PLUGIN_ROOT} token intact for the
    // sync-time rewriter.
    let tmp = tempfile::tempdir().unwrap();
    let src = cc_plugin_fixture(tmp.path());
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();
    let outcome = run(&src, &config(out.clone())).expect("convert");
    let root = out.join(&outcome.final_name);

    assert!(
        root.join("hooks/run.sh").is_file(),
        "hooks/run.sh must be copied"
    );

    let text = fs::read_to_string(root.join("hooks/hooks.json")).unwrap();
    // Token must survive normalisation (sync-time rewriter owns it).
    assert!(
        text.contains("${CLAUDE_PLUGIN_ROOT}"),
        "token must NOT be rewritten: {text}"
    );
    // The output must be the event-map form (no wrapper key) so harness sync works.
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let obj = v.as_object().unwrap();
    assert!(
        !obj.contains_key("hooks"),
        "converted hooks.json must be event-map, not wrapped: {text}"
    );
    assert!(
        obj.contains_key("SessionStart"),
        "event must be at the top level after normalisation: {text}"
    );
}

#[test]
fn conversion_is_byte_stable_across_runs() {
    // FR-027: re-running over unchanged input is byte-identical (whole tree,
    // not just the serializer).
    let tmp = tempfile::tempdir().unwrap();
    let src = cc_plugin_fixture(tmp.path());
    let out1 = tmp.path().join("out1");
    let out2 = tmp.path().join("out2");
    fs::create_dir(&out1).unwrap();
    fs::create_dir(&out2).unwrap();
    run(&src, &config(out1.clone())).unwrap();
    run(&src, &config(out2.clone())).unwrap();

    let t1 = out1.join("demo-tome");
    let t2 = out2.join("demo-tome");
    for f in [
        "tome-plugin.toml",
        "skills/greet/SKILL.md",
        "skills/greet/scripts/run.sh",
        "commands/say.md",
        ".mcp.json",
        "hooks/hooks.json",
        "hooks/run.sh",
    ] {
        let a = fs::read(t1.join(f)).unwrap_or_else(|_| panic!("missing {f} in run1"));
        let b = fs::read(t2.join(f)).unwrap_or_else(|_| panic!("missing {f} in run2"));
        assert_eq!(a, b, "byte drift in {f}");
    }
}

#[test]
fn strict_aborts_on_an_unreadable_hooks_json() {
    // A binary (non-UTF-8) hooks.json emits convert/hooks-unreadable (Warning,
    // strict-blocking) and, under --strict, aborts before writing anything.
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(src.join(".claude-plugin")).unwrap();
    fs::write(
        src.join(".claude-plugin/plugin.json"),
        br#"{"name":"p","version":"1.0.0"}"#,
    )
    .unwrap();
    fs::create_dir_all(src.join("hooks")).unwrap();
    // 4 bytes that are not valid UTF-8.
    fs::write(src.join("hooks/hooks.json"), [0xFF, 0xFE, 0x00, 0x9C]).unwrap();
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();
    let mut cfg = config(out.clone());
    cfg.strict = true;
    let err = run(&src, &cfg).unwrap_err();
    assert_eq!(err.exit_code(), 84, "{err}");
    assert!(!out.join("p-tome").exists(), "strict abort writes nothing");
}

#[test]
fn strict_aborts_on_a_malformed_hooks_json() {
    // Symmetry with the unreadable case: valid-UTF-8/invalid-JSON hooks.json
    // is strict-blocking too (it would hard-fail harness sync at exit 43).
    // Contrast: without --strict the convert succeeds with a lint warning only.
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(src.join(".claude-plugin")).unwrap();
    fs::write(
        src.join(".claude-plugin/plugin.json"),
        br#"{"name":"p","version":"1.0.0"}"#,
    )
    .unwrap();
    fs::create_dir_all(src.join("hooks")).unwrap();
    fs::write(src.join("hooks/hooks.json"), b"{not json").unwrap();
    let out = tmp.path().join("out");
    let mut cfg = config(out.clone());
    cfg.strict = true;
    let err = run(&src, &cfg).unwrap_err();
    assert_eq!(err.exit_code(), 84, "{err}");
    assert!(!out.exists(), "strict abort writes nothing");
}

#[test]
fn output_collision_is_81_without_force_and_overwrites_with_force() {
    let tmp = tempfile::tempdir().unwrap();
    let src = cc_plugin_fixture(tmp.path());
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();

    run(&src, &config(out.clone())).unwrap();
    // Second run, same target, no --force → OutputExists(81).
    let err = run(&src, &config(out.clone())).unwrap_err();
    assert_eq!(err.exit_code(), 81);
    // --force overwrites.
    let mut cfg = config(out.clone());
    cfg.force = true;
    run(&src, &cfg).unwrap();
}

#[test]
fn dry_run_strict_reports_the_plan_and_carries_the_verdict() {
    // CON-1: `--dry-run --strict` does NOT abort; it carries the would-be
    // verdict so the plan is still reported (the wrapper then exits 84).
    let tmp = tempfile::tempdir().unwrap();
    let src = cc_plugin_fixture(tmp.path()); // has monitors/ (strict-blocking)
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();
    let mut cfg = config(out.clone());
    cfg.strict = true;
    cfg.dry_run = true;

    let outcome = run(&src, &cfg).unwrap();
    assert!(
        outcome.strict_blocked.is_some(),
        "carries the would-be strict verdict"
    );
    assert!(
        !outcome.written.is_empty(),
        "still reports the planned files"
    );
    assert!(
        !out.join("demo-tome").exists(),
        "dry-run writes nothing even under --strict"
    );
}

#[test]
fn marketplace_with_a_traversal_plugin_name_is_refused_with_no_escape() {
    // SEC-1: a vendored plugin whose own plugin.json `name` is a traversal
    // payload must be refused, with nothing written anywhere.
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("mkt");
    fs::create_dir_all(src.join(".claude-plugin")).unwrap();
    fs::write(
        src.join(".claude-plugin/marketplace.json"),
        br#"{"name":"m","owner":{"name":"o","email":"o@x.io"},"plugins":[{"name":"evil","source":"./evil"}]}"#,
    )
    .unwrap();
    fs::create_dir_all(src.join("evil/.claude-plugin")).unwrap();
    fs::write(
        src.join("evil/.claude-plugin/plugin.json"),
        br#"{"name":"../../escaped","version":"1.0.0"}"#,
    )
    .unwrap();
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();

    let err = run(&src, &catalog_config(out.clone(), false)).unwrap_err();
    assert_eq!(err.exit_code(), 7);
    assert!(!out.join("m-tome").exists());
    assert!(
        !tmp.path().join("escaped").exists(),
        "no traversal write outside the output dir"
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_skill_child_aborts_the_convert() {
    // A symlinked entry inside the plugin tree is refused at list time and
    // aborts the whole convert (fail-closed), emitting nothing.
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(src.join(".claude-plugin")).unwrap();
    fs::write(
        src.join(".claude-plugin/plugin.json"),
        br#"{"name":"p","version":"1.0.0"}"#,
    )
    .unwrap();
    fs::create_dir(src.join("skills")).unwrap();
    let outside = tmp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, src.join("skills/evil")).unwrap();
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();

    let err = run(&src, &config(out.clone())).unwrap_err();
    assert_eq!(err.exit_code(), 7);
    assert!(!out.join("p-tome").exists());
}

// --- security regression tests (code-review fixes) -------------------------

#[test]
fn remote_fetch_rejects_disallowed_url_schemes() {
    // The argument-injection guard: an `ext::` transport (or any non-allowlisted
    // scheme) must be refused BEFORE any git spawn, degrading to fetch-failed.
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("market");
    fs::create_dir_all(src.join(".claude-plugin")).unwrap();
    fs::write(
        src.join(".claude-plugin/marketplace.json"),
        br#"{"name":"m","version":"1.0.0","description":"d",
             "owner":{"name":"o","email":"o@x.io"},
             "plugins":[{"name":"evil","source":{"source":"url","url":"ext::sh -c id"}}]}"#,
    )
    .unwrap();
    let out = tmp.path().join("out");
    let outcome = run(&src, &catalog_config(out.clone(), false)).expect("convert proceeds");
    assert!(outcome.report.diagnostics.iter().any(|d| {
        d.rule_id == "convert/remote-plugin-fetch-failed"
            && d.message.contains("unsupported remote URL scheme")
    }));
}

#[test]
fn remote_fetch_skips_a_plugin_with_an_unsafe_marketplace_name() {
    // I1 regression: one hostile entry name must not poison the catalog.
    tome::authoring::import::claude_code::ALLOW_FILE_URLS_FOR_TESTS
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let tmp = tempfile::tempdir().unwrap();
    let good = remote_plugin_repo(tmp.path(), "good-one");
    let evil = remote_plugin_repo(tmp.path(), "evil-src");
    let src = tmp.path().join("market");
    fs::create_dir_all(src.join(".claude-plugin")).unwrap();
    fs::write(
        src.join(".claude-plugin/marketplace.json"),
        format!(
            r#"{{"name":"m","version":"1.0.0","description":"d",
                 "owner":{{"name":"o","email":"o@x.io"}},
                 "plugins":[
                   {{"name":"good-one","source":{{"source":"url","url":"{good}"}}}},
                   {{"name":"../escape","source":{{"source":"url","url":"{evil}"}}}}
                 ]}}"#
        ),
    )
    .unwrap();
    let out = tmp.path().join("out");
    let outcome = run(&src, &catalog_config(out.clone(), false)).expect("convert proceeds");
    let root = out.join(&outcome.final_name);
    assert!(
        root.join("good-one/tome-plugin.toml").is_file(),
        "good plugin still vendors"
    );
    assert!(outcome.report.diagnostics.iter().any(|d| {
        d.rule_id == "convert/remote-plugin-fetch-failed" && d.message.contains("unsafe name")
    }));
    assert!(!tmp.path().join("escape").exists());
}

#[test]
fn remote_fetch_honours_a_ref_pin_and_degrades_on_a_missing_ref() {
    tome::authoring::import::claude_code::ALLOW_FILE_URLS_FOR_TESTS
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let tmp = tempfile::tempdir().unwrap();
    let url = remote_plugin_repo(tmp.path(), "pinned");
    // Tag the current state, then move main past it with a marker file inside
    // the skill dir (collect_supporting copies all non-SKILL.md files there).
    let repo = tmp.path().join("pinned");
    git_cmd(&["tag", "v1"], &repo);
    fs::write(repo.join("skills/hello/AFTER_TAG.md"), b"post-tag marker").unwrap();
    git_cmd(&["add", "-A"], &repo);
    git_cmd(&["commit", "-q", "-m", "after tag"], &repo);

    let src = tmp.path().join("market");
    fs::create_dir_all(src.join(".claude-plugin")).unwrap();
    fs::write(
        src.join(".claude-plugin/marketplace.json"),
        format!(
            r#"{{"name":"m","version":"1.0.0","description":"d",
                 "owner":{{"name":"o","email":"o@x.io"}},
                 "plugins":[
                   {{"name":"pinned","source":{{"source":"url","url":"{url}","ref":"v1"}}}},
                   {{"name":"missing-ref","source":{{"source":"url","url":"{url}","ref":"no-such-ref"}}}}
                 ]}}"#
        ),
    )
    .unwrap();
    let out = tmp.path().join("out");
    let outcome = run(&src, &catalog_config(out.clone(), false)).expect("convert proceeds");
    let root = out.join(&outcome.final_name);
    // The v1 pin vendored the PRE-tag tree (no AFTER_TAG.md in the skill dir).
    assert!(root.join("pinned/tome-plugin.toml").is_file());
    assert!(
        !root.join("pinned/skills/hello/AFTER_TAG.md").exists(),
        "v1 pin must not include the post-tag supporting file"
    );
    // A missing ref fails the clone and degrades to the warning.
    assert!(outcome.report.diagnostics.iter().any(|d| {
        d.rule_id == "convert/remote-plugin-fetch-failed" && d.message.contains("missing-ref")
    }));
}

// ---------------------------------------------------------------------------
// FR #298 — the post-convert "what next" bridge is rendered on the HUMAN emit
// path only. These drive the real binary (the bridge lives in the command
// layer, not `authoring::convert::run`), asserting the `Next:` line's presence,
// its `<level> lint <target> --autofix` + `harness use` content, and its
// suppression under `--dry-run` and `--json`.
// ---------------------------------------------------------------------------

/// Run `tome plugin convert <src>` with the given extra args in an isolated
/// `$HOME`, returning `(stdout, stderr, success)`.
fn run_plugin_convert(src: &Path, extra: &[&str]) -> (String, String, bool) {
    let home = tempfile::tempdir().unwrap();
    let mut args = vec!["plugin", "convert", src.to_str().unwrap()];
    args.extend_from_slice(extra);
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_tome"))
        .args(&args)
        .env("HOME", home.path())
        .env("TOME_TELEMETRY", "0")
        .env_remove("TOME_LOG")
        .env_remove("RUST_LOG")
        .output()
        .expect("spawn tome");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

#[test]
fn human_convert_with_warnings_prints_the_next_bridge() {
    let tmp = tempfile::tempdir().unwrap();
    let src = cc_plugin_fixture(tmp.path());
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();

    // `--output <out>` lands the copy at `<out>/demo-tome`; the fixture carries
    // a `monitors/` unsupported component + a tool restriction ⇒ warnings.
    let target = out.join("demo-tome");
    let (stdout, stderr, ok) = run_plugin_convert(&src, &["--output", out.to_str().unwrap()]);
    assert!(ok, "convert succeeds; stderr: {stderr}");

    // The bridge points into the iteration loop with the real convert level and
    // the actual output target, then `harness use`.
    let expected = format!(
        "Next: run `tome plugin lint {} --autofix`, then `tome harness use <harness>`",
        target.display()
    );
    assert!(
        stdout.contains(&expected),
        "human output should carry the Next bridge.\nexpected substring: {expected}\ngot:\n{stdout}"
    );
    assert!(stdout.contains("Done:"), "summary still present:\n{stdout}");
}

#[test]
fn dry_run_convert_prints_no_bridge() {
    let tmp = tempfile::tempdir().unwrap();
    let src = cc_plugin_fixture(tmp.path());
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();

    let (stdout, stderr, ok) =
        run_plugin_convert(&src, &["--output", out.to_str().unwrap(), "--dry-run"]);
    assert!(ok, "dry-run succeeds; stderr: {stderr}");
    assert!(
        stdout.contains("Dry run:"),
        "dry-run summary present:\n{stdout}"
    );
    assert!(
        !stdout.contains("Next:"),
        "a --dry-run wrote nothing, so no lint bridge:\n{stdout}"
    );
}

#[test]
fn json_convert_has_no_bridge_in_the_stream() {
    let tmp = tempfile::tempdir().unwrap();
    let src = cc_plugin_fixture(tmp.path());
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();

    let (stdout, stderr, ok) =
        run_plugin_convert(&src, &["--output", out.to_str().unwrap(), "--json"]);
    assert!(ok, "json convert succeeds; stderr: {stderr}");
    // The JSONL stream is machine wire only — the bridge is human-mode text.
    assert!(
        !stdout.contains("Next:"),
        "the --json stream must not carry the human bridge:\n{stdout}"
    );
    // Sanity: it IS the JSON result stream (unchanged wire shape).
    assert!(
        stdout.contains("\"type\":\"result\"") || stdout.contains("\"type\": \"result\""),
        "json result line present:\n{stdout}"
    );
}

#[test]
fn json_convert_diagnostic_lines_carry_lint_finding_fields() {
    // Issue #299: convert's `--json` diagnostic lines now carry `file`/`line`/
    // `autofixable` with the SAME field names + value semantics as `lint --json`
    // findings — so a caller parsing lint findings can parse convert diagnostic
    // lines the same way. The JSONL envelope is preserved: per-diagnostic lines
    // followed by the trailing `type: "result"` line.
    let tmp = tempfile::tempdir().unwrap();
    // The CC fixture ships a `monitors/` unsupported component + a tool
    // restriction ⇒ at least one diagnostic in the stream.
    let src = cc_plugin_fixture(tmp.path());
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();

    let (stdout, stderr, ok) =
        run_plugin_convert(&src, &["--output", out.to_str().unwrap(), "--json"]);
    assert!(ok, "json convert succeeds; stderr: {stderr}");

    // Each non-empty line is one JSON object; classify by `type`.
    let mut diagnostic_lines = 0usize;
    let mut result_lines = 0usize;
    for line in stdout.lines().filter(|l| !l.trim().is_empty()) {
        let v: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("non-JSON line `{line}`: {e}"));
        match v["type"].as_str() {
            Some("diagnostic") => {
                diagnostic_lines += 1;
                // The enriched finding fields are present with lint's semantics.
                assert!(v.get("rule").is_some(), "diagnostic has `rule`: {line}");
                assert!(
                    v.get("severity").is_some(),
                    "diagnostic has `severity`: {line}"
                );
                assert!(
                    v.get("message").is_some(),
                    "diagnostic has `message`: {line}"
                );
                // #299 additions: present (a value or JSON null), not missing.
                assert!(
                    v.as_object().unwrap().contains_key("file"),
                    "diagnostic carries `file` (issue #299): {line}"
                );
                assert!(
                    v.as_object().unwrap().contains_key("line"),
                    "diagnostic carries `line` (issue #299): {line}"
                );
                assert!(
                    v["autofixable"].is_boolean(),
                    "diagnostic carries a boolean `autofixable` (issue #299): {line}"
                );
            }
            Some("result") => result_lines += 1,
            other => panic!("unexpected JSONL `type` {other:?} on line: {line}"),
        }
    }
    assert!(
        diagnostic_lines >= 1,
        "the fixture converts with at least one diagnostic:\n{stdout}"
    );
    assert_eq!(
        result_lines, 1,
        "exactly one trailing `type: \"result\"` line (envelope preserved):\n{stdout}"
    );
}
