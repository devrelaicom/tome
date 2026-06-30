//! I-D — symlink refusal coverage for the dispatch hook-file and manifest paths.
//!
//! Design §15 requires exit-44 symlink refusal on:
//!   (a) the Cursor dispatch hook-file sink (`.cursor/hooks.json`)
//!   (b) the Tome-owned per-(workspace, harness) manifest path
//!
//! The existing unit test (`command_hook_symlink_refusal_is_exit_44` in
//! `src/harness/reconcile/hooks.rs`) covers only the session-steering
//! `DevinHooksV1` path via the low-level `merge_command_hook` function.
//! These tests exercise the dispatch-hook-file and manifest guards end-to-end
//! through the real `sync_project` path, verifying that `HookSettingsWriteFailed`
//! (exit 44) is returned to the caller rather than written through the symlink.

#[cfg(unix)]
mod symlink_tests {
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;

    use crate::common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};
    use tempfile::TempDir;
    use tome::error::TomeError;
    use tome::harness::sync::{self, SyncDeps};
    use tome::workspace::WorkspaceName;

    struct Fixture {
        _home: TempDir,
        paths: tome::paths::Paths,
        project: PathBuf,
        workspace: WorkspaceName,
    }

    impl Fixture {
        fn build(workspace_name: &str) -> Self {
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
                format!("workspace = \"{workspace_name}\"\nharnesses = [\"cursor\"]\n"),
            )
            .expect("write marker");
            std::fs::write(marker_dir.join("RULES.md"), "ROUTING DIRECTIVE BODY\n")
                .expect("write rules");

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
                only_harness: None,
            }
        }
    }

    /// Seed a plugin with a PreToolUse command hook in its `hooks/hooks.json`.
    fn seed_hooks_source(paths: &tome::paths::Paths, plugin: &str, body: &str) -> String {
        let url = format!("https://example.test/{plugin}.git");
        let hooks_dir = paths.cache_dir_for(&url).join(plugin).join("hooks");
        std::fs::create_dir_all(&hooks_dir).expect("create hooks source dir");
        std::fs::write(hooks_dir.join("hooks.json"), body).expect("write source hooks.json");
        url
    }

    fn insert_enabled_skill_row(
        paths: &tome::paths::Paths,
        workspace: &str,
        catalog: &str,
        plugin: &str,
    ) {
        let conn = rusqlite::Connection::open(&paths.index_db).expect("open rw");
        conn.execute(
            "INSERT INTO skills
                (catalog, plugin, name, kind, description, plugin_version,
                 path, content_hash, searchable, user_invocable, when_to_use, indexed_at)
             VALUES (?1, ?2, 'demo', 'skill', 'd', '0.0.0',
                     'skills/demo/SKILL.md', 'h', 1, 0, NULL, '1970-01-01T00:00:00Z')",
            rusqlite::params![catalog, plugin],
        )
        .expect("insert skill row");
        let skill_id: i64 = conn
            .query_row(
                "SELECT id FROM skills WHERE catalog=?1 AND plugin=?2 AND kind='skill'",
                rusqlite::params![catalog, plugin],
                |r| r.get(0),
            )
            .expect("skill id");
        let ws_id: i64 = conn
            .query_row(
                "SELECT id FROM workspaces WHERE name = ?1",
                rusqlite::params![workspace],
                |r| r.get(0),
            )
            .expect("ws id");
        conn.execute(
            "INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at) VALUES (?1, ?2, 0)",
            rusqlite::params![ws_id, skill_id],
        )
        .expect("enrol skill");
    }

    const HOOKS_JSON: &str = r#"{
        "PreToolUse": [
            { "matcher": "Bash",
              "hooks": [ { "type": "command", "command": "/opt/guard.sh check" } ] }
        ]
    }"#;

    /// (a) Cursor dispatch sink: when `.cursor/hooks.json` is itself a symlink
    /// (with `.cursor` being a real directory so guardrails can write there
    /// without interference), `sync_project` must return `HookSettingsWriteFailed`
    /// (exit 44), NOT write through the symlink.
    ///
    /// Why symlink the file, not the directory: symlinking `.cursor` would also
    /// cause `reconcile_guardrails` (which writes `.cursor/rules/TOME_SKILLS.md`)
    /// to fail with `TomeError::Io` (exit 7), and that path propagates via `?`
    /// before the dispatch error is inspected. By making `.cursor` a real directory
    /// and symlinking only the final node `.cursor/hooks.json`, the guardrails
    /// writer sees a normal directory tree (`.cursor/rules/` doesn't exist yet →
    /// no symlink → permit) while the dispatch writer correctly refuses the
    /// symlinked `hooks.json`.
    ///
    /// This exercises `reconcile_dispatch_hook_file` → `refuse_symlinked_component`
    /// on `.cursor/hooks.json`. The guard is at line 1388 of
    /// `src/harness/reconcile/hooks.rs` — previously exercised only for the
    /// `DevinHooksV1` path via `merge_command_hook`.
    #[test]
    fn cursor_dispatch_hook_file_symlink_is_refused_exit_44() {
        let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::cursor::Cursor)]);

        let fx = Fixture::build("sym-ws");

        let url = seed_hooks_source(&fx.paths, "plugin-a", HOOKS_JSON);
        let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
        tome::index::workspace_catalogs::insert(&conn, "sym-ws", "cat", &url, "main")
            .expect("enrol catalog");
        drop(conn);
        insert_enabled_skill_row(&fx.paths, "sym-ws", "cat", "plugin-a");

        // Make `.cursor` a real directory (so guardrails writing
        // `.cursor/rules/TOME_SKILLS.md` is unaffected), then symlink ONLY the
        // final node `.cursor/hooks.json` to a sibling file.
        let cursor_dir = fx.project.join(".cursor");
        std::fs::create_dir_all(&cursor_dir).expect("create .cursor dir");
        let decoy = fx.project.join(".cursor-hooks-decoy.json");
        std::fs::write(&decoy, b"{}").expect("create decoy file");
        let hooks_json = cursor_dir.join("hooks.json");
        symlink(&decoy, &hooks_json).expect("symlink .cursor/hooks.json → decoy");

        let result = sync::sync_project(&fx.project, &fx.deps());

        let err = result.expect_err("sync must fail when .cursor/hooks.json is a symlink");
        assert_eq!(
            err.exit_code(),
            44,
            "symlink refusal for .cursor/hooks.json must be exit 44; got: {err:?}"
        );
        assert!(
            matches!(err, TomeError::HookSettingsWriteFailed { .. }),
            "error must be HookSettingsWriteFailed; got: {err:?}"
        );
        // The decoy must NOT have been written through.
        let decoy_content = std::fs::read_to_string(&decoy).expect("read decoy");
        assert_eq!(
            decoy_content, "{}",
            ".cursor/hooks.json symlink target must not be overwritten; got: {decoy_content:?}"
        );
    }

    /// (b) Manifest write: when the manifest directory's parent has a symlinked
    /// component, `sync_project` must return `HookSettingsWriteFailed` (exit 44)
    /// rather than writing the manifest through the symlink.
    ///
    /// This exercises `reconcile_dispatch_manifest` → `write_manifest` →
    /// `write_hook_file` → `refuse_symlinked_component` for the Tome-owned
    /// per-(workspace, harness) manifest path (inside `~/.tome/`).
    #[test]
    fn manifest_path_symlink_is_refused_exit_44() {
        let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::cursor::Cursor)]);

        let fx = Fixture::build("mani-ws");

        let url = seed_hooks_source(&fx.paths, "plugin-b", HOOKS_JSON);
        let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
        tome::index::workspace_catalogs::insert(&conn, "mani-ws", "cat", &url, "main")
            .expect("enrol catalog");
        drop(conn);
        insert_enabled_skill_row(&fx.paths, "mani-ws", "cat", "plugin-b");

        // Determine where the manifest for (mani-ws, cursor) will be written and
        // plant a symlink at its parent directory so the component check fires.
        let workspace = WorkspaceName::parse("mani-ws").unwrap();
        let manifest_path = fx.paths.hooks_manifest(&workspace, "cursor");
        let manifest_dir = manifest_path.parent().expect("manifest has parent");

        // Create the grandparent so we can symlink the parent.
        std::fs::create_dir_all(manifest_dir.parent().expect("manifest dir has grandparent"))
            .expect("create grandparent");
        let real_manifest_dir = manifest_dir.parent().unwrap().join(format!(
            "{}-real",
            manifest_dir.file_name().unwrap().to_string_lossy()
        ));
        std::fs::create_dir_all(&real_manifest_dir).expect("create real manifest dir");
        symlink(&real_manifest_dir, manifest_dir).expect("plant manifest dir symlink");

        let result = sync::sync_project(&fx.project, &fx.deps());

        let err = result.expect_err("sync must fail when manifest dir is a symlink");
        assert_eq!(
            err.exit_code(),
            44,
            "symlink refusal for manifest path must be exit 44; got: {err:?}"
        );
        assert!(
            matches!(err, TomeError::HookSettingsWriteFailed { .. }),
            "error must be HookSettingsWriteFailed; got: {err:?}"
        );
        // The manifest must NOT have been written through the symlink.
        let real_has_manifest = std::fs::read_dir(&real_manifest_dir)
            .expect("read real manifest dir")
            .any(|e| {
                e.ok()
                    .and_then(|e| e.file_name().to_str().map(|s| s.contains("cursor")))
                    .unwrap_or(false)
            });
        assert!(
            !real_has_manifest,
            "manifest must NOT have been written through symlink to real dir"
        );
    }
}
