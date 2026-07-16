//! DB-backed integration coverage for `uri_resolver::resolve`.
//!
//! Task 4 unit-tests the pure `collapse` decision logic (filter/dedupe/
//! sort/one-many-none) directly in `src/mcp/tools/uri_resolver.rs` with
//! constructed `SkillRecord`/`ResolvedEntry` values and no DB. This file
//! covers the DB-backed orchestration `resolve` adds on top: opening a
//! read-only connection, parsing the URI, resolving each candidate against
//! the real index, and collapsing the result — exercised end-to-end via the
//! `StagedWorkspace` fixture (real catalog + index, `StubEmbedder`, no
//! ONNX).
//!
//! The multi-match-across-catalogs case (a bare name colliding in two
//! catalogs → `Many`) is DEFERRED to Task 7, which adds a second-catalog
//! fixture when it wires `resolve` into `get_skill::handle`.

use tome::mcp::tools::uri_resolver::{ResolveOutcome, resolve};
use tome::plugin::identity::EntryKind;

use crate::common::mcp_harness::StagedWorkspace;

const SKILL: &str = "---\nname: foo\ndescription: The foo skill.\n---\nFoo body.\n";

#[test]
fn resolve_unique_triple_returns_one() {
    let ws = StagedWorkspace::stage(&[("foo", SKILL)], &[]);
    let uri = format!("{}:{}:foo", ws.catalog_name, ws.plugin_name);

    let out = resolve(&ws.paths, "global", &uri, &[EntryKind::Skill]).unwrap();
    match out {
        ResolveOutcome::One(entry) => {
            assert_eq!(entry.record.catalog, ws.catalog_name);
            assert_eq!(entry.record.plugin, ws.plugin_name);
            assert_eq!(entry.record.name, "foo");
            assert_eq!(entry.record.kind, EntryKind::Skill);
        }
        other => panic!("expected One, got {other:?}"),
    }
}

#[test]
fn resolve_unknown_uri_returns_nomatch_with_available() {
    let ws = StagedWorkspace::stage(&[("foo", SKILL)], &[]);
    let uri = format!("{}:{}:absent", ws.catalog_name, ws.plugin_name);

    let out = resolve(&ws.paths, "global", &uri, &[EntryKind::Skill]).unwrap();
    match out {
        ResolveOutcome::NoMatch { available } => {
            assert!(available.iter().any(|r| r.name == "foo"));
        }
        other => panic!("expected NoMatch, got {other:?}"),
    }
}
