//! Phase 6 / US3 — Phase 4 rules-file correction (FR-020/021/022, T083).
//!
//! Claude Code does not natively read `AGENTS.md`. The Phase 4 candidate
//! ladder put `AGENTS.md` first, so a project's `tome:begin/end` rules block
//! could land where Claude Code would never see it. Phase 6 corrects the
//! `claude_code` candidate set to `CLAUDE.md` > `.claude/CLAUDE.md`, with
//! `AGENTS.md` removed entirely.
//!
//! These tests drive the real `claude-code` / `codex` harness modules through
//! `sync_project` to confirm:
//!
//! 1. The rules-include block lands in `CLAUDE.md` (NOT `AGENTS.md`) for
//!    claude-code.
//! 2. An `AGENTS.md`-only project keeps a single block on `AGENTS.md` for the
//!    other harnesses (codex), and claude-code still writes `CLAUDE.md`.
//! 3. Both blocks resolve the SAME `.tome/RULES.md` via `@`-includes.

use std::path::PathBuf;

use crate::common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};
use tempfile::TempDir;
use tome::harness::sync::{self, SyncDeps};
use tome::workspace::WorkspaceName;

struct Fixture {
    _home: TempDir,
    paths: tome::paths::Paths,
    project: PathBuf,
    workspace: WorkspaceName,
}

impl Fixture {
    fn build(workspace_name: &str, harnesses_toml: &str) -> Self {
        let env = ToolEnv::new();
        let paths = paths_for(&env);
        std::fs::create_dir_all(&paths.root).expect("create tome root");
        seed_workspace(&paths, workspace_name);
        let workspace = WorkspaceName::parse(workspace_name).expect("parse workspace");

        let project = env.home_path().join("project");
        std::fs::create_dir_all(&project).expect("create project");
        let marker_dir = project.join(".tome");
        std::fs::create_dir_all(&marker_dir).expect("create marker dir");
        std::fs::write(
            marker_dir.join("config.toml"),
            format!("workspace = \"{workspace_name}\"\n{harnesses_toml}\n"),
        )
        .expect("write marker");
        // A project RULES.md so the @-include resolves to a real file.
        std::fs::write(marker_dir.join("RULES.md"), "# project rules\n").expect("write rules");

        Fixture {
            _home: env.home,
            paths,
            project,
            workspace,
        }
    }

    fn deps(&self) -> SyncDeps<'_> {
        SyncDeps {
            paths: &self.paths,
            home_root: self._home.path(),
            workspace_name: &self.workspace,
            force: false,
        }
    }
}

#[test]
fn rules_block_lands_in_claude_md_not_agents_md() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard =
        HarnessModulesGuard::install(vec![Box::new(tome::harness::claude_code::CLAUDE_CODE)]);

    let fx = Fixture::build("test-ws", "harnesses = [\"claude-code\"]");
    sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    // CLAUDE.md carries the block; AGENTS.md is never created.
    let claude_md = fx.project.join("CLAUDE.md");
    assert!(claude_md.is_file(), "claude-code must write CLAUDE.md");
    let body = std::fs::read_to_string(&claude_md).unwrap();
    assert!(
        body.contains("<!-- tome:begin -->") && body.contains("<!-- tome:end -->"),
        "CLAUDE.md must carry the rules block; got: {body}"
    );
    assert!(
        body.contains("@.tome/RULES.md"),
        "block must @-include the marker RULES.md; got: {body}"
    );
    assert!(
        !fx.project.join("AGENTS.md").exists(),
        "AGENTS.md MUST NOT be created for claude-code (Phase 6 correction)",
    );
}

#[test]
fn agents_only_project_keeps_one_block_on_agents_for_other_harness() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // claude-code + codex both enabled. Codex sinks to AGENTS.md; claude-code
    // sinks to CLAUDE.md. A pre-existing AGENTS.md must NOT pull claude-code
    // back onto it (the correction removed AGENTS.md from its candidate set).
    let _guard = HarnessModulesGuard::install(vec![
        Box::new(tome::harness::claude_code::CLAUDE_CODE),
        Box::new(tome::harness::codex::CODEX),
    ]);

    let fx = Fixture::build("test-ws", "harnesses = [\"claude-code\", \"codex\"]");
    // Pre-create an AGENTS.md (developer-authored, the "AGENTS.md-only"
    // project shape from the contract).
    std::fs::write(fx.project.join("AGENTS.md"), "# existing agents rules\n").unwrap();

    sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    // AGENTS.md carries EXACTLY ONE rules block (codex's). claude-code did
    // not also write here.
    let agents_body = std::fs::read_to_string(fx.project.join("AGENTS.md")).unwrap();
    assert_eq!(
        agents_body.matches("<!-- tome:begin -->").count(),
        1,
        "AGENTS.md must carry exactly one block (codex's); got: {agents_body}"
    );
    assert!(
        agents_body.contains("# existing agents rules"),
        "developer prose preserved; got: {agents_body}"
    );

    // claude-code wrote its own CLAUDE.md block.
    let claude_md = fx.project.join("CLAUDE.md");
    assert!(claude_md.is_file(), "claude-code wrote CLAUDE.md");
    let claude_body = std::fs::read_to_string(&claude_md).unwrap();
    assert!(
        claude_body.contains("<!-- tome:begin -->"),
        "CLAUDE.md carries the block; got: {claude_body}"
    );

    // Both blocks resolve the SAME .tome/RULES.md (two small include
    // directives, no duplicated rules content — NFR-009).
    assert!(
        agents_body.contains("@.tome/RULES.md"),
        "AGENTS.md @-includes the marker RULES.md; got: {agents_body}"
    );
    assert!(
        claude_body.contains("@.tome/RULES.md"),
        "CLAUDE.md @-includes the SAME marker RULES.md; got: {claude_body}"
    );
}
