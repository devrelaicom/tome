//! End-to-end test for the `tome workspace use` bind + sync flow against
//! the real `claude-code` harness module (Phase 4 / US1.c — T158).
//!
//! Library-API only — the CLI binary would load real ONNX models, which
//! is unnecessary here. We drive
//! [`tome::workspace::binding::bind_project`] +
//! [`tome::commands::harness::sync_for_project_root`] (the same seams the
//! CLI wrapper calls) against a `TempDir`-rooted home and project, with
//! global `settings.toml` declaring `harnesses = ["claude-code"]`.
//!
//! The real `SUPPORTED_HARNESSES` registry drives the dispatch. The
//! other four registered harnesses (`codex`, `cursor`, `gemini`,
//! `opencode`) appear in the registry but are NOT in the effective list
//! — sync runs the cleanup branch for each, which is a no-op against
//! absent paths. Their MCP config targets live under `<home>` / `<project>`
//! and both are isolated `TempDir`s, so even an unintended write would
//! stay sealed inside the test.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::common::{lifecycle_paths, seed_workspace};
use serde_json::Value;
use tempfile::TempDir;
use tome::commands::harness::sync_for_project_root;
use tome::harness::sync::{self, SyncDeps};
use tome::workspace::WorkspaceName;
use tome::workspace::binding::{self, BindDeps};

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

/// Live state shared between the bind + sync calls and the assertions.
struct Fixture {
    /// `TempDir` rooted at `<home>` — Tome's per-user state lives at
    /// `<home>/.tome/`, claude_code's per-user dir would live at
    /// `<home>/.claude/`.
    _home: TempDir,
    /// `TempDir` rooted at the project the test binds to. Distinct from
    /// `_home` so we can assert that home-targeted writes (none in this
    /// test) stay out of the project tree and vice-versa.
    _project: TempDir,
    paths: tome::paths::Paths,
    home_path: PathBuf,
    project_path: PathBuf,
    workspace: WorkspaceName,
}

impl Fixture {
    /// Build a single-workspace fixture with the global settings.toml
    /// declaring `harnesses = ["claude-code"]`. The workspace's
    /// `RULES.md` is seeded so the bind step copies it into the project
    /// marker; sync then computes the `@`-include path against it.
    fn build(workspace_name: &str) -> Self {
        Self::build_with_existing(workspace_name, &[])
    }

    /// Variant that pre-creates files under the project root before
    /// binding — used to verify "preserve surrounding content" behaviour
    /// for AGENTS.md / `.mcp.json` / `.claude/settings.json` (legacy).
    fn build_with_existing(workspace_name: &str, files: &[(&str, &str)]) -> Self {
        let home = TempDir::new().expect("home tempdir");
        let project = TempDir::new().expect("project tempdir");
        let home_path = home.path().to_path_buf();
        let project_path = project.path().to_path_buf();

        let paths = lifecycle_paths(&home_path.join(".tome"));
        fs::create_dir_all(&paths.root).expect("create tome root");

        // Task 2: global harness settings now live in config.toml [harness].enabled.
        fs::write(
            &paths.global_config_file,
            "[harness]\nenabled = [\"claude-code\"]\n",
        )
        .expect("write global config");

        // Seed the workspace row.
        seed_workspace(&paths, workspace_name);
        let workspace = WorkspaceName::parse(workspace_name).expect("parse workspace name");

        // Workspace-layer RULES.md (the @-include target).
        let workspace_dir = paths.workspace_dir(&workspace);
        fs::create_dir_all(&workspace_dir).expect("create workspace dir");
        fs::write(
            workspace_dir.join("RULES.md"),
            "# Test rules\n\nHello from workspace.\n",
        )
        .expect("write workspace RULES.md");

        // Pre-existing project files (used for surrounding-content tests).
        for (rel, contents) in files {
            let path = project_path.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent");
            }
            fs::write(&path, contents).expect("write pre-existing file");
        }

        Fixture {
            _home: home,
            _project: project,
            paths,
            home_path,
            project_path,
            workspace,
        }
    }

    fn bind_deps(&self) -> BindDeps<'_> {
        BindDeps {
            paths: &self.paths,
            home_root: &self.home_path,
        }
    }

    fn sync_deps(&self, force: bool) -> SyncDeps<'_> {
        SyncDeps {
            paths: &self.paths,
            home_root: &self.home_path,
            workspace_name: &self.workspace,
            force,
            only_harness: None,
            dry_run: false,
        }
    }

    /// Convenience: run the bind + sync sequence exactly like the CLI
    /// wrapper at `commands::workspace::use_::run`.
    fn bind_then_sync(&self) {
        let outcome = binding::bind_project(
            &self.project_path,
            self.workspace.clone(),
            /* force */ false,
            &self.bind_deps(),
        )
        .expect("bind_project");
        sync_for_project_root(
            &outcome.project_root,
            &outcome.workspace,
            &self.bind_deps(),
            false,
        )
        .expect("sync_for_project_root");
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn mtime(path: &Path) -> SystemTime {
    fs::metadata(path)
        .unwrap_or_else(|e| panic!("stat {}: {e}", path.display()))
        .modified()
        .expect("modified time")
}

fn read_json(path: &Path) -> Value {
    let body = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&body)
        .unwrap_or_else(|e| panic!("parse JSON at {}: {e}\nbody:\n{body}", path.display()))
}

// ---------------------------------------------------------------------------
// 1. Happy path: bind + sync writes the marker, CLAUDE.md block, and
//    .claude/settings.json entry. (Phase 6 correction: claude-code's rules
//    sink is CLAUDE.md, not AGENTS.md.)
// ---------------------------------------------------------------------------

#[test]
fn bind_then_sync_writes_claude_code_artefacts() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let fx = Fixture::build("test-ws");
    fx.bind_then_sync();

    // 1. Project marker landed.
    let marker_config = fx.project_path.join(".tome/config.toml");
    let marker_body = fs::read_to_string(&marker_config).unwrap();
    assert!(
        marker_body.contains("workspace = \"test-ws\""),
        "marker config.toml must name the workspace; got: {marker_body}"
    );

    // 2. Workspace RULES.md was copied into the marker.
    let marker_rules = fx.project_path.join(".tome/RULES.md");
    let rules_body = fs::read_to_string(&marker_rules).unwrap();
    assert!(
        rules_body.contains("Hello from workspace"),
        "marker RULES.md must mirror the workspace RULES.md; got: {rules_body}"
    );

    // 3. CLAUDE.md exists and carries the AtInclude block (Phase 6
    //    correction — never AGENTS.md for claude-code).
    let claude_md = fx.project_path.join("CLAUDE.md");
    let claude_body = fs::read_to_string(&claude_md).unwrap();
    assert!(
        claude_body.contains("<!-- tome:begin -->"),
        "CLAUDE.md must carry begin marker; got: {claude_body}"
    );
    assert!(
        claude_body.contains("<!-- tome:end -->"),
        "CLAUDE.md must carry end marker; got: {claude_body}"
    );
    assert!(
        claude_body.contains("@.tome/RULES.md"),
        "CLAUDE.md block must @-include the marker RULES.md; got: {claude_body}"
    );
    assert!(
        !fx.project_path.join("AGENTS.md").exists(),
        "AGENTS.md must NOT be created for claude-code (Phase 6 correction)"
    );

    // 4. .mcp.json at project root carries the canonical Tome MCP entry
    //    (issue #496: Claude Code 2.x reads .mcp.json, not .claude/settings.json).
    let settings = fx.project_path.join(".mcp.json");
    let parsed = read_json(&settings);
    let tome_entry = parsed
        .get("mcpServers")
        .and_then(|v| v.get("tome"))
        .expect("mcpServers.tome must exist");
    // #337: command is the resolved absolute launcher, recognised by basename /
    // self-equality (not the bare `tome`, which a PATH-less host can't start).
    assert!(
        tome::harness::launcher::looks_like_tome_launcher(tome_entry["command"].as_str().unwrap()),
        "command must be a recognised Tome launcher; got {}",
        tome_entry["command"],
    );
    let args = tome_entry["args"]
        .as_array()
        .expect("args must be an array");
    // Phase 9 / US3 (FR-030): the synced args now also carry the host-harness
    // stamp `--harness <name>` (a later arg — the `args[0]=="mcp"` ownership
    // marker is preserved).
    assert_eq!(
        args.len(),
        5,
        "args = ['mcp', '--workspace', '<name>', '--harness', '<harness>']"
    );
    assert_eq!(args[0], "mcp");
    assert_eq!(args[1], "--workspace");
    assert_eq!(args[2], "test-ws");
    assert_eq!(args[3], "--harness");
    assert_eq!(args[4], "claude-code");
}

// ---------------------------------------------------------------------------
// 2. Rebind: switching to a different workspace updates the --workspace arg
//    and leaves the CLAUDE.md block body unchanged (path is unchanged).
// ---------------------------------------------------------------------------

#[test]
fn rebind_to_different_workspace_updates_mcp_args() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut fx = Fixture::build("test-ws");
    fx.bind_then_sync();

    // Seed a second workspace + flip the binding.
    seed_workspace(&fx.paths, "test-ws-2");
    let ws2 = WorkspaceName::parse("test-ws-2").unwrap();
    let ws2_dir = fx.paths.workspace_dir(&ws2);
    fs::create_dir_all(&ws2_dir).unwrap();
    fs::write(ws2_dir.join("RULES.md"), "# second\n").unwrap();

    fx.workspace = ws2.clone();

    let outcome = binding::bind_project(&fx.project_path, ws2.clone(), false, &fx.bind_deps())
        .expect("rebind");
    assert!(
        outcome.rebind_from.is_some(),
        "rebind_from must be set on a workspace flip"
    );
    sync_for_project_root(
        &outcome.project_root,
        &outcome.workspace,
        &fx.bind_deps(),
        false,
    )
    .expect("sync after rebind");

    // MCP args reflect the new workspace.
    let settings = fx.project_path.join(".mcp.json");
    let parsed = read_json(&settings);
    let args = parsed["mcpServers"]["tome"]["args"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(
        args[2], "test-ws-2",
        "args must reference the new workspace"
    );

    // CLAUDE.md block body unchanged — the @-include path doesn't depend
    // on the workspace name.
    let claude_body = fs::read_to_string(fx.project_path.join("CLAUDE.md")).unwrap();
    assert!(claude_body.contains("@.tome/RULES.md"));
}

// ---------------------------------------------------------------------------
// 3. Pre-existing CLAUDE.md: original content preserved verbatim, block
//    appended. (Phase 6 correction: claude-code's rules sink is CLAUDE.md.)
// ---------------------------------------------------------------------------

#[test]
fn existing_claude_md_preserves_surrounding_content() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let original = "# My rules\n\nSee below.\n";
    let fx = Fixture::build_with_existing("test-ws", &[("CLAUDE.md", original)]);
    fx.bind_then_sync();

    let body = fs::read_to_string(fx.project_path.join("CLAUDE.md")).unwrap();
    assert!(
        body.contains("# My rules"),
        "original heading must survive; got: {body}"
    );
    assert!(
        body.contains("See below."),
        "original prose must survive; got: {body}"
    );
    assert!(
        body.contains("<!-- tome:begin -->") && body.contains("<!-- tome:end -->"),
        "Tome block must be appended; got: {body}"
    );

    // The block must come after the original prose (block append, not prepend).
    let block_start = body.find("<!-- tome:begin -->").unwrap();
    let prose_idx = body.find("See below.").unwrap();
    assert!(
        prose_idx < block_start,
        "original prose must precede the Tome block; body: {body}"
    );
}

// ---------------------------------------------------------------------------
// 4. Pre-existing .mcp.json: other MCP entries preserved, Tome entry
//    added; insertion order preserved. (Issue #496: MCP config moved from
//    .claude/settings.json to .mcp.json.)
// ---------------------------------------------------------------------------

#[test]
fn existing_mcp_json_preserves_other_entries() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let prior = r#"{
  "mcpServers": {
    "other": {
      "command": "elsewhere",
      "args": []
    }
  }
}"#;
    let fx = Fixture::build_with_existing("test-ws", &[(".mcp.json", prior)]);
    fx.bind_then_sync();

    let mcp_json = fx.project_path.join(".mcp.json");
    let parsed = read_json(&mcp_json);
    let servers = parsed.get("mcpServers").expect("mcpServers");

    // The user-owned `other` entry must survive.
    let other = servers.get("other").expect("other must survive");
    assert_eq!(other["command"], "elsewhere");

    // The Tome entry must have been inserted alongside it.
    let tome = servers.get("tome").expect("tome must be inserted");
    // #337: command is the resolved launcher, recognised by basename / self-eq.
    assert!(
        tome::harness::launcher::looks_like_tome_launcher(tome["command"].as_str().unwrap()),
        "command must be a recognised Tome launcher; got {}",
        tome["command"],
    );

    // Order: `preserve_order` should keep `other` first, `tome` appended.
    let object = servers.as_object().expect("mcpServers is an object");
    let keys: Vec<&String> = object.keys().collect();
    assert_eq!(
        keys,
        vec![&"other".to_string(), &"tome".to_string()],
        "preserve_order: existing key first, Tome appended; got {keys:?}"
    );
}

// ---------------------------------------------------------------------------
// 4b. Legacy migration: a pre-#496 .claude/settings.json Tome entry is
//     cleaned up on the first sync under Tome ≥ next; non-Tome entries in
//     that file are left alone.
// ---------------------------------------------------------------------------

#[test]
fn legacy_claude_settings_json_tome_entry_removed_on_sync() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // Simulate what Tome ≤ 0.7.16 wrote: a Tome-owned entry inside
    // .claude/settings.json alongside a user-owned `other` entry.
    let legacy_settings = r#"{
  "mcpServers": {
    "other": {
      "command": "elsewhere",
      "args": []
    },
    "tome": {
      "command": "tome",
      "args": ["mcp", "--workspace", "test-ws", "--harness", "claude-code"]
    }
  }
}"#;
    let fx = Fixture::build_with_existing("test-ws", &[(".claude/settings.json", legacy_settings)]);
    fx.bind_then_sync();

    // The Tome entry must now live in .mcp.json.
    let mcp_json = fx.project_path.join(".mcp.json");
    let parsed = read_json(&mcp_json);
    assert!(
        tome::harness::launcher::looks_like_tome_launcher(
            parsed["mcpServers"]["tome"]["command"].as_str().unwrap()
        ),
        ".mcp.json must contain the Tome entry after migration",
    );

    // The legacy .claude/settings.json Tome entry must have been cleaned up;
    // the non-Tome `other` entry is preserved.
    let old_settings = fx.project_path.join(".claude/settings.json");
    let old_parsed = read_json(&old_settings);
    assert!(
        old_parsed["mcpServers"]["tome"].is_null(),
        "legacy .claude/settings.json must not contain the Tome entry after migration; \
         got: {}",
        old_parsed,
    );
    assert_eq!(
        old_parsed["mcpServers"]["other"]["command"], "elsewhere",
        "non-Tome entries in .claude/settings.json must survive migration",
    );
}

// ---------------------------------------------------------------------------
// 5. Idempotent re-sync: a second sync (no bind) leaves CLAUDE.md and
//    .mcp.json mtimes unchanged (FR-525). (Issue #496: was .claude/settings.json)
// ---------------------------------------------------------------------------

#[test]
fn idempotent_resync_no_disk_changes() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let fx = Fixture::build("test-ws");
    fx.bind_then_sync();

    let claude_md = fx.project_path.join("CLAUDE.md");
    let mcp_json = fx.project_path.join(".mcp.json");

    let agents_mtime_1 = mtime(&claude_md);
    let settings_mtime_1 = mtime(&mcp_json);

    // Wait long enough for mtime granularity (HFS+/APFS = 1s; ext4 = ms).
    std::thread::sleep(Duration::from_millis(1500));

    // Re-sync only — no bind. Same `SyncDeps`, same workspace.
    let outcome =
        sync::sync_project(&fx.project_path, &fx.sync_deps(false)).expect("re-sync must succeed");

    assert!(
        outcome.added.is_empty(),
        "no additions on idempotent re-sync; got {:?}",
        outcome.added
    );
    assert!(
        outcome.updated.is_empty(),
        "no updates on idempotent re-sync; got {:?}",
        outcome.updated
    );
    assert!(
        outcome.removed.is_empty(),
        "no removals on idempotent re-sync; got {:?}",
        outcome.removed
    );

    assert_eq!(
        mtime(&claude_md),
        agents_mtime_1,
        "CLAUDE.md mtime must not advance on idempotent re-sync"
    );
    assert_eq!(
        mtime(&mcp_json),
        settings_mtime_1,
        ".mcp.json mtime must not advance on idempotent re-sync"
    );
}

// ---------------------------------------------------------------------------
// 5b. Issue #496 migration + Phase 9 / US3 (FR-030): a pre-P9 Tome-owned
//     MCP entry that lived in the old .claude/settings.json location is
//     migrated to .mcp.json on the first sync. The canonical 5-element form
//     (with --harness stamp) is written to the new location and the legacy
//     entry is cleaned up. A subsequent re-sync is idempotent (LeftAlone).
// ---------------------------------------------------------------------------

#[test]
fn legacy_mcp_entry_migrates_to_new_path_then_idempotent() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // A pre-P9 Tome-owned entry in the OLD location: marker present, but no
    // `--harness` stamp AND stored in .claude/settings.json.
    let legacy = r#"{
  "mcpServers": {
    "tome": {
      "command": "tome",
      "args": ["mcp", "--workspace", "test-ws"]
    }
  }
}"#;
    let fx = Fixture::build_with_existing("test-ws", &[(".claude/settings.json", legacy)]);
    fx.bind_then_sync();

    // After migration: the canonical entry lives at .mcp.json.
    let mcp_json = fx.project_path.join(".mcp.json");
    let read_args = |path: &Path| -> Vec<String> {
        read_json(path)["mcpServers"]["tome"]["args"]
            .as_array()
            .expect("args array")
            .iter()
            .map(|a| a.as_str().expect("string arg").to_owned())
            .collect()
    };

    let args = read_args(&mcp_json);
    assert_eq!(
        args,
        vec!["mcp", "--workspace", "test-ws", "--harness", "claude-code"],
        "migrated entry must carry the canonical 5-element form",
    );
    assert_eq!(
        args.iter().filter(|a| *a == "--harness").count(),
        1,
        "exactly one --harness after migration",
    );
    assert_eq!(args[0], "mcp", "ownership marker preserved");

    // The legacy location must have had its Tome entry cleaned up.
    let old_settings = fx.project_path.join(".claude/settings.json");
    if old_settings.exists() {
        let old_parsed = read_json(&old_settings);
        assert!(
            old_parsed["mcpServers"]["tome"].is_null(),
            "legacy .claude/settings.json Tome entry must be gone after migration; \
             got: {}",
            old_parsed,
        );
    }

    // A subsequent re-sync (no bind) is idempotent — the migrated entry is
    // LeftAlone, the stamp not duplicated.
    let outcome =
        sync::sync_project(&fx.project_path, &fx.sync_deps(false)).expect("re-sync must succeed");
    assert!(
        outcome.updated.is_empty(),
        "migrated entry is LeftAlone on re-sync; got {:?}",
        outcome.updated,
    );
    assert_eq!(
        read_args(&mcp_json)
            .iter()
            .filter(|a| *a == "--harness")
            .count(),
        1,
        "still exactly one --harness after the idempotent re-sync",
    );
}

// ---------------------------------------------------------------------------
// 6. Nested CLAUDE.md (.claude/CLAUDE.md): the AtInclude path is computed
//    relative to the harness's rules-file target. With only
//    `.claude/CLAUDE.md` pre-existing, claude-code's corrected precedence
//    ladder (CLAUDE.md > .claude/CLAUDE.md) picks the nested file, and the
//    `@`-include must traverse up one level.
// ---------------------------------------------------------------------------

#[test]
fn nested_claude_md_uses_parent_path_at_include() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // Pre-create ONLY .claude/CLAUDE.md so the claude-code harness
    // resolves its target to that nested path.
    let fx = Fixture::build_with_existing("test-ws", &[(".claude/CLAUDE.md", "# nested rules\n")]);
    fx.bind_then_sync();

    // AGENTS.md and the top-level CLAUDE.md must NOT have been created
    // — the harness's precedence ladder picked the nested file.
    assert!(
        !fx.project_path.join("AGENTS.md").exists(),
        "AGENTS.md must not be created when .claude/CLAUDE.md already exists"
    );
    assert!(
        !fx.project_path.join("CLAUDE.md").exists(),
        "top-level CLAUDE.md must not be created when .claude/CLAUDE.md already exists"
    );

    // The nested file received the Tome block, and the `@`-include
    // walks up out of `.claude/` to reach `.tome/RULES.md`.
    let nested = fx.project_path.join(".claude/CLAUDE.md");
    let body = fs::read_to_string(&nested).expect("read nested CLAUDE.md");
    assert!(
        body.contains("<!-- tome:begin -->") && body.contains("<!-- tome:end -->"),
        ".claude/CLAUDE.md must carry the Tome block; got: {body}"
    );
    assert!(
        body.contains("@../.tome/RULES.md"),
        ".claude/CLAUDE.md block must @-include `../.tome/RULES.md`; got: {body}"
    );
    // The original prose must survive.
    assert!(
        body.contains("# nested rules"),
        "original heading must survive; got: {body}"
    );
}
