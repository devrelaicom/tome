//! Phase 5 / US1.b — prompt-name collision resolution.
//!
//! Covers `contracts/mcp-prompts.md` § Collision handling: tie-break on
//! `indexed_at` then `(catalog, plugin, kind, name)` lex order; counter
//! suffix starts at `2`; each collision recorded.

use tome::mcp::prompt_collision::{EntryIdentity, resolve_collisions};
use tome::plugin::identity::EntryKind;

fn id(
    catalog: &str,
    plugin: &str,
    kind: EntryKind,
    name: &str,
    indexed_at: &str,
    derived: &str,
) -> EntryIdentity {
    EntryIdentity {
        catalog: catalog.into(),
        plugin: plugin.into(),
        kind,
        name: name.into(),
        indexed_at: indexed_at.into(),
        derived_name: derived.into(),
    }
}

#[test]
fn no_collisions_returns_inputs_with_empty_record_set() {
    let entries = vec![
        id(
            "midnight-expert",
            "compact-dev",
            EntryKind::Command,
            "fix-issue",
            "2026-01-01T00:00:00Z",
            "compact-dev__fix-issue",
        ),
        id(
            "midnight-expert",
            "wallet",
            EntryKind::Command,
            "deploy",
            "2026-01-01T00:00:00Z",
            "wallet__deploy",
        ),
    ];
    let (pairs, collisions) = resolve_collisions(&entries);
    assert_eq!(pairs.len(), 2);
    assert!(collisions.is_empty());
}

#[test]
fn counter_suffix_starts_at_2() {
    // Two entries derive the same prompt name. Winner gets the base;
    // loser gets `<base>2` (NOT `<base>1`).
    let entries = vec![
        // Earlier indexed_at — wins.
        id(
            "a",
            "p",
            EntryKind::Command,
            "foo",
            "2026-01-01T00:00:00Z",
            "shared",
        ),
        id(
            "b",
            "p",
            EntryKind::Command,
            "foo",
            "2026-02-01T00:00:00Z",
            "shared",
        ),
    ];
    let (pairs, collisions) = resolve_collisions(&entries);
    let names: Vec<&str> = pairs.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"shared"), "winner gets unsuffixed name");
    assert!(
        names.contains(&"shared2"),
        "first loser uses counter suffix `2` (got: {names:?})"
    );
    assert!(
        !names.contains(&"shared1"),
        "counter must NOT start at 1 (got: {names:?})"
    );
    assert_eq!(collisions.len(), 1);
    assert_eq!(collisions[0].base_name, "shared");
}

#[test]
fn three_way_collision_increments_2_3() {
    let entries = vec![
        id(
            "z",
            "p",
            EntryKind::Command,
            "foo",
            "2026-03-01T00:00:00Z",
            "shared",
        ),
        id(
            "a",
            "p",
            EntryKind::Command,
            "foo",
            "2026-01-01T00:00:00Z",
            "shared",
        ),
        id(
            "m",
            "p",
            EntryKind::Command,
            "foo",
            "2026-02-01T00:00:00Z",
            "shared",
        ),
    ];
    let (pairs, collisions) = resolve_collisions(&entries);
    let mut names: Vec<String> = pairs.iter().map(|(n, _)| n.clone()).collect();
    names.sort();
    assert_eq!(names, vec!["shared", "shared2", "shared3"]);
    assert_eq!(collisions.len(), 1);
    assert_eq!(collisions[0].entries.len(), 3);
}

#[test]
fn tie_break_on_catalog_plugin_kind_name_when_indexed_at_ties() {
    // Both indexed at the same instant — tie-break falls to the lex
    // tuple. Catalog `aardvark` < `zebra`.
    let entries = vec![
        id(
            "zebra",
            "p",
            EntryKind::Command,
            "foo",
            "2026-01-01T00:00:00Z",
            "shared",
        ),
        id(
            "aardvark",
            "p",
            EntryKind::Command,
            "foo",
            "2026-01-01T00:00:00Z",
            "shared",
        ),
    ];
    let (pairs, _) = resolve_collisions(&entries);
    let winner = pairs
        .iter()
        .find(|(n, _)| n == "shared")
        .expect("winner gets base name");
    assert_eq!(winner.1.catalog, "aardvark");

    let loser = pairs
        .iter()
        .find(|(n, _)| n == "shared2")
        .expect("loser gets counter suffix");
    assert_eq!(loser.1.catalog, "zebra");
}

#[test]
fn collision_record_carries_tie_break_order_winner_first() {
    let entries = vec![
        id(
            "b",
            "p",
            EntryKind::Command,
            "foo",
            "2026-02-01T00:00:00Z",
            "shared",
        ),
        id(
            "a",
            "p",
            EntryKind::Command,
            "foo",
            "2026-01-01T00:00:00Z",
            "shared",
        ),
    ];
    let (_, collisions) = resolve_collisions(&entries);
    assert_eq!(collisions.len(), 1);
    let record = &collisions[0];
    // Winner first.
    assert_eq!(record.entries[0].identity.catalog, "a");
    assert_eq!(record.entries[0].final_name, "shared");
    assert_eq!(record.entries[1].identity.catalog, "b");
    assert_eq!(record.entries[1].final_name, "shared2");
}

#[test]
fn override_derived_name_can_collide_with_default_derived_name() {
    // One entry uses default derivation, another uses an explicit
    // `prompt_name` override — both end up at the same string. The
    // resolver doesn't care HOW the name was derived, only that two
    // identities share a bucket.
    let entries = vec![
        id(
            "midnight",
            "compact-dev",
            EntryKind::Command,
            "fix",
            "2026-01-01T00:00:00Z",
            "shared",
        ),
        id(
            "midnight",
            "wallet",
            EntryKind::Command,
            "deploy",
            "2026-01-02T00:00:00Z",
            "shared", // override-derived, same key
        ),
    ];
    let (pairs, collisions) = resolve_collisions(&entries);
    assert_eq!(collisions.len(), 1);
    let names: Vec<&str> = pairs.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"shared"));
    assert!(names.contains(&"shared2"));
}
