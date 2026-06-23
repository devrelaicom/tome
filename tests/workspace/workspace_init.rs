//! Phase 4 / US2.a-1 — `tome workspace init <name>` library-API tests.
//!
//! These exercise `workspace::init::init` directly. The CLI binary
//! wrapper is a thin adapter; the exit-code surface is enforced by
//! `tests/exit_codes.rs`.

use crate::common::lifecycle_paths;
use tempfile::TempDir;
use tome::error::TomeError;
use tome::index::{self, OpenOptions, workspace_catalogs};
use tome::workspace::{self, WorkspaceName};

fn parse(name: &str) -> WorkspaceName {
    WorkspaceName::parse(name).expect("valid workspace name")
}

fn open_central(paths: &tome::paths::Paths) -> rusqlite::Connection {
    let (e, r, s) = tome::commands::plugin::registry_seeds();
    index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: e,
            reranker: r,
            summariser: s,
            profile: None,
        },
    )
    .expect("open central DB")
}

#[test]
fn init_with_invalid_name_exits_15() {
    // The WorkspaceName::parse gate fires BEFORE init even gets called.
    // Verify the exit-code shape so the CLI surface lines up with FR-347.
    let err = WorkspaceName::parse("bad name with spaces").unwrap_err();
    assert!(matches!(err, TomeError::WorkspaceNameInvalid { .. }));
    assert_eq!(err.exit_code(), 15);
}

#[test]
fn init_reserved_name_global_is_rejected() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    let name = parse("global");
    let err = workspace::init::init(name, false, &paths).unwrap_err();
    assert!(
        matches!(err, TomeError::WorkspaceNameInvalid { .. }),
        "expected WorkspaceNameInvalid, got {err:?}",
    );
    assert_eq!(err.exit_code(), 15);
}

#[test]
fn init_with_existing_name_exits_14() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    let outcome = workspace::init::init(parse("foo"), false, &paths).expect("first init");
    assert_eq!(outcome.name.as_str(), "foo");

    let err = workspace::init::init(parse("foo"), false, &paths).unwrap_err();
    assert!(
        matches!(err, TomeError::WorkspaceAlreadyExists { .. }),
        "expected WorkspaceAlreadyExists, got {err:?}",
    );
    assert_eq!(err.exit_code(), 14);
}

#[test]
fn init_happy_path_creates_dir_and_row() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    let outcome = workspace::init::init(parse("new-ws"), false, &paths).expect("init");
    assert_eq!(outcome.name.as_str(), "new-ws");
    assert_eq!(outcome.catalogs_inherited, 0);

    // On-disk shape: settings.toml + RULES.md inside the workspace dir.
    let ws_dir = paths.workspace_dir(&outcome.name);
    assert!(
        ws_dir.is_dir(),
        "workspace dir {} missing",
        ws_dir.display()
    );
    let settings_body = std::fs::read_to_string(ws_dir.join("settings.toml")).unwrap();
    assert!(
        settings_body.contains("name = \"new-ws\""),
        "settings.toml missing name: {settings_body}",
    );
    // [summaries] is shipped as a commented-out header (the strict
    // deserialiser rejects an empty `[summaries]` table because
    // CachedSummaries' fields are required). The comment makes the
    // shape visible to humans editing the file.
    assert!(
        settings_body.contains("[summaries]"),
        "settings.toml should reference [summaries] section: {settings_body}",
    );
    assert!(
        settings_body.contains("regen-summary"),
        "settings.toml should point at regen-summary: {settings_body}",
    );
    let rules_body = std::fs::read_to_string(ws_dir.join("RULES.md")).unwrap();
    assert!(
        rules_body.contains("No summary yet"),
        "RULES.md should ship with placeholder comment, got: {rules_body:?}",
    );
    assert!(
        rules_body.contains("regen-summary"),
        "RULES.md placeholder should point at regen-summary, got: {rules_body:?}",
    );

    // DB shape: a `workspaces` row with the given name + a non-zero
    // created_at + last_used_at.
    let conn = open_central(&paths);
    let row: (String, i64, i64) = conn
        .query_row(
            "SELECT name, created_at, last_used_at FROM workspaces WHERE name = ?1",
            rusqlite::params!["new-ws"],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(row.0, "new-ws");
    assert!(row.1 > 0);
    assert!(row.2 > 0);
}

#[test]
fn init_inherit_global_copies_catalogs() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    // Seed global with two enrolments.
    {
        let conn = open_central(&paths);
        workspace_catalogs::insert(&conn, "global", "cat-a", "https://example.com/a", "main")
            .unwrap();
        workspace_catalogs::insert(&conn, "global", "cat-b", "https://example.com/b", "v1")
            .unwrap();
    }

    let outcome = workspace::init::init(parse("derived"), true, &paths).expect("init");
    assert_eq!(outcome.catalogs_inherited, 2);

    // DB: junction rows under the new workspace.
    let conn = open_central(&paths);
    let rows = workspace_catalogs::list_for_workspace(&conn, "derived").unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].catalog_name, "cat-a");
    assert_eq!(rows[0].url, "https://example.com/a");
    assert_eq!(rows[0].pinned_ref, "main");
    assert_eq!(rows[1].catalog_name, "cat-b");

    // Settings file: [[catalogs]] array mirrors the junction rows.
    let body = std::fs::read_to_string(paths.workspace_settings_file(&outcome.name)).unwrap();
    assert!(
        body.contains("[[catalogs]]"),
        "missing [[catalogs]]: {body}"
    );
    assert!(body.contains("name = \"cat-a\""), "missing cat-a: {body}");
    assert!(body.contains("name = \"cat-b\""), "missing cat-b: {body}");
    assert!(
        body.contains("url = \"https://example.com/a\""),
        "missing url a: {body}",
    );
}

#[test]
fn init_inherit_global_with_no_catalogs_is_noop() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    // Open the DB so `global` is seeded but with zero enrolments.
    let _ = open_central(&paths);

    let outcome = workspace::init::init(parse("empty"), true, &paths).expect("init");
    assert_eq!(outcome.catalogs_inherited, 0);

    let body = std::fs::read_to_string(paths.workspace_settings_file(&outcome.name)).unwrap();
    assert!(
        !body.contains("[[catalogs]]"),
        "empty inherit should NOT emit a [[catalogs]] block: {body}",
    );
}

#[test]
fn init_writes_settings_toml_at_workspace_dir() {
    // Pin the on-disk layout: <root>/workspaces/<name>/settings.toml
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    let outcome = workspace::init::init(parse("layout"), false, &paths).unwrap();
    let expected_dir = paths.root.join("workspaces").join("layout");
    assert_eq!(outcome.path, expected_dir);
    assert!(expected_dir.join("settings.toml").is_file());
    assert!(expected_dir.join("RULES.md").is_file());
}
