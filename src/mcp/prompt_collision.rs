//! Phase 5 — prompt-name collision resolution for MCP prompts.
//!
//! Two or more entries deriving to the same prompt name share a bucket;
//! the winner keeps the base name and losers get counter-suffixed
//! starting at `2`. The tie-break per FR-062 sorts on:
//!
//! 1. `indexed_at` ASC.
//! 2. Tuple `(catalog, plugin, kind, name)` lex order for ties.
//!
//! Each collision is recorded for `tracing` + the doctor surface
//! (FR-121) to expose.
//!
//! Contract: `specs/005-phase-5-commands-prompts/contracts/mcp-prompts.md`
//! § Collision handling.

use crate::plugin::identity::EntryKind;

/// Minimal identity carrier used by the resolver. Decoupled from
/// `SkillRecord` so the resolver stays pure and trivially testable.
#[derive(Debug, Clone)]
pub struct EntryIdentity {
    pub catalog: String,
    pub plugin: String,
    pub kind: EntryKind,
    pub name: String,
    /// Indexed timestamp (RFC 3339 string stored in `skills.indexed_at`).
    /// Used as the primary tie-break per FR-062. Lexicographic ordering
    /// on RFC 3339 strings agrees with chronological ordering.
    pub indexed_at: String,
    /// The derived prompt name (the bucket key). Pre-computed by the
    /// caller via [`crate::mcp::prompt_name::derive_name`] so the
    /// resolver only does grouping + tie-break.
    pub derived_name: String,
}

/// Record of one collision bucket — one entry per name that had two or
/// more candidates. Emitted regardless of how many entries collided so
/// the doctor surface can list each one.
#[derive(Debug, Clone)]
pub struct CollisionRecord {
    /// The bucket key — the derived name BEFORE counter suffixing.
    pub base_name: String,
    /// The bucket members, sorted in tie-break order (winner first).
    /// Each carries the final name assigned post-suffixing.
    pub entries: Vec<ResolvedCollisionEntry>,
}

/// One entry inside a [`CollisionRecord`] with its final assigned name
/// (the base name for the winner, suffixed for losers).
#[derive(Debug, Clone)]
pub struct ResolvedCollisionEntry {
    pub identity: EntryIdentity,
    pub final_name: String,
}

/// Resolve collisions across a candidate set. Returns:
///
/// 1. The final `(prompt_name, entry)` mapping for every input entry —
///    keys may differ from `entry.derived_name` when counter-suffixed.
/// 2. One `CollisionRecord` per bucket that had >= 2 candidates.
///
/// Order of the returned pairs follows the input order after the
/// tie-break has resolved each bucket; callers needing alphabetical
/// output sort the keys themselves (rmcp's `PromptRouter::list_all`
/// already does this for `prompts/list`).
pub fn resolve_collisions(
    entries: &[EntryIdentity],
) -> (Vec<(String, EntryIdentity)>, Vec<CollisionRecord>) {
    // Group by derived name, preserving original insertion order so the
    // result is deterministic for fixtures.
    let mut buckets: std::collections::BTreeMap<String, Vec<EntryIdentity>> =
        std::collections::BTreeMap::new();
    for e in entries {
        buckets
            .entry(e.derived_name.clone())
            .or_default()
            .push(e.clone());
    }

    let mut out: Vec<(String, EntryIdentity)> = Vec::with_capacity(entries.len());
    let mut collisions: Vec<CollisionRecord> = Vec::new();

    for (base, mut members) in buckets {
        if members.len() == 1 {
            out.push((base, members.pop().expect("len == 1")));
            continue;
        }

        // Tie-break: indexed_at ASC, then (catalog, plugin, kind, name).
        // `kind.as_str()` gives the wire-stable lowercase string.
        members.sort_by(|a, b| {
            a.indexed_at
                .cmp(&b.indexed_at)
                .then_with(|| a.catalog.cmp(&b.catalog))
                .then_with(|| a.plugin.cmp(&b.plugin))
                .then_with(|| a.kind.as_str().cmp(b.kind.as_str()))
                .then_with(|| a.name.cmp(&b.name))
        });

        let mut record_entries: Vec<ResolvedCollisionEntry> = Vec::with_capacity(members.len());
        for (idx, identity) in members.into_iter().enumerate() {
            let final_name = if idx == 0 {
                base.clone()
            } else {
                // Counter-suffix starts at 2 — first loser is index 1.
                format!("{base}{}", idx + 1)
            };
            out.push((final_name.clone(), identity.clone()));
            record_entries.push(ResolvedCollisionEntry {
                identity,
                final_name,
            });
        }
        collisions.push(CollisionRecord {
            base_name: base,
            entries: record_entries,
        });
    }

    (out, collisions)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn no_collisions_pass_through() {
        let entries = vec![
            id(
                "a",
                "p",
                EntryKind::Command,
                "foo",
                "2026-01-01T00:00:00Z",
                "a__foo",
            ),
            id(
                "b",
                "q",
                EntryKind::Command,
                "bar",
                "2026-01-01T00:00:00Z",
                "b__bar",
            ),
        ];
        let (pairs, collisions) = resolve_collisions(&entries);
        assert_eq!(pairs.len(), 2);
        assert!(collisions.is_empty());
    }

    #[test]
    fn collision_winner_takes_base_name() {
        let entries = vec![
            id(
                "z",
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
        let (pairs, collisions) = resolve_collisions(&entries);
        assert_eq!(collisions.len(), 1);
        // Earlier indexed_at wins — that's the catalog "a" row.
        let winner = pairs.iter().find(|(n, _)| n == "shared").unwrap();
        assert_eq!(winner.1.catalog, "a");
        let loser = pairs.iter().find(|(n, _)| n == "shared2").unwrap();
        assert_eq!(loser.1.catalog, "z");
    }
}
