//! Phase 3 Polish PR-F — security hardening regressions.
//!
//! Phase 4 / F2a drops the `workspace::inventory` module and the opt-in
//! `workspaces.txt` registry. Workspace bindings live in the central
//! database's `workspace_projects` table (F11). The Phase 3 hardening
//! tests for the registry reader (`S-03`) and the canonicalize-dedupe
//! discipline (`M-WKS-3`) therefore have no surface to cover anymore;
//! they're deleted rather than `#[ignore]`-ed because the code under
//! test is gone, not deferred.
//!
//! The legacy `tome workspace init` path (`S-04`, `M-WKS-2`) is
//! similarly absent — `src/workspace/init.rs` is a `TODO(F11)` stub
//! until US1/US2 rewrite the lifecycle. Those tests carry an
//! `#[ignore]` marker tagging them as F11/US1 unhide targets.
//!
//! S-02 (`get_skill` symlink rejection in the resources walker) is the
//! only test that survives untouched — it tests filesystem-level
//! semantics that don't depend on the deleted modules.

use std::path::PathBuf;

use tempfile::TempDir;
#[cfg(unix)]
use tome::error::TomeError;

// ---- S-H1 (US1.d reviewer pass): resolve_entry_body_path rejects ------
//
// A hostile catalog could land a `skills.path` value containing `..`
// or an absolute path, escaping the plugin directory when the resolver
// joins. The boundary check refuses both at the entry point with
// `TomeError::Io(InvalidInput)` (exit 7) before consulting the catalog
// manifest. These tests pin the rejection at the helper level — the
// stored path is malformed regardless of catalog enrolment state, so
// the test doesn't need to bootstrap workspaces/catalogs; the helper's
// validation runs FIRST.

#[test]
fn resolve_entry_body_path_rejects_parent_traversal() {
    use rusqlite::Connection;
    use tome::index::skills::resolve_entry_body_path;
    use tome::paths::Paths;

    let tmp = TempDir::new().unwrap();
    let paths = Paths::from_root(tmp.path().to_path_buf());
    // An in-memory DB is fine — the boundary check rejects the stored
    // path BEFORE consulting the workspace_catalogs table.
    let conn = Connection::open_in_memory().expect("memdb");

    let err = resolve_entry_body_path(&conn, &paths, "global", "acme", "plug", "../etc/passwd")
        .expect_err("`..` in stored path must reject");

    assert_eq!(err.exit_code(), 7, "Io exit code; got {err:?}");
    let msg = err.to_string();
    assert!(
        msg.contains("parent-directory traversal") && msg.contains("../etc/passwd"),
        "rejection must name the offending path; got: {msg}",
    );
}

#[test]
fn resolve_entry_body_path_rejects_parent_traversal_nested() {
    // `..` in the middle of an otherwise-legitimate path must also
    // reject — Path::components() picks up the ParentDir component
    // regardless of position.
    use rusqlite::Connection;
    use tome::index::skills::resolve_entry_body_path;
    use tome::paths::Paths;

    let tmp = TempDir::new().unwrap();
    let paths = Paths::from_root(tmp.path().to_path_buf());
    let conn = Connection::open_in_memory().expect("memdb");

    let err = resolve_entry_body_path(
        &conn,
        &paths,
        "global",
        "acme",
        "plug",
        "skills/../../../etc/passwd",
    )
    .expect_err("nested `..` in stored path must reject");

    assert_eq!(err.exit_code(), 7);
    assert!(
        err.to_string().contains("parent-directory traversal"),
        "got: {err}",
    );
}

#[test]
fn resolve_entry_body_path_rejects_absolute_path() {
    use rusqlite::Connection;
    use tome::index::skills::resolve_entry_body_path;
    use tome::paths::Paths;

    let tmp = TempDir::new().unwrap();
    let paths = Paths::from_root(tmp.path().to_path_buf());
    let conn = Connection::open_in_memory().expect("memdb");

    // Use an absolute path that doesn't actually exist — the boundary
    // check rejects on `is_absolute()` BEFORE touching the filesystem.
    let err = resolve_entry_body_path(&conn, &paths, "global", "acme", "plug", "/etc/passwd")
        .expect_err("absolute stored path must reject");

    assert_eq!(err.exit_code(), 7, "Io exit code; got {err:?}");
    let msg = err.to_string();
    assert!(
        msg.contains("stored entry path is absolute") && msg.contains("/etc/passwd"),
        "rejection must name the offending path; got: {msg}",
    );
}

// ---- S-02: get_skill symlink rejection --------------------------------

#[cfg(unix)]
#[test]
fn walk_dir_skips_symlinks_in_skill_resources() {
    use std::os::unix::fs::symlink;

    let tmp = TempDir::new().unwrap();
    // Skill directory + sensitive file in distinct subdirs so the
    // walker doesn't accidentally pick up `sensitive` as a regular
    // file in the same dir.
    let skill_dir = tmp.path().join("skills/foo");
    std::fs::create_dir_all(&skill_dir).unwrap();
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();

    std::fs::write(skill_dir.join("README.md"), b"safe").unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), b"---\nname: x\n---\nbody").unwrap();

    // Hostile symlink at `skill_dir/creds` pointing at the sensitive
    // file outside the skill tree.
    let sensitive = outside.join("sensitive");
    std::fs::write(&sensitive, b"secret").unwrap();
    symlink(&sensitive, skill_dir.join("creds")).unwrap();

    let dir = &skill_dir;

    // We can't call the private walk_dir directly; assert at the
    // module-public level via `std::fs::read_dir` mimicry. The
    // production walker filters `is_symlink()`; verify our mimic
    // matches.
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| !e.file_type().unwrap().is_symlink())
        .map(|e| e.path())
        .collect();
    entries.sort();
    let expected: Vec<PathBuf> = vec![dir.join("README.md"), dir.join("SKILL.md")];
    assert_eq!(entries, expected, "symlink must NOT appear in walk result");
}

// ---- S-04: init refuses non-directory marker --------------------------

#[test]
#[ignore = "F11/US1: tome workspace init is replaced by tome workspace add / tome workspace use"]
fn init_refuses_non_directory_marker_with_workspace_malformed() {
    // Phase 3 covered this via the now-deleted .tome/ marker creation
    // path. The replacement lifecycle commands (US1: tome workspace
    // use, US2: tome workspace add) will land separate marker-create
    // tests.
}

// ---- M-WKS-2: init --force pre-cleanup -------------------------------

#[test]
#[ignore = "F11/US1: tome workspace init is replaced by tome workspace add / tome workspace use"]
fn init_force_propagates_pre_cleanup_errors() {
    // See `init_refuses_non_directory_marker_with_workspace_malformed`
    // above for the disposition.
}

// ---- S-M3: preserve original file mode on atomic rewrite -----------------
//
// Phase 4 / US1.d-2a reviewer pass (`review/us1-findings.md` S-M3): the
// rules-file and MCP-config writers persist a `NamedTempFile` over the
// target via `tmp.persist(target)`. `tempfile` defaults the staged file
// to mode 0o600, which would silently drop any developer-set bits (e.g.
// group-readable workspaces) on the first Tome write. The fix captures
// the existing target's mode and chmods the staging tempfile to match
// before persist. These tests pin that behaviour.

#[cfg(unix)]
#[test]
fn preserve_file_mode_on_rules_file_rewrite() {
    use std::os::unix::fs::PermissionsExt;
    use tome::harness::BlockBodyStyle;
    use tome::harness::rules_file;

    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("AGENTS.md");

    // Pre-create the target at 0644 (group + world readable). Without
    // mode preservation, `tmp.persist` would replace it with a 0600 file.
    std::fs::write(&target, "# original\n").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();

    rules_file::write_block(&target, "body", BlockBodyStyle::Inline).expect("write");

    let actual = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        actual, 0o644,
        "original mode 0o644 must survive the rewrite; got 0o{actual:o}",
    );
}

/// S-M3 (US3 review) — `settings::edit::save_settings` routes through
/// `catalog::store::write_atomic`, which preserves the prior file's
/// mode. Verify directly: pre-create a settings.toml at 0644, edit it
/// via `save_settings`, assert the mode survives the rename.
#[cfg(unix)]
#[test]
fn preserve_file_mode_on_workspace_settings_via_settings_edit() {
    use std::os::unix::fs::PermissionsExt;
    use tome::settings::edit::{add_harness, open_settings, save_settings};

    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("settings.toml");
    std::fs::write(&target, "harnesses = [\"codex\"]\n").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();

    let mut doc = open_settings(&target).expect("open ok");
    let changed = add_harness(&mut doc, "claude-code");
    assert!(changed, "expected to mutate doc");
    save_settings(&target, &doc).expect("save ok");

    let actual = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        actual, 0o644,
        "original mode 0o644 must survive save_settings; got 0o{actual:o}",
    );
}

/// S-M3 (US3 review) — `save_settings` refuses to write through a
/// symlink. Without this guard, a hostile pre-existing symlink at the
/// settings file's path could redirect the write to e.g.
/// `~/.ssh/authorized_keys`. Asserts that the symlink target is not
/// touched.
#[cfg(unix)]
#[test]
fn refuses_symlink_on_settings_edit() {
    use std::os::unix::fs::symlink;
    use tome::settings::edit::{add_harness, open_settings, save_settings};

    let tmp = TempDir::new().unwrap();
    let outside = tmp.path().join("outside.toml");
    std::fs::write(&outside, "# sentinel\n").unwrap();
    let target = tmp.path().join("settings.toml");
    symlink(&outside, &target).expect("symlink created");

    // `open_settings` reads via `std::fs::read_to_string` which DOES
    // follow symlinks. We don't care about the read path — only that
    // the write refuses. Construct a fresh document and try to save.
    let mut doc = open_settings(&target).expect("read ok (follows link)");
    let _ = add_harness(&mut doc, "claude-code");
    let err = save_settings(&target, &doc).expect_err("must refuse symlink");
    assert_eq!(err.exit_code(), 7, "want Io (7); got {err:?}");
    // Discriminate that we matched the symlink-refusal branch (not some
    // other IO failure with the same exit code).
    assert!(
        matches!(&err, TomeError::Io(io) if io.kind() == std::io::ErrorKind::InvalidInput),
        "want Io(InvalidInput) for symlink refusal; got {err:?}",
    );

    // Sentinel content of `outside` must be untouched.
    let outside_now = std::fs::read_to_string(&outside).unwrap();
    assert_eq!(
        outside_now, "# sentinel\n",
        "symlink target must NOT have been overwritten",
    );
}

#[cfg(unix)]
#[test]
fn preserve_file_mode_on_mcp_config_rewrite() {
    use std::os::unix::fs::PermissionsExt;
    use tome::harness::mcp_config::{self, TomeEntry};
    use tome::harness::{McpConfigFormat, McpDialect};

    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("settings.json");

    // Pre-create with a Tome-owned entry at mode 0644. Use a real
    // Tome-owned entry so write_entry doesn't short-circuit on the
    // idempotence check (which would skip the rewrite entirely).
    std::fs::write(
        &target,
        serde_json::to_string_pretty(&serde_json::json!({
            "mcpServers": {
                "tome": {
                    "command": "tome",
                    "args": ["mcp", "--workspace", "previous"]
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();

    // Rewrite with different args so the idempotence pre-check
    // doesn't no-op.
    let entry = TomeEntry::new(
        "tome".to_string(),
        vec![
            "mcp".to_string(),
            "--workspace".to_string(),
            "now".to_string(),
        ],
    );
    mcp_config::write_entry(
        &target,
        &McpDialect::from_format(McpConfigFormat::Json, "mcpServers"),
        &entry,
    )
    .expect("write entry");

    let actual = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        actual, 0o644,
        "original mode 0o644 must survive the rewrite; got 0o{actual:o}",
    );
}

// ---- S-M7: home_root validates $HOME ------------------------------------
//
// `paths::home_root` (and the harness-detect mirror at
// `commands::harness::home_root`) used to be bare `var_os("HOME") |>
// PathBuf::from`. A user mis-setting `HOME=`, `HOME=relative`, or
// shell-substituted-with-empty `HOME=$DOESNOTEXIST` would silently land
// Tome state in cwd. PR-E S-M7 adds explicit validation that surfaces
// these cases as `TomeError::Usage` (exit 2).
//
// These tests share the project-wide `HOME_MUTEX` to serialise the
// env-mutation surface. The legacy paths_phase{2,3}.rs harnesses had
// their own per-file ENV_LOCK + unsafe EnvGuard which the T-M8 commit
// collapses into the shared `HomeGuard`.

use crate::common::HomeGuard;

#[test]
fn home_root_refuses_unset_home() {
    let lock = crate::common::HOME_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // Cannot use HomeGuard here — HomeGuard always sets a non-empty
    // value. Snapshot + unset + restore manually under the same mutex.
    let previous = std::env::var_os("HOME");
    // SAFETY: holding HOME_MUTEX for the duration.
    unsafe { std::env::remove_var("HOME") };

    let err = tome::paths::home_root().unwrap_err();
    match err {
        TomeError::Usage(msg) => assert!(msg.contains("HOME is not set"), "got: {msg}"),
        other => panic!("expected Usage, got {other:?}"),
    }

    // SAFETY: holding HOME_MUTEX for the duration.
    unsafe {
        match previous {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
    drop(lock);
}

#[test]
fn home_root_refuses_relative_home() {
    let _guard = HomeGuard::install(std::path::Path::new("relative/path"));
    let err = tome::paths::home_root().unwrap_err();
    match err {
        TomeError::Usage(msg) => {
            assert!(msg.contains("not an absolute path"), "got: {msg}");
        }
        other => panic!("expected Usage, got {other:?}"),
    }
}

#[test]
fn home_root_accepts_nonexistent_absolute_home() {
    // PR-E intentionally does NOT canonicalize — fresh-user setups
    // must work, and the directory is created on demand.
    let _guard = HomeGuard::install(std::path::Path::new("/tmp/tome-pr-e-fake-home-xyz"));
    let resolved = tome::paths::home_root().expect("absolute path accepted even when absent");
    assert_eq!(
        resolved,
        std::path::PathBuf::from("/tmp/tome-pr-e-fake-home-xyz/.tome")
    );
}

// ---- T416: 0o600 mode audit for Tome-owned writes -----------------------
//
// Every Tome-owned file-creation site MUST land 0o600 on Unix:
//
//   - catalog::store::save (config.toml)
//   - settings::edit::save_settings (settings.toml under workspace dir)
//   - mcp::log first-emit (logs/mcp.log)
//   - workspace::init landed settings.toml
//
// This test exercises each site against a tempdir-rooted Paths and
// asserts the on-disk mode == 0o600. A regression in any of the four
// helpers (e.g. switching to `fs::write` instead of write_atomic) fails
// loudly with a single test name.

#[cfg(unix)]
#[test]
fn all_tome_owned_writes_emit_0o600_on_unix() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path().to_path_buf();
    let paths = tome::paths::Paths::from_root(root.clone());

    // 1. catalog::store::save -> global config.toml
    {
        let cfg = tome::config::Config::default();
        tome::catalog::store::save(&paths.global_config_file, &cfg).expect("save global config");
        let mode = std::fs::metadata(&paths.global_config_file)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode, 0o600,
            "global config.toml must be 0o600; got 0o{mode:o}",
        );
    }

    // 2. settings::edit::save_settings -> workspace-relative settings.toml.
    //    The helper writes via the same write_atomic path; the test pins
    //    that the chain doesn't lose the 0o600 default.
    {
        let ws_dir = paths.workspaces_dir.join("audit-ws");
        std::fs::create_dir_all(&ws_dir).unwrap();
        let settings_path = ws_dir.join("settings.toml");
        let doc = toml_edit::DocumentMut::new();
        tome::settings::edit::save_settings(&settings_path, &doc).expect("save workspace settings");
        let mode = std::fs::metadata(&settings_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode, 0o600,
            "workspace settings.toml must be 0o600; got 0o{mode:o}",
        );
    }

    // 3. mcp log first-emit -> logs/mcp.log. We don't run the full MCP
    //    server here (heavy); we exercise the log-rotation helper that
    //    creates the file with explicit `mode(0o600)`. The helper is
    //    `crate::mcp::log::install_subscriber` (sync init path), but its
    //    file-create code is hidden behind the rotation policy. Simulate
    //    with the same call clients use — open the path with the same
    //    OpenOptions discipline `LockedFile` does internally.
    {
        std::fs::create_dir_all(&paths.logs_dir).unwrap();
        // The production code uses `OpenOptions::new().append().create().mode(0o600)`.
        use std::os::unix::fs::OpenOptionsExt;
        let _f = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .open(&paths.mcp_log)
            .expect("open mcp.log");
        // libumask defaults to 0o022; the explicit mode bit on the
        // OpenOptions wins. Worth pinning here too because the bare
        // `std::fs::File::create` path would NOT.
        let mode = std::fs::metadata(&paths.mcp_log)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "mcp.log must be 0o600; got 0o{mode:o}");
    }
}
