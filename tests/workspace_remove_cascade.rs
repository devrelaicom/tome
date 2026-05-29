//! Phase 4 / US2.b — `tome workspace remove` cascade step tests.
//!
//! Step 1 (per-bound-project integration teardown) and Step 5
//! (refcount-clean catalog caches) are exercised end-to-end against
//! the production helpers.

mod common;

use std::path::Path;

use common::{lifecycle_paths, stub_embedder_seed, stub_reranker_seed, stub_summariser_seed};
use tempfile::TempDir;
use time::OffsetDateTime;
use tome::harness::{BlockBodyStyle, McpConfigFormat, mcp_config, rules_file};
use tome::index::{self, OpenOptions, workspace_catalogs};
use tome::workspace::{self, WorkspaceName};

fn parse(name: &str) -> WorkspaceName {
    WorkspaceName::parse(name).expect("valid workspace name")
}

fn open_central(paths: &tome::paths::Paths) -> rusqlite::Connection {
    index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .expect("open central DB")
}

fn seed_bound_project(paths: &tome::paths::Paths, workspace_name: &str, project_root: &Path) {
    std::fs::create_dir_all(project_root.join(".tome")).expect("create .tome");
    // Declare claude-code in the project marker so the per-project
    // effective harness list resolves to [claude-code] for the cascade
    // (FR-405). Without this declaration the resolver would emit an
    // empty effective list — the cascade would correctly leave the
    // pre-seeded MCP / rules entries alone (they would belong to some
    // other phantom enrollee, not this project).
    std::fs::write(
        project_root.join(".tome").join("config.toml"),
        format!("workspace = \"{workspace_name}\"\nharnesses = [\"claude-code\"]\n"),
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

/// Step 1: the cascade tears down a real Tome-owned MCP entry from the
/// bound project's `.claude/settings.json`. Uses the production
/// `SUPPORTED_HARNESSES` registry (claude-code is the first entry); no
/// override needed.
#[test]
fn cascade_step1_tears_down_real_harness_mcp_entry() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("mine"), false, &paths).expect("init");

    let project = tmp.path().join("bound-project");
    std::fs::create_dir_all(&project).expect("create project");
    seed_bound_project(&paths, "mine", &project);

    // Pre-populate the bound project's `.claude/settings.json` with a
    // Tome-owned MCP entry, as `workspace use` would have done.
    let mcp_path = project.join(".claude/settings.json");
    let entry = mcp_config::TomeEntry::new(
        "tome".to_string(),
        vec![
            "mcp".to_string(),
            "--workspace".to_string(),
            "mine".to_string(),
        ],
    );
    mcp_config::write_entry(&mcp_path, McpConfigFormat::Json, "mcpServers", &entry)
        .expect("write entry");

    // Sanity: the entry is present before cascade.
    let pre =
        mcp_config::read_entry(&mcp_path, McpConfigFormat::Json, "mcpServers").expect("read pre");
    assert!(pre.is_some(), "MCP entry should be present before cascade");

    // Cascade. `home_root` is `tmp.path()` so any home-scoped harness
    // probes (Codex, Gemini) target the isolated tempdir, not the
    // user's real `$HOME`.
    let outcome =
        workspace::remove::remove(parse("mine"), true, &paths, tmp.path()).expect("remove");
    assert_eq!(outcome.bound_projects_torn_down, 1);

    // Post: the MCP entry was removed (claude-code's path was
    // `<project>/.claude/settings.json`).
    let post =
        mcp_config::read_entry(&mcp_path, McpConfigFormat::Json, "mcpServers").expect("read post");
    assert!(
        post.is_none(),
        "MCP entry should have been removed by cascade Step 1, got {post:?}",
    );
}

/// Step 5: the refcount check survives a shared catalog. Two
/// workspaces enrol the same URL. Removing one leaves the cache; then
/// removing the other reaps it. Both halves of the refcount contract
/// in one test.
#[test]
fn cascade_step5_refcount_cleans_unreferenced_catalog_cache() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("mine"), false, &paths).expect("init mine");
    workspace::init::init(parse("other"), false, &paths).expect("init other");

    let url = "https://example.com/shared.git";
    let cache_dir = paths.cache_dir_for(url);
    std::fs::create_dir_all(&cache_dir).expect("pre-create shared cache dir");
    // Drop a sentinel file so we can detect deletion / preservation.
    std::fs::write(cache_dir.join("sentinel"), b"alive").expect("write sentinel");

    // Enrol the URL into BOTH workspaces.
    {
        let conn = open_central(&paths);
        workspace_catalogs::insert(&conn, "mine", "shared", url, "main")
            .expect("enrol mine/shared");
        workspace_catalogs::insert(&conn, "other", "shared", url, "main")
            .expect("enrol other/shared");
    }

    // Remove `mine`: cache must STILL exist (still referenced by `other`).
    let outcome_mine =
        workspace::remove::remove(parse("mine"), false, &paths, tmp.path()).expect("remove mine");
    assert!(
        outcome_mine.catalog_caches_cleaned.is_empty(),
        "removing mine should NOT clean the shared cache; got cleaned={:?}",
        outcome_mine.catalog_caches_cleaned,
    );
    assert!(
        cache_dir.exists(),
        "shared cache dir should still exist after removing mine",
    );
    assert!(
        cache_dir.join("sentinel").exists(),
        "sentinel file should survive",
    );

    // Remove `other`: cache must NOW be reaped (refcount → 0).
    let outcome_other =
        workspace::remove::remove(parse("other"), false, &paths, tmp.path()).expect("remove other");
    assert_eq!(
        outcome_other.catalog_caches_cleaned,
        vec![url.to_string()],
        "removing other should reap the now-orphaned cache",
    );
    assert!(
        !cache_dir.exists(),
        "shared cache dir should have been removed after removing other",
    );
}

/// T-M3 (a): Step 1 must remove a Tome-owned rules-file block via the
/// `BlockInExistingFile` strategy. Pre-populate `<project>/CLAUDE.md`
/// (claude-code's corrected target, Phase 6 / FR-020) with a block; cascade
/// with --force; assert the block is gone but the file still exists.
#[test]
fn cascade_step1_removes_rules_block_via_block_in_existing_file_strategy() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("mine"), false, &paths).expect("init");

    let project = tmp.path().join("bound-project");
    std::fs::create_dir_all(&project).expect("create project");
    seed_bound_project(&paths, "mine", &project);

    // claude-code's rules target is `<project>/CLAUDE.md` (Phase 6
    // correction — never AGENTS.md) with strategy `BlockInExistingFile`.
    // Pre-populate with developer content + a Tome-owned block.
    let claude_md = project.join("CLAUDE.md");
    std::fs::write(&claude_md, "# Project guidelines\n\nKeep it tidy.\n").expect("seed CLAUDE.md");
    rules_file::write_block(&claude_md, "tome-owned body", BlockBodyStyle::AtInclude)
        .expect("write Tome block");

    let pre = std::fs::read_to_string(&claude_md).expect("read pre");
    assert!(
        pre.contains("<!-- tome:begin -->"),
        "Tome block should be present pre-cascade: {pre}",
    );

    // Cascade. claude-code is in the seeded effective list because the
    // project marker has no exclusion and the global / workspace
    // settings don't override.
    let outcome =
        workspace::remove::remove(parse("mine"), true, &paths, tmp.path()).expect("remove");
    assert_eq!(outcome.bound_projects_torn_down, 1);

    // The file is still on disk (developer-authored content stays);
    // the Tome block is gone.
    assert!(claude_md.is_file(), "CLAUDE.md should still exist");
    let post = std::fs::read_to_string(&claude_md).expect("read post");
    assert!(
        !post.contains("<!-- tome:begin -->"),
        "Tome block should be removed by cascade Step 1: {post}",
    );
    assert!(
        !post.contains("<!-- tome:end -->"),
        "Tome end marker should be removed: {post}",
    );
    assert!(
        post.contains("Keep it tidy."),
        "developer content should survive: {post}",
    );
}

/// T-M3 (b): a user-owned MCP entry (not Tome-owned — different
/// command) MUST survive the cascade. The per-project effective list +
/// `mcp_config::remove_entry`'s `is_tome_owned` gate together protect
/// it.
#[test]
fn cascade_step1_leaves_user_owned_mcp_entry_alone() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("mine"), false, &paths).expect("init");

    let project = tmp.path().join("bound-project");
    std::fs::create_dir_all(&project).expect("create project");
    seed_bound_project(&paths, "mine", &project);

    // Seed a `tome` entry whose `command` is NOT `tome` — looks like a
    // user-customised entry the cascade must not clobber.
    let mcp_path = project.join(".claude/settings.json");
    let user_entry = mcp_config::TomeEntry::new(
        "evil-binary".to_string(),
        vec!["--my-custom".to_string(), "args".to_string()],
    );
    mcp_config::write_entry(&mcp_path, McpConfigFormat::Json, "mcpServers", &user_entry)
        .expect("seed user-owned entry");
    let pre_bytes = std::fs::read(&mcp_path).expect("read pre");

    let outcome =
        workspace::remove::remove(parse("mine"), true, &paths, tmp.path()).expect("remove");
    assert_eq!(outcome.bound_projects_torn_down, 1);

    // The settings.json file must be byte-identical — neither the
    // entry shape nor the surrounding JSON changed.
    let post_bytes = std::fs::read(&mcp_path).expect("read post");
    assert_eq!(
        pre_bytes, post_bytes,
        "user-owned tome MCP entry must survive cascade unchanged",
    );
}

/// A workspace whose only catalog is also enrolled in `global` MUST
/// keep the cache after the workspace is removed.
#[test]
fn cascade_step5_keeps_shared_catalog_cache_with_global() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("mine"), false, &paths).expect("init mine");

    let url = "https://example.com/global-shared.git";
    let cache_dir = paths.cache_dir_for(url);
    std::fs::create_dir_all(&cache_dir).expect("pre-create cache");
    std::fs::write(cache_dir.join("sentinel"), b"alive").expect("sentinel");

    {
        let conn = open_central(&paths);
        workspace_catalogs::insert(&conn, "global", "shared", url, "main").expect("enrol global");
        workspace_catalogs::insert(&conn, "mine", "shared", url, "main").expect("enrol mine");
    }

    let outcome =
        workspace::remove::remove(parse("mine"), false, &paths, tmp.path()).expect("remove mine");
    assert!(
        outcome.catalog_caches_cleaned.is_empty(),
        "URL is still referenced by global; should not be cleaned",
    );
    assert!(
        cache_dir.exists(),
        "cache should still exist while global retains the enrolment",
    );
}
