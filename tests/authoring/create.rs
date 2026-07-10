//! End-to-end `create` tests (US4): scaffold each artifact level → emit to disk
//! → parse back → lint, asserting the contract's "a freshly-created artifact
//! MUST pass lint with zero findings" invariant, plus the skill naming rules
//! (`<plugin>:<name>`), `--bare`, `name == dir`, and symlink refusal.

use std::fs;
use std::path::Path;

use serde_json::Value as JsonValue;
use tome::authoring::detect::ArtifactLevel;
use tome::authoring::emit::{EmitOptions, emit};
use tome::authoring::lint::parse::parse_artifact;
use tome::authoring::lint::{rules, run};
use tome::authoring::scaffold::{CreateParams, ScaffoldComponent, create_artifact};

fn params(name: &str) -> CreateParams {
    CreateParams {
        name: name.to_owned(),
        plugin_name: None,
        description: None,
        author_name: None,
        date: "2026-06-08".to_owned(),
        bare: false,
        component: ScaffoldComponent::Skill,
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

// G9 — scaffold new component kinds.

#[test]
fn command_scaffold_lints_clean() {
    // `plugin create my-plugin --kind command` → plugin with commands/my-plugin.md.
    let tmp = tempfile::tempdir().unwrap();
    let mut p = params("my-plugin");
    p.component = ScaffoldComponent::Command;
    let root = scaffold_to_disk(tmp.path(), ArtifactLevel::Plugin, &p);
    assert!(root.join("tome-plugin.toml").is_file(), "manifest present");
    assert!(
        root.join("commands/my-plugin.md").is_file(),
        "command file present"
    );
    assert!(
        !root.join("skills").exists(),
        "no skills/ dir for a command scaffold"
    );
    assert_lints_clean(&root);
}

#[test]
fn agent_scaffold_lints_clean() {
    // `plugin create my-agent --kind agent` → plugin with agents/my-agent.md.
    let tmp = tempfile::tempdir().unwrap();
    let mut p = params("my-agent");
    p.component = ScaffoldComponent::Agent;
    let root = scaffold_to_disk(tmp.path(), ArtifactLevel::Plugin, &p);
    assert!(root.join("tome-plugin.toml").is_file(), "manifest present");
    assert!(
        root.join("agents/my-agent.md").is_file(),
        "agent file present"
    );
    assert!(
        !root.join("skills").exists(),
        "no skills/ dir for an agent scaffold"
    );
    assert_lints_clean(&root);
}

#[test]
fn hooks_scaffold_emits_hooks_json_and_script() {
    // `plugin create my-hooks --kind hooks` → plugin with hooks/hooks.json +
    // hooks/on-start.sh.
    let tmp = tempfile::tempdir().unwrap();
    let mut p = params("my-hooks");
    p.component = ScaffoldComponent::Hooks;
    let root = scaffold_to_disk(tmp.path(), ArtifactLevel::Plugin, &p);
    assert!(root.join("tome-plugin.toml").is_file(), "manifest present");
    assert!(
        root.join("hooks/hooks.json").is_file(),
        "hooks.json present"
    );
    assert!(
        root.join("hooks/on-start.sh").is_file(),
        "on-start.sh present"
    );
    // The hooks.json must be valid JSON in the event-map form (top-level keys are
    // event names, not a "hooks" wrapper) and reference the TOME_PLUGIN_ROOT token.
    let hooks_content = fs::read_to_string(root.join("hooks/hooks.json")).unwrap();
    let parsed: JsonValue =
        serde_json::from_str(&hooks_content).expect("hooks/hooks.json must be valid JSON");
    assert!(
        parsed.is_object() && parsed.get("hooks").is_none(),
        "hooks.json must use the event-map form (no top-level 'hooks' wrapper): {hooks_content}"
    );
    assert!(
        hooks_content.contains("${TOME_PLUGIN_ROOT}"),
        "hooks.json must reference TOME_PLUGIN_ROOT: {hooks_content}"
    );
    assert_lints_clean(&root);
}

#[test]
fn mcp_scaffold_emits_mcp_json() {
    // `plugin create my-server --kind mcp` → plugin with .mcp.json.
    let tmp = tempfile::tempdir().unwrap();
    let mut p = params("my-server");
    p.component = ScaffoldComponent::Mcp;
    let root = scaffold_to_disk(tmp.path(), ArtifactLevel::Plugin, &p);
    assert!(root.join("tome-plugin.toml").is_file(), "manifest present");
    assert!(root.join(".mcp.json").is_file(), ".mcp.json present");

    // The .mcp.json must be valid JSON with an mcpServers top-level key.
    let mcp_content = fs::read_to_string(root.join(".mcp.json")).unwrap();
    let parsed: JsonValue =
        serde_json::from_str(&mcp_content).expect(".mcp.json must be valid JSON");
    assert!(
        parsed.get("mcpServers").is_some(),
        ".mcp.json must have a top-level 'mcpServers' key"
    );
    // The server name must match the plugin name.
    assert!(
        parsed["mcpServers"].get("my-server").is_some(),
        ".mcp.json server name must match the plugin name: {mcp_content}"
    );
    assert_lints_clean(&root);
}

#[test]
fn clap_parses_plugin_create_kind_flag() {
    // `tome plugin create my-server --kind mcp` must parse successfully and
    // produce a `ScaffoldKindArg::Mcp` on `PluginCreateArgs.kind`.
    use clap::Parser;
    use tome::cli::{Cli, Command, PluginCommand, ScaffoldKindArg};

    for (kind_str, expected) in [
        ("skill", ScaffoldKindArg::Skill),
        ("command", ScaffoldKindArg::Command),
        ("agent", ScaffoldKindArg::Agent),
        ("hooks", ScaffoldKindArg::Hooks),
        ("mcp", ScaffoldKindArg::Mcp),
    ] {
        let cli =
            Cli::try_parse_from(["tome", "plugin", "create", "my-plugin", "--kind", kind_str])
                .unwrap_or_else(|e| panic!("parse failed for --kind {kind_str}: {e}"));
        match cli.command {
            Command::Plugin(pa) => match pa.command {
                Some(PluginCommand::Create(args)) => {
                    assert_eq!(
                        args.kind,
                        Some(expected),
                        "--kind {kind_str} must parse to {expected:?}"
                    );
                }
                other => panic!("expected plugin create, got {other:?}"),
            },
            other => panic!("expected plugin command, got {other:?}"),
        }
    }

    // Omitting --kind produces None (the default).
    let cli = Cli::try_parse_from(["tome", "plugin", "create", "my-plugin"]).unwrap();
    match cli.command {
        Command::Plugin(pa) => match pa.command {
            Some(PluginCommand::Create(args)) => {
                assert_eq!(args.kind, None, "omitting --kind must yield None");
            }
            other => panic!("expected plugin create, got {other:?}"),
        },
        other => panic!("expected plugin command, got {other:?}"),
    }
}
