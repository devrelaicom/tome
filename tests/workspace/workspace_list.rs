//! Phase 4 / US2.a-1 — `tome workspace list` library-API tests.
//!
//! Covers the bootstrap-not-yet path (no DB file → one synthetic
//! `global` entry), the populated-registry path (multiple workspaces
//! with distinct counts), and the JSON wire-shape byte-stability pin.
//!
//! Issue #300 adds: the row resolved for the current directory is flagged
//! `current: true` (and no other), the `--json` `current` field, and the
//! human relative/absolute `Last used` rendering.

use crate::common::{ToolEnv, lifecycle_paths, paths_for, seed_workspace};
use std::path::Path;
use tempfile::TempDir;
use time::OffsetDateTime;
use tome::commands::workspace::list::{WorkspaceListEntry, assemble};
use tome::index::workspace_catalogs;
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

fn open_central(paths: &tome::paths::Paths) -> rusqlite::Connection {
    let (e, r, s) = tome::commands::plugin::registry_seeds();
    tome::index::open(
        &paths.index_db,
        &tome::index::OpenOptions {
            embedder: e,
            reranker: r,
            summariser: s,
            profile: None,
        },
    )
    .expect("open central DB")
}

/// A `ResolvedScope` resolving to `name` (the value the resolver would
/// hand `list`). `source` is informational here — `list` only reads the
/// resolved *name* to mark the active row.
fn scope_for(name: &str, source: ScopeSource) -> ResolvedScope {
    ResolvedScope {
        scope: Scope(WorkspaceName::parse(name).unwrap()),
        source,
        project_root: None,
        overridden_project_marker: None,
    }
}

/// The `global` fallback scope (the common "nothing bound" default).
fn global_scope() -> ResolvedScope {
    ResolvedScope::global_fallback()
}

#[test]
fn list_only_global_on_bootstrap_not_yet() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    let entries = assemble(&global_scope(), &paths).expect("assemble");
    assert_eq!(entries.len(), 1);
    let g = &entries[0];
    assert_eq!(g.name, "global");
    assert_eq!(g.catalogs, 0);
    assert_eq!(g.enabled_plugins, 0);
    assert_eq!(g.indexed_skills, 0);
    assert_eq!(g.bound_projects, 0);
    assert_eq!(g.last_used_at, 0);
    // The `global` fallback resolves to `global`, so the synthetic row is
    // the active one.
    assert!(g.current, "global row is current under the global fallback");
}

#[test]
fn list_reports_seeded_global_after_bootstrap() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    // Open central DB to trigger schema bootstrap (seeds `global`).
    let _ = open_central(&paths);

    let entries = assemble(&global_scope(), &paths).expect("assemble");
    assert_eq!(entries.len(), 1);
    let g = &entries[0];
    assert_eq!(g.name, "global");
    // The bootstrap stamps last_used_at to the bootstrap time.
    assert!(
        g.last_used_at > 0,
        "global should have non-zero last_used_at"
    );
    assert!(g.current, "global row is current under the global fallback");
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

    let entries = assemble(&global_scope(), &paths).expect("assemble");
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
    // counts and last_used_at = 0. Under the global fallback the row is
    // `current: true`.
    let entries = assemble(&global_scope(), &paths).expect("assemble");
    let json = serde_json::to_string(&entries).expect("serialise");
    assert_eq!(
        json,
        r#"[{"name":"global","catalogs":0,"enabled_plugins":0,"indexed_skills":0,"bound_projects":0,"last_used_at":0,"current":true}]"#,
    );
}

#[test]
fn list_entries_are_sorted_alphabetically() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    seed_workspace(&paths, "zeta");
    seed_workspace(&paths, "alpha");

    let entries = assemble(&global_scope(), &paths).expect("assemble");
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["alpha", "global", "zeta"]);
}

/// Issue #300: exactly the resolved workspace's row is flagged `current`,
/// and no other. Here the scope resolves to `alpha`, so only `alpha` is
/// marked — `global` and `zeta` are not.
#[test]
fn list_marks_only_the_resolved_workspace_current() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    seed_workspace(&paths, "zeta");
    seed_workspace(&paths, "alpha");

    let scope = scope_for("alpha", ScopeSource::ProjectMarker);
    let entries = assemble(&scope, &paths).expect("assemble");

    let current: Vec<&str> = entries
        .iter()
        .filter(|e| e.current)
        .map(|e| e.name.as_str())
        .collect();
    assert_eq!(current, vec!["alpha"], "only the resolved row is current");

    // Every other row is explicitly not current.
    for e in &entries {
        if e.name == "alpha" {
            assert!(e.current, "alpha must be current");
        } else {
            assert!(!e.current, "{} must not be current", e.name);
        }
    }
}

/// Issue #300: a `--workspace zeta` style resolution marks `zeta`, not the
/// `global` fallback — proving the marker follows the ACTUAL resolved name
/// regardless of source.
#[test]
fn list_current_follows_the_resolved_name_not_global() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    seed_workspace(&paths, "zeta");

    let scope = scope_for("zeta", ScopeSource::Flag);
    let entries = assemble(&scope, &paths).expect("assemble");

    let zeta = entries.iter().find(|e| e.name == "zeta").expect("zeta row");
    let global = entries
        .iter()
        .find(|e| e.name == "global")
        .expect("global row");
    assert!(zeta.current, "zeta is the resolved workspace");
    assert!(!global.current, "global is not the resolved workspace");
}

/// T-M6: a workspace seeded with bound projects + enabled plugins +
/// catalog enrolments must report non-zero counts. Validates the
/// COUNT-aggregate SQL in `assemble`.
#[test]
fn list_reports_non_zero_counts_for_active_workspace() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    seed_workspace(&paths, "active");

    // Catalog enrolment.
    {
        let conn = open_central(&paths);
        workspace_catalogs::insert(&conn, "active", "primary", "https://example.com/p", "main")
            .unwrap();
    }

    // Two enabled skills under one (catalog, plugin), so enabled_plugins
    // distinct-count is 1 but indexed_skills is 2.
    seed_enabled_skill_for_test(&paths, "active", "primary", "myplug", "skill-a");
    seed_enabled_skill_for_test(&paths, "active", "primary", "myplug", "skill-b");

    // Two bound projects.
    let project_a = tmp.path().join("proj-a");
    let project_b = tmp.path().join("proj-b");
    seed_bound_project_for_test(&paths, "active", &project_a);
    seed_bound_project_for_test(&paths, "active", &project_b);

    let entries = assemble(&global_scope(), &paths).expect("assemble");
    let active = entries
        .iter()
        .find(|e| e.name == "active")
        .expect("`active` entry present");
    assert_eq!(active.catalogs, 1);
    assert_eq!(active.enabled_plugins, 1, "two skills under one plugin");
    assert_eq!(active.indexed_skills, 2);
    assert_eq!(active.bound_projects, 2);
}

/// Helper local to this file. Other suites have their own copies; we
/// keep the helper inline here to avoid a `common/mod.rs` widening.
fn seed_enabled_skill_for_test(
    paths: &tome::paths::Paths,
    workspace_name: &str,
    catalog: &str,
    plugin: &str,
    skill_name: &str,
) {
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
        "INSERT INTO skills
           (catalog, plugin, name, description, plugin_version, path, content_hash, indexed_at)
         VALUES (?1, ?2, ?3, '', '0.0.0', '/dev/null', 'hash', ?4)",
        rusqlite::params![catalog, plugin, skill_name, now],
    )
    .expect("insert skill");
    let skill_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![workspace_id, skill_id, now],
    )
    .expect("insert workspace_skills");
}

fn seed_bound_project_for_test(paths: &tome::paths::Paths, workspace_name: &str, project: &Path) {
    std::fs::create_dir_all(project.join(".tome")).expect("create .tome");
    std::fs::write(
        project.join(".tome").join("config.toml"),
        format!("workspace = \"{workspace_name}\"\n"),
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
        rusqlite::params![project.to_string_lossy().to_string(), workspace_id, now],
    )
    .expect("seed workspace_projects");
}

#[test]
fn list_entry_wire_struct_pins_field_order() {
    // Spot-check the field order of a single record. Combined with
    // list_json_wire_shape_is_byte_stable above, this fails fast if
    // a contributor reorders the struct. `current` is appended LAST.
    let entry = WorkspaceListEntry {
        name: "x".into(),
        catalogs: 0,
        enabled_plugins: 0,
        indexed_skills: 0,
        bound_projects: 0,
        last_used_at: 0,
        current: false,
    };
    let json = serde_json::to_string(&entry).unwrap();
    assert_eq!(
        json,
        r#"{"name":"x","catalogs":0,"enabled_plugins":0,"indexed_skills":0,"bound_projects":0,"last_used_at":0,"current":false}"#,
    );
}

/// Issue #300: `--json` carries the absolute unix-second timestamp
/// UNCHANGED — the relative/absolute distinction is human-only, so a
/// non-zero `last_used_at` serialises as the raw integer, never a relative
/// string.
#[test]
fn list_json_last_used_stays_absolute_integer() {
    let entry = WorkspaceListEntry {
        name: "recent".into(),
        catalogs: 0,
        enabled_plugins: 0,
        indexed_skills: 0,
        bound_projects: 0,
        last_used_at: 1_700_000_000,
        current: true,
    };
    let json = serde_json::to_string(&entry).unwrap();
    assert!(
        json.contains("\"last_used_at\":1700000000"),
        "--json must carry the absolute integer timestamp; got {json}",
    );
    assert!(
        json.contains("\"current\":true"),
        "--json must carry the current bool; got {json}",
    );
}

/// Issue #300 — end-to-end coverage of the human-table glue that the pure
/// `human_last_used` unit tests can't reach: the `Cur` column `*` marker and
/// the `run → emit → emit_human` `--absolute` threading. Drives the REAL
/// `tome workspace list` binary under an isolated `$HOME`, so a regression in
/// the marker render or an un-threaded `--absolute` flag fails here even
/// though the unit tests stay green.
///
/// Determinism: the resolved workspace's `last_used_at` is stamped to a KNOWN
/// past instant (10 days ago) so the relative form is a stable "days ago"
/// bucket (never a wall-clock-sensitive "just now") and the absolute form is a
/// concrete RFC 3339 string. Resolution is pinned via `TOME_WORKSPACE` (the
/// highest-precedence non-flag source, membership-checked against the seeded
/// DB) so `my-project` is unambiguously the current row for the spawned
/// process's CWD.
#[test]
fn list_binary_renders_cur_marker_and_absolute_flag() {
    let env = ToolEnv::new();

    // `tome workspace init my-project` bootstraps the central DB (seeds
    // `global`) and inserts the named workspace.
    let init = env
        .cmd()
        .args(["workspace", "init", "my-project"])
        .output()
        .expect("spawn workspace init");
    assert!(
        init.status.success(),
        "workspace init must succeed; stderr={}",
        String::from_utf8_lossy(&init.stderr),
    );

    // Stamp `my-project`'s last_used_at to a fixed instant 10 days ago so the
    // relative rendering is a deterministic "10 days ago" and the absolute
    // rendering is a concrete RFC 3339 timestamp.
    let paths = paths_for(&env);
    let ten_days_ago = OffsetDateTime::now_utc().unix_timestamp() - 10 * 86_400;
    {
        let conn = open_central(&paths);
        conn.execute(
            "UPDATE workspaces SET last_used_at = ?1 WHERE name = ?2",
            rusqlite::params![ten_days_ago, "my-project"],
        )
        .expect("stamp last_used_at");
    }

    // --- Default (relative) render -----------------------------------------
    let relative = env
        .cmd()
        .env("TOME_WORKSPACE", "my-project")
        .env("COLUMNS", "200") // wide → the table never wraps a row
        .args(["workspace", "list"])
        .output()
        .expect("spawn workspace list");
    assert!(
        relative.status.success(),
        "workspace list must succeed; stderr={}",
        String::from_utf8_lossy(&relative.stderr),
    );
    let out = String::from_utf8_lossy(&relative.stdout);

    let current_row = row_containing(&out, "my-project");
    let other_row = row_containing(&out, "global");

    // The resolved workspace's row carries the `*` current marker.
    assert!(
        current_row.contains('*'),
        "the resolved workspace row must carry the `*` marker; row={current_row:?}\nfull:\n{out}",
    );
    // A non-resolved row does NOT.
    assert!(
        !other_row.contains('*'),
        "a non-resolved workspace row must not carry the `*` marker; row={other_row:?}\nfull:\n{out}",
    );
    // `Last used` renders relative (the "ago" bucket), NOT an RFC 3339 stamp.
    assert!(
        current_row.contains("ago"),
        "default render must show a relative time (contains 'ago'); row={current_row:?}\nfull:\n{out}",
    );
    assert!(
        !is_rfc3339_shaped(current_row),
        "default render must NOT be an RFC 3339 timestamp; row={current_row:?}\nfull:\n{out}",
    );

    // --- `--absolute` render -----------------------------------------------
    let absolute = env
        .cmd()
        .env("TOME_WORKSPACE", "my-project")
        .env("COLUMNS", "200")
        .args(["workspace", "list", "--absolute"])
        .output()
        .expect("spawn workspace list --absolute");
    assert!(
        absolute.status.success(),
        "workspace list --absolute must succeed; stderr={}",
        String::from_utf8_lossy(&absolute.stderr),
    );
    let out_abs = String::from_utf8_lossy(&absolute.stdout);
    let current_row_abs = row_containing(&out_abs, "my-project");

    // `--absolute` is actually threaded through to the human render: the
    // same row now shows an RFC 3339 timestamp, not the relative form.
    assert!(
        is_rfc3339_shaped(current_row_abs),
        "--absolute must render an RFC 3339 timestamp (T…Z); row={current_row_abs:?}\nfull:\n{out_abs}",
    );
    assert!(
        !current_row_abs.contains("ago"),
        "--absolute must NOT render the relative form; row={current_row_abs:?}\nfull:\n{out_abs}",
    );
    // The `*` marker is unaffected by `--absolute`.
    assert!(
        current_row_abs.contains('*'),
        "the `*` marker must persist under --absolute; row={current_row_abs:?}\nfull:\n{out_abs}",
    );
}

/// Return the first line of `haystack` containing `needle`. The comfy-table
/// output puts each workspace on its own row line, so this isolates the row
/// under test for cell-level assertions. Panics with the full output if the
/// row is absent (a clearer failure than an empty match).
fn row_containing<'a>(haystack: &'a str, needle: &str) -> &'a str {
    haystack
        .lines()
        .find(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no row containing {needle:?} in output:\n{haystack}"))
}

/// A loose RFC 3339 shape check: an ISO date with the `T` separator and a
/// trailing `Z`, e.g. `2026-06-21T10:23:11Z`. Deliberately not a full parse —
/// robust against exact seconds while still distinguishing "2026-…T…Z" from a
/// relative "10 days ago".
fn is_rfc3339_shaped(s: &str) -> bool {
    // Find a `NNNN-NN-NNTNN:NN:NN` prefix followed (within the same token) by
    // a `Z`. Scan tokens so surrounding table borders don't interfere.
    s.split_whitespace().any(|tok| {
        let bytes = tok.as_bytes();
        // Minimum: 20 chars "2026-06-21T10:23:11Z".
        bytes.len() >= 20
            && tok.contains('T')
            && tok.ends_with('Z')
            && bytes[4] == b'-'
            && bytes[7] == b'-'
            && bytes[10] == b'T'
            && bytes[13] == b':'
            && bytes[16] == b':'
    })
}
