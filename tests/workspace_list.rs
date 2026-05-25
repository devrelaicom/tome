//! Phase 4 / US2.a-1 — `tome workspace list` library-API tests.
//!
//! Covers the bootstrap-not-yet path (no DB file → one synthetic
//! `global` entry), the populated-registry path (multiple workspaces
//! with distinct counts), and the JSON wire-shape byte-stability pin.

mod common;

use common::{lifecycle_paths, seed_workspace};
use tempfile::TempDir;
use tome::commands::workspace::list::{WorkspaceListEntry, assemble};
use tome::index::workspace_catalogs;

fn open_central(paths: &tome::paths::Paths) -> rusqlite::Connection {
    let (e, r, s) = tome::commands::plugin::registry_seeds();
    tome::index::open(
        &paths.index_db,
        &tome::index::OpenOptions {
            embedder: e,
            reranker: r,
            summariser: s,
        },
    )
    .expect("open central DB")
}

#[test]
fn list_only_global_on_bootstrap_not_yet() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    let entries = assemble(&paths).expect("assemble");
    assert_eq!(entries.len(), 1);
    let g = &entries[0];
    assert_eq!(g.name, "global");
    assert_eq!(g.catalogs, 0);
    assert_eq!(g.enabled_plugins, 0);
    assert_eq!(g.indexed_skills, 0);
    assert_eq!(g.bound_projects, 0);
    assert_eq!(g.last_used_at, 0);
}

#[test]
fn list_reports_seeded_global_after_bootstrap() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    // Open central DB to trigger schema bootstrap (seeds `global`).
    let _ = open_central(&paths);

    let entries = assemble(&paths).expect("assemble");
    assert_eq!(entries.len(), 1);
    let g = &entries[0];
    assert_eq!(g.name, "global");
    // The bootstrap stamps last_used_at to the bootstrap time.
    assert!(
        g.last_used_at > 0,
        "global should have non-zero last_used_at"
    );
}

#[test]
fn list_two_workspaces_with_distinct_counts() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    // Bootstrap + extra workspace.
    seed_workspace(&paths, "extra");

    // Seed catalog enrolments: global has 2 catalogs; extra has 1.
    {
        let conn = open_central(&paths);
        workspace_catalogs::insert(&conn, "global", "a", "https://example.com/a", "main").unwrap();
        workspace_catalogs::insert(&conn, "global", "b", "https://example.com/b", "main").unwrap();
        workspace_catalogs::insert(&conn, "extra", "c", "https://example.com/c", "v1").unwrap();
    }

    let entries = assemble(&paths).expect("assemble");
    assert_eq!(entries.len(), 2);
    // Alphabetical by name: extra, global.
    assert_eq!(entries[0].name, "extra");
    assert_eq!(entries[0].catalogs, 1);
    assert_eq!(entries[1].name, "global");
    assert_eq!(entries[1].catalogs, 2);
    for e in &entries {
        // No skills enabled in this fixture.
        assert_eq!(e.enabled_plugins, 0);
        assert_eq!(e.indexed_skills, 0);
        assert_eq!(e.bound_projects, 0);
    }
}

#[test]
fn list_json_wire_shape_is_byte_stable() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    // Bootstrap-not-yet → one synthetic `global` entry with all-zero
    // counts and last_used_at = 0.
    let entries = assemble(&paths).expect("assemble");
    let json = serde_json::to_string(&entries).expect("serialise");
    assert_eq!(
        json,
        r#"[{"name":"global","catalogs":0,"enabled_plugins":0,"indexed_skills":0,"bound_projects":0,"last_used_at":0}]"#,
    );
}

#[test]
fn list_entries_are_sorted_alphabetically() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    seed_workspace(&paths, "zeta");
    seed_workspace(&paths, "alpha");

    let entries = assemble(&paths).expect("assemble");
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["alpha", "global", "zeta"]);
}

#[test]
fn list_entry_wire_struct_pins_field_order() {
    // Spot-check the field order of a single record. Combined with
    // list_json_wire_shape_is_byte_stable above, this fails fast if
    // a contributor reorders the struct.
    let entry = WorkspaceListEntry {
        name: "x".into(),
        catalogs: 0,
        enabled_plugins: 0,
        indexed_skills: 0,
        bound_projects: 0,
        last_used_at: 0,
    };
    let json = serde_json::to_string(&entry).unwrap();
    assert_eq!(
        json,
        r#"{"name":"x","catalogs":0,"enabled_plugins":0,"indexed_skills":0,"bound_projects":0,"last_used_at":0}"#,
    );
}
