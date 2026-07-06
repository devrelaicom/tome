//! #425 — `tome sync --dry-run`: full classification, zero filesystem
//! mutation.
//!
//! Drives the REAL command path (`tome::commands::sync::sync_one_project` with
//! `dry_run: true`), against the stub harness override, and proves two things:
//!
//! 1. **The target tree is byte-frozen** — every file's contents (and, on
//!    Unix, inode: the atomic writers replace-by-rename, so even a same-bytes
//!    rewrite would change it) is identical before and after the dry run.
//! 2. **The preview is the real classification** — the change set a dry run
//!    reports equals the change set the subsequent REAL run then performs,
//!    for both the write direction (fresh project) and the removal direction
//!    (harness dropped from the marker).
//!
//! The snapshot is scoped to the PROJECT tree: every stub-harness sink
//! (STUB_RULES.md, stub.mcp.json, guardrails, agents) and the `.tome/RULES.md`
//! rules half land there. The central DB is opened read-only by sync and is
//! not part of the assertion (SQLite may touch WAL bookkeeping on open).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::common::{
    HarnessModulesGuard, HomeGuard, ToolEnv, paths_for, seed_workspace, stub_embedder_seed,
    stub_reranker_seed, stub_summariser_seed,
};
use tempfile::TempDir;
use time::OffsetDateTime;
use tome::harness::StubHarness;
use tome::index::{self, OpenOptions};
use tome::paths::Paths;
use tome::workspace::WorkspaceName;

fn open_central(paths: &Paths) -> rusqlite::Connection {
    index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
            profile: None,
        },
    )
    .expect("open central DB")
}

/// Create `<root>/.tome/config.toml` declaring `harnesses` and insert the
/// `workspace_projects` binding row (the `plugin_sync_flag` fixture shape).
fn seed_bound_project(paths: &Paths, workspace_name: &str, project_root: &Path, harnesses: &str) {
    std::fs::create_dir_all(project_root.join(".tome")).expect("create .tome");
    std::fs::write(
        project_root.join(".tome").join("config.toml"),
        format!("workspace = \"{workspace_name}\"\nharnesses = {harnesses}\n"),
    )
    .expect("write project config.toml");
    let conn = open_central(paths);
    let workspace_id: i64 = conn
        .query_row(
            "SELECT id FROM workspaces WHERE name = ?1",
            rusqlite::params![workspace_name],
            |row| row.get(0),
        )
        .expect("lookup workspace_id");
    let now = OffsetDateTime::now_utc().unix_timestamp();
    conn.execute(
        "INSERT INTO workspace_projects (project_path, workspace_id, bound_at)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![
            project_root.to_string_lossy().to_string(),
            workspace_id,
            now
        ],
    )
    .expect("seed workspace_projects");
}

struct Fixture {
    home: TempDir,
    paths: Paths,
    project: PathBuf,
    workspace: WorkspaceName,
}

fn build(workspace_name: &str) -> Fixture {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).expect("create tome root");
    seed_workspace(&paths, workspace_name);
    // A central RULES.md so the rules half has bytes to propagate.
    let rules = paths.workspace_rules_file(&WorkspaceName::parse(workspace_name).unwrap());
    std::fs::create_dir_all(rules.parent().expect("rules file has a parent"))
        .expect("create workspace dir");
    std::fs::write(&rules, "workspace rules body\n").expect("write central RULES.md");
    let project = env.home_path().join("project");
    std::fs::create_dir_all(&project).expect("create project");
    seed_bound_project(&paths, workspace_name, &project, "[\"stub\"]");
    Fixture {
        home: env.home,
        paths,
        project,
        workspace: WorkspaceName::parse(workspace_name).expect("parse workspace"),
    }
}

/// One file's identity for the freeze assertion: contents plus (Unix) inode —
/// the atomic writers replace-by-rename, so even a same-bytes rewrite changes
/// the inode and fails the comparison.
#[derive(Debug, PartialEq, Eq)]
struct FileId {
    contents: Vec<u8>,
    #[cfg(unix)]
    inode: u64,
}

/// Snapshot every file under `root` (recursive), keyed by relative path.
fn snapshot_tree(root: &Path) -> BTreeMap<PathBuf, FileId> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, FileId>) {
        for entry in std::fs::read_dir(dir).expect("read_dir") {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                #[cfg(unix)]
                let inode = {
                    use std::os::unix::fs::MetadataExt;
                    entry.metadata().expect("metadata").ino()
                };
                out.insert(
                    path.strip_prefix(root).expect("under root").to_path_buf(),
                    FileId {
                        contents: std::fs::read(&path).expect("read file"),
                        #[cfg(unix)]
                        inode,
                    },
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn args(dry_run: bool) -> tome::cli::SyncArgs {
    tome::cli::SyncArgs {
        all: false,
        rules_only: false,
        harness_only: false,
        harness: vec![],
        dry_run,
    }
}

/// The `(op-ignored) harness + path` set of a project outcome, for comparing a
/// dry run's prediction against the real run's actions.
fn predicted(outcome: &tome::commands::sync::ProjectOutcome) -> Vec<(String, PathBuf)> {
    outcome
        .changes
        .iter()
        .map(|c| (c.harness.clone(), c.path.clone()))
        .collect()
}

#[test]
fn dry_run_freezes_the_tree_and_predicts_the_real_run() {
    // HARNESS_OVERRIDE_MUTEX before HOME_MUTEX (via HomeGuard) — the documented
    // lock order when a test needs both (`sync_one_project` reads $HOME).
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness::default())]);
    let fx = build("ws-dry");
    let _home = HomeGuard::install(fx.home.path());

    // ---- Phase 1: dry run against a fresh project — writes NOTHING. ----
    let before = snapshot_tree(&fx.project);
    let dry =
        tome::commands::sync::sync_one_project(&fx.workspace, &fx.project, &args(true), &fx.paths)
            .expect("dry run");

    // The full classification ran: the rules half WOULD sync, the harness half
    // WOULD create the stub's rules directive + MCP config.
    assert_eq!(dry.rules, Some("synced"), "{dry:?}");
    assert!(
        dry.harness_changes >= 2,
        "dry run must classify the would-be writes: {dry:?}",
    );
    assert_eq!(
        dry.changes.len(),
        dry.harness_changes,
        "one enumerated line per counted change: {dry:?}",
    );

    // Zero filesystem mutation: contents AND (unix) inodes identical.
    assert_eq!(
        before,
        snapshot_tree(&fx.project),
        "a dry run must not touch the project tree",
    );
    assert!(
        !fx.project.join("STUB_RULES.md").exists(),
        "dry run must not write the rules directive",
    );
    assert!(
        !fx.project.join("stub.mcp.json").exists(),
        "dry run must not write the MCP config",
    );

    // ---- Phase 2: the REAL run performs exactly what the dry run said. ----
    let real =
        tome::commands::sync::sync_one_project(&fx.workspace, &fx.project, &args(false), &fx.paths)
            .expect("real run");
    assert_eq!(
        predicted(&dry),
        predicted(&real),
        "dry-run prediction must equal the real run's actions",
    );
    assert!(fx.project.join("STUB_RULES.md").is_file());
    assert!(fx.project.join("stub.mcp.json").is_file());
    assert_eq!(
        std::fs::read(fx.project.join(".tome/RULES.md")).unwrap(),
        b"workspace rules body\n",
        "the real run writes the rules the dry run only previewed",
    );

    // ---- Phase 3: removal direction. Drop the harness from the marker; a ----
    // dry run predicts the removals without unlinking anything.
    std::fs::write(
        fx.project.join(".tome").join("config.toml"),
        "workspace = \"ws-dry\"\nharnesses = []\n",
    )
    .expect("rewrite marker without harnesses");

    let before_removal = snapshot_tree(&fx.project);
    let dry_removal =
        tome::commands::sync::sync_one_project(&fx.workspace, &fx.project, &args(true), &fx.paths)
            .expect("dry removal run");
    assert!(
        dry_removal.harness_changes >= 2,
        "dry run must classify the would-be removals: {dry_removal:?}",
    );
    assert_eq!(
        before_removal,
        snapshot_tree(&fx.project),
        "a dry removal run must not unlink or rewrite anything",
    );
    assert!(
        fx.project.join("STUB_RULES.md").is_file(),
        "the rules directive survives a dry removal run",
    );

    // The real removal then performs exactly the predicted set, and the tree
    // actually changed this time (proving the dry run was the only no-op).
    let real_removal =
        tome::commands::sync::sync_one_project(&fx.workspace, &fx.project, &args(false), &fx.paths)
            .expect("real removal run");
    assert_eq!(predicted(&dry_removal), predicted(&real_removal));
    assert_ne!(
        before_removal,
        snapshot_tree(&fx.project),
        "the real removal run must actually mutate the tree",
    );
}
