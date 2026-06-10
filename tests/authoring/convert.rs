//! End-to-end `convert` pipeline tests (US2): a Claude Code plugin fixture →
//! native Tome plugin on disk, verifying the manifest cutover, harness-ism
//! rewrite, supporting-file copy, unsupported-component warnings, rename,
//! `--dry-run` (zero writes), and `--strict` abort.

use std::fs;
use std::path::{Path, PathBuf};

use tome::authoring::convert::{ConvertConfig, run};
use tome::authoring::detect::ArtifactLevel;
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

fn skill_config(output_dir: PathBuf, from: Option<&str>) -> ConvertConfig {
    ConvertConfig {
        level: ArtifactLevel::Skill,
        from: from.map(str::to_owned),
        new_name: None,
        strict: false,
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

    let outcome = run(&src, &skill_config(out.clone(), Some("cline"))).unwrap();
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
    src
}

fn catalog_config(output_dir: PathBuf, strict: bool) -> ConvertConfig {
    ConvertConfig {
        level: ArtifactLevel::Catalog,
        from: None,
        new_name: None,
        strict,
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
        "hooks",
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
    // 7 component dirs + settings.json.
    assert_eq!(count, 8, "{:?}", outcome.report.diagnostics);
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
    ] {
        let a = fs::read(t1.join(f)).unwrap_or_else(|_| panic!("missing {f} in run1"));
        let b = fs::read(t2.join(f)).unwrap_or_else(|_| panic!("missing {f} in run2"));
        assert_eq!(a, b, "byte drift in {f}");
    }
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
