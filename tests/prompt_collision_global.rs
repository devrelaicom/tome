//! Phase 7 / F-MCP-PROMPT-COLLISION (FR-004) — prompt names must be
//! assigned against a SINGLE global taken-set so a counter suffix minted
//! in one bucket can never collide with a base name (or another suffix)
//! produced by a different bucket.
//!
//! Regression: previously `resolve_collisions` grouped candidates into
//! per-derived-name buckets and suffixed losers (`{base}{idx+1}`) WITHOUT
//! re-checking the suffix against names produced by OTHER buckets. So a
//! `foo`/`foo` collision (winner `foo`, loser `foo2`) plus an independent
//! `foo2` candidate yielded TWO entries named `foo2`; the terminal
//! `by_name.insert` in `PromptRegistry::build_for_workspace` then silently
//! dropped one user-invocable entry (absent from `prompts/list`,
//! unresolvable on `prompts/get`).
//!
//! `resolve_collisions` is sync + pure, so we drive it directly — the
//! in-process MCP harness (FR-012) does not exist yet.

use std::collections::HashSet;

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

/// The contract case from FR-004 / §R-5. Three candidates in insertion
/// order:
///   1. Command `foo`        → derived `foo`
///   2. user-invocable Skill `foo` → derived `foo`
///   3. Command `foo2`       → derived `foo2`
///
/// The `foo`/`foo` bucket mints loser suffix `foo2`, which ALREADY belongs
/// to candidate 3. A single global taken-set must push the loser to the
/// next free slot (`foo3`) so all three final names are distinct and no
/// entry is dropped.
#[test]
fn cross_bucket_suffix_collision_keeps_all_three_distinct() {
    let entries = vec![
        // Earliest indexed_at among the `foo` bucket — wins the base name.
        id(
            "cat",
            "p",
            EntryKind::Command,
            "foo",
            "2026-01-01T00:00:00Z",
            "foo",
        ),
        // Same derived name, later indexed_at — loser of the `foo` bucket.
        id(
            "cat",
            "p",
            EntryKind::Skill,
            "foo",
            "2026-02-01T00:00:00Z",
            "foo",
        ),
        // Independent candidate already occupying `foo2`.
        id(
            "cat",
            "p",
            EntryKind::Command,
            "foo2",
            "2026-03-01T00:00:00Z",
            "foo2",
        ),
    ];

    let (pairs, _collisions) = resolve_collisions(&entries);

    // No entry dropped: one resolved pair per input identity.
    assert_eq!(
        pairs.len(),
        3,
        "every input identity must map to a final name (got: {:?})",
        pairs.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
    );

    // Every final name is globally unique — the crux of the bug. Under the
    // old per-bucket logic this set would be {\"foo\", \"foo2\"} (len 2)
    // because the `foo` loser and the standalone `foo2` collide.
    let unique: HashSet<&str> = pairs.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        unique.len(),
        3,
        "final prompt names must be globally distinct (got: {:?})",
        pairs.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
    );

    // Each input identity is independently resolvable by its (catalog,
    // plugin, kind, name) key — i.e. the winner-keeps-base semantics still
    // hold and the standalone `foo2` is untouched.
    let find = |kind: EntryKind, name: &str| -> &str {
        pairs
            .iter()
            .find(|(_, idn)| idn.kind == kind && idn.name == name)
            .map(|(n, _)| n.as_str())
            .unwrap_or_else(|| panic!("input ({kind:?}, {name}) dropped from resolution"))
    };

    // First-in-insertion-order keeps the base name.
    assert_eq!(find(EntryKind::Command, "foo"), "foo");
    // The standalone candidate keeps its already-free derived name.
    assert_eq!(find(EntryKind::Command, "foo2"), "foo2");
    // The `foo` loser is pushed PAST the occupied `foo2` to the next free
    // slot rather than overwriting it.
    let loser = find(EntryKind::Skill, "foo");
    assert_ne!(loser, "foo2", "loser must not collide with standalone foo2");
    assert_eq!(loser, "foo3", "loser advances to the next free suffix");
}

/// The `CollisionRecord` surfaced to `tome doctor` must reflect the ACTUAL
/// final names assigned (post global de-confliction), not the naive
/// per-bucket suffix — otherwise the doctor collision surface misreports.
#[test]
fn collision_record_reflects_actual_globally_resolved_names() {
    let entries = vec![
        id(
            "cat",
            "p",
            EntryKind::Command,
            "foo",
            "2026-01-01T00:00:00Z",
            "foo",
        ),
        id(
            "cat",
            "p",
            EntryKind::Skill,
            "foo",
            "2026-02-01T00:00:00Z",
            "foo",
        ),
        id(
            "cat",
            "p",
            EntryKind::Command,
            "foo2",
            "2026-03-01T00:00:00Z",
            "foo2",
        ),
    ];

    let (_pairs, collisions) = resolve_collisions(&entries);

    // Only the `foo` bucket had >= 2 candidates.
    let foo_record = collisions
        .iter()
        .find(|r| r.base_name == "foo")
        .expect("the foo bucket collided and must be recorded");

    assert_eq!(foo_record.entries.len(), 2, "two members in the foo bucket");
    // Winner first, holding the base name.
    assert_eq!(foo_record.entries[0].final_name, "foo");
    // The recorded loser name must match what was actually assigned: the
    // de-conflicted `foo3`, never the colliding `foo2`.
    assert_eq!(
        foo_record.entries[1].final_name, "foo3",
        "the record must carry the real assigned name, not the naive foo2",
    );
}
