//! US3 — the MCP `meta` tool + the reserved `add-tome-conversion-skill` prompt,
//! driven through the in-process MCP harness (real `Server`, real routers).

mod common;

use common::mcp_harness::{McpHarness, StagedWorkspace, mcp_error_exit_code, mcp_error_slug};
use common::{HomeGuard, ToolEnv, paths_for};
use tempfile::TempDir;
use tome::authoring::meta as meta_skill;
use tome::mcp::prompts::PromptRegistry;
use tome::mcp::tools::meta;

fn install(skill_id: &str, scope: meta::Scope) -> meta::Input {
    meta::Input {
        action: meta::Action::Install,
        skill_id: skill_id.to_string(),
        scope,
    }
}

// --- the `meta` tool --------------------------------------------------------

#[test]
fn meta_tool_installs_to_stamped_host_harness() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    let _home = HomeGuard::install(env.home_path());
    // Server stamped with the host harness (as `harness sync` would).
    let harness = McpHarness::with_host(
        &paths,
        PromptRegistry::default(),
        Some("claude-code".to_string()),
        None,
    );

    let out = harness
        .call_meta(install("convert-marketplace", meta::Scope::Global))
        .expect("install");
    assert_eq!(out.skill_id, "convert-marketplace");
    assert_eq!(out.installed_at.harness, "claude-code");
    assert_eq!(out.installed_at.scope, "global");
    // The reported revision is the embedded skill's build-time revision, not
    // just some non-empty string (US3 closeout MINOR-4: pin the Output-surface
    // mapping end-to-end, not only `!is_empty()`).
    let embedded_rev = meta_skill::find("convert-marketplace")
        .expect("embedded skill present")
        .revision;
    assert_eq!(
        out.installed_at.revision, embedded_rev,
        "reported revision == the embedded build-time revision",
    );

    // Lands in the host harness's global skills dir and persists to disk.
    let skill_md = env
        .home_path()
        .join(".claude/skills/convert-marketplace/SKILL.md");
    assert!(
        skill_md.is_file(),
        "skill installed under the host harness dir"
    );
    let body = std::fs::read_to_string(&skill_md).unwrap();
    assert!(body.contains("tome_skill_revision"), "revision stamped");
    assert!(
        body.contains(embedded_rev),
        "the on-disk stamp carries the same revision that was reported",
    );
}

#[test]
fn meta_tool_fails_closed_when_host_unstamped() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    let _home = HomeGuard::install(env.home_path());
    // No host harness (legacy/unstamped config) → fail closed (FR-029).
    let harness = McpHarness::with_host(&paths, PromptRegistry::default(), None, None);

    let err = harness
        .call_meta(install("convert-marketplace", meta::Scope::Global))
        .expect_err("must fail closed without a host harness");
    assert_eq!(mcp_error_slug(&err), "no_harness_detected");
    assert_eq!(
        mcp_error_exit_code(&err),
        89,
        "slug maps to the CLI's exit 89"
    );
    // Nothing was written.
    assert!(!env.home_path().join(".claude/skills").exists());
}

#[test]
fn meta_tool_unknown_skill_is_meta_skill_not_found() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    let _home = HomeGuard::install(env.home_path());
    let harness = McpHarness::with_host(
        &paths,
        PromptRegistry::default(),
        Some("claude-code".to_string()),
        None,
    );

    let err = harness
        .call_meta(install("no-such-skill", meta::Scope::Global))
        .expect_err("unknown skill");
    assert_eq!(mcp_error_slug(&err), "meta_skill_not_found");
    assert_eq!(
        mcp_error_exit_code(&err),
        87,
        "slug maps to the CLI's exit 87"
    );
    // US3 closeout MINOR-3: the unknown-skill check is before any I/O, so
    // nothing is created — pin that ordering against a future reorder.
    assert!(
        !env.home_path().join(".claude/skills").exists(),
        "an unknown skill writes nothing",
    );
}

/// US3 closeout MAJOR-1(b): a stamped-but-unknown host harness name (e.g. a
/// typo or a future harness this binary does not know) fails closed exactly
/// like an unstamped host — `harness::lookup` returns None → exit 89, no write.
#[test]
fn meta_tool_unknown_host_harness_fails_closed() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    let _home = HomeGuard::install(env.home_path());
    let harness = McpHarness::with_host(
        &paths,
        PromptRegistry::default(),
        Some("bogus-harness".to_string()),
        None,
    );

    let err = harness
        .call_meta(install("convert-marketplace", meta::Scope::Global))
        .expect_err("an unknown host harness must fail closed");
    assert_eq!(mcp_error_slug(&err), "no_harness_detected");
    assert_eq!(mcp_error_exit_code(&err), 89);
    assert!(
        !env.home_path().join(".claude/skills").exists(),
        "nothing written for an unknown host harness",
    );
}

/// US3 closeout MAJOR-1(c): a KNOWN host harness that does not consume native
/// skills (Gemini — `supports_native_skills() == false`) also fails closed.
/// This is the realistic miss: stamping `Some("gemini")` is a valid config,
/// and the tool must refuse rather than mis-route to a non-existent sink.
#[test]
fn meta_tool_skill_incapable_host_fails_closed() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    let _home = HomeGuard::install(env.home_path());
    let harness = McpHarness::with_host(
        &paths,
        PromptRegistry::default(),
        Some("gemini".to_string()),
        None,
    );

    let err = harness
        .call_meta(install("convert-marketplace", meta::Scope::Global))
        .expect_err("a skill-incapable host must fail closed");
    assert_eq!(mcp_error_slug(&err), "no_harness_detected");
    assert_eq!(mcp_error_exit_code(&err), 89);
    // Gemini has no skills sink at any scope — nothing should be written
    // anywhere under home.
    assert!(
        !env.home_path().join(".gemini").exists()
            && !env.home_path().join(".claude/skills").exists(),
        "nothing written for a skill-incapable host",
    );
}

/// T-2: the meta tool's PROJECT scope (the contract default) lands under the
/// resolved project root, leaving the global home untouched.
#[test]
fn meta_tool_installs_project_scope_under_resolved_root() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    let _home = HomeGuard::install(env.home_path());
    let project = TempDir::new().unwrap();
    let harness = McpHarness::with_host(
        &paths,
        PromptRegistry::default(),
        Some("claude-code".to_string()),
        Some(project.path().to_path_buf()),
    );

    let out = harness
        .call_meta(install("convert-marketplace", meta::Scope::Project))
        .expect("project install");
    assert_eq!(out.installed_at.scope, "project");
    assert!(
        project
            .path()
            .join(".claude/skills/convert-marketplace/SKILL.md")
            .is_file(),
        "project-scope install lands under the resolved project root"
    );
    assert!(
        !env.home_path()
            .join(".claude/skills/convert-marketplace")
            .exists(),
        "global home untouched under project scope"
    );
}

/// T-5: the MCP install path inherits the symlink-safe fail-closed guard — a
/// symlinked skills-root component → `meta_install_failed` (88), no escape.
#[cfg(unix)]
#[test]
fn meta_tool_symlinked_target_is_meta_install_failed() {
    use std::os::unix::fs::symlink;
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    let base = env.home_path().canonicalize().unwrap();
    let _home = HomeGuard::install(&base);
    // Plant a symlinked `.claude` so the global skills root traverses a symlink.
    let outside = TempDir::new().unwrap();
    symlink(outside.path(), base.join(".claude")).unwrap();
    let harness = McpHarness::with_host(
        &paths,
        PromptRegistry::default(),
        Some("claude-code".to_string()),
        None,
    );

    let err = harness
        .call_meta(install("convert-marketplace", meta::Scope::Global))
        .expect_err("symlinked target must be refused");
    assert_eq!(mcp_error_slug(&err), "meta_install_failed");
    assert_eq!(mcp_error_exit_code(&err), 88);
    assert!(
        !outside.path().join("skills/convert-marketplace").exists(),
        "no write escaped through the symlink"
    );
}

// --- the reserved prompt ----------------------------------------------------

#[test]
fn reserved_prompt_is_registered_with_install_body() {
    let skill_body = "---\nname: some-skill\ndescription: a staged skill.\n---\n# body\n";
    let ws = StagedWorkspace::stage(&[("some-skill", skill_body)], &[]);
    let harness = ws.harness();

    assert!(
        harness
            .prompt_names()
            .iter()
            .any(|n| n == "add-tome-conversion-skill"),
        "reserved prompt registered: {:?}",
        harness.prompt_names()
    );

    let body = harness
        .prompts_get_text("add-tome-conversion-skill", None)
        .expect("get reserved prompt");
    assert!(body.contains("meta"), "drives the meta tool: {body}");
    assert!(body.contains("convert-marketplace"), "names the skill");
    assert!(body.to_lowercase().contains("install"));
}

#[test]
fn reserved_prompt_wins_collision_against_plugin_entry() {
    // A plugin command derives the same prompt name. The reserved built-in
    // (empty-seed identity) wins the base name; the command is counter-suffixed.
    let cmd_body = "---\nname: add-tome-conversion-skill\ndescription: a colliding command.\n---\nDo a thing.\n";
    let ws = StagedWorkspace::stage(&[], &[("add-tome-conversion-skill", cmd_body)]);
    let harness = ws.harness();
    let names = harness.prompt_names();

    assert!(
        names.iter().any(|n| n == "add-tome-conversion-skill"),
        "base name present: {names:?}"
    );
    // The base name resolves to the RESERVED meta body — not the command's —
    // proving the reservation held against the colliding plugin entry.
    let body = harness
        .prompts_get_text("add-tome-conversion-skill", None)
        .expect("get base name");
    assert!(
        body.contains("convert-marketplace") && body.contains("meta"),
        "base name kept by the reserved built-in, not the colliding command: {body}"
    );
    // Both the reserved prompt AND the (suffixed) command are advertised.
    assert!(
        names.len() >= 2,
        "the colliding command is still advertised under a counter-suffixed name: {names:?}"
    );
    // US3 closeout MINOR-1: the command was SUFFIXED (counter-bumped), not
    // dropped — the suffixed name must resolve to the command's OWN body, not
    // the reserved one. Find the non-base name and confirm it serves the
    // command's "Do a thing." body.
    let suffixed = names
        .iter()
        .find(|n| n.as_str() != "add-tome-conversion-skill")
        .expect("a counter-suffixed name exists");
    let suffixed_body = harness
        .prompts_get_text(suffixed, None)
        .expect("get suffixed command");
    assert!(
        suffixed_body.contains("Do a thing."),
        "the suffixed name serves the displaced command's own body, not the reserved one: {suffixed_body}"
    );
}
