//! Byte-stable JSON wire-shape pin for `search_skills`'s `Output`
//! / `SkillMatch` shape. Phase 5 / US4.c, extended by #285.
//!
//! Snapshots pinned:
//!
//! 1. `SkillMatch` for a skill-kind hit — `kind: "skill"` lowercased per
//!    `EntryKind`'s `#[serde(rename_all = "lowercase")]` derive, all
//!    documented fields present in document order per
//!    `contracts/mcp-tools-p5.md` § `search_skills` result element.
//! 2. `SkillMatch` for a command-kind hit — `kind: "command"` to prove
//!    the discriminator round-trips both variants.
//!
//! Each snapshot is constructed directly from the public types so the
//! test doesn't need a staged workspace or the index — it pins the
//! Serialize impl shape, not the handler's behaviour (which the
//! `mcp_search_skills_truncation.rs` / `mcp_search_skills_signal.rs`
//! tests cover end-to-end).
//!
//! #285: the `Output` gained two ALWAYS-present fields (`corpus_size`,
//! `scoring`) plus three OPTIONAL fields (`reranker_drift`,
//! `no_results_reason`, `hint`, all `skip_serializing_if`). A normal
//! non-empty result's shape therefore stays stable except for the two
//! new always-present fields — the optional ones are absent unless drift
//! is detected / the result set is empty.
//!
//! Any field rename, reorder, default flip, or accidental
//! `#[serde(skip_serializing_if = ...)]` addition will flip this test
//! red.

use tome::mcp::tools::search_skills::{NoResultsReason, Output, SkillMatch};
use tome::plugin::identity::EntryKind;

/// Build a single-result `Output` with the #285 always-present fields set
/// to their common-path values (a populated corpus, reranked scoring, no
/// drift, non-empty matches → no reason/hint).
fn output_with(matches: Vec<SkillMatch>) -> Output {
    Output {
        matches,
        corpus_size: 42,
        scoring: "reranked".into(),
        reranker_drift: None,
        no_results_reason: None,
        hint: None,
    }
}

#[test]
fn skill_match_wire_shape_for_skill_kind() {
    let m = SkillMatch {
        catalog: "midnight-expert".into(),
        plugin: "compact-dev".into(),
        name: "compact-circuits".into(),
        kind: EntryKind::Skill,
        description: "Truncated description body.".into(),
        plugin_version: "1.4.0".into(),
        path: "/abs/path/to/SKILL.md".into(),
        score: 0.87,
        // A non-invocable skill has no prompt — `prompt_name` is `None` and
        // MUST be omitted (the #289 additive field is `skip_serializing_if`),
        // keeping the pre-#289 skill wire shape byte-identical.
        prompt_name: None,
    };
    let out = output_with(vec![m]);

    let json = serde_json::to_string(&out).expect("serialise");

    // SkillMatch document order: catalog, plugin, name, kind, description,
    // plugin_version, path, score. `kind` is lowercase via
    // `#[serde(rename_all = "lowercase")]` on `EntryKind`. `prompt_name` is
    // ABSENT (None + skip_serializing_if). Output-level: `matches`, then the
    // #285 always-present `corpus_size` + `scoring`; the optional #285 fields
    // (`reranker_drift`/`no_results_reason`/`hint`) are ABSENT here.
    let expected = r#"{"matches":[{"catalog":"midnight-expert","plugin":"compact-dev","name":"compact-circuits","kind":"skill","description":"Truncated description body.","plugin_version":"1.4.0","path":"/abs/path/to/SKILL.md","score":0.87}],"corpus_size":42,"scoring":"reranked"}"#;

    assert_eq!(
        json, expected,
        "search_skills skill-kind JSON wire shape drift — check field renames, reorders, or default flips",
    );
}

#[test]
fn skill_match_wire_shape_for_command_kind() {
    // #289: a user-invocable command carries its derived MCP `prompt_name` so
    // the result is actionable via `prompts/get`. The field is appended LAST
    // and serialises only when present.
    let m = SkillMatch {
        catalog: "midnight-expert".into(),
        plugin: "compact-dev".into(),
        name: "fix-issue".into(),
        kind: EntryKind::Command,
        description: "Fix a GitHub issue.".into(),
        plugin_version: "1.4.0".into(),
        path: "/abs/path/to/commands/fix-issue.md".into(),
        score: 0.42,
        prompt_name: Some("compact-dev__fix-issue".into()),
    };
    let out = output_with(vec![m]);

    let json = serde_json::to_string(&out).expect("serialise");

    let expected = r#"{"matches":[{"catalog":"midnight-expert","plugin":"compact-dev","name":"fix-issue","kind":"command","description":"Fix a GitHub issue.","plugin_version":"1.4.0","path":"/abs/path/to/commands/fix-issue.md","score":0.42,"prompt_name":"compact-dev__fix-issue"}],"corpus_size":42,"scoring":"reranked"}"#;

    assert_eq!(
        json, expected,
        "search_skills command-kind JSON wire shape drift — `kind` must serialise as lowercase `command`, `prompt_name` appended LAST",
    );
}

#[test]
fn skill_match_wire_shape_for_non_invocable_command_omits_prompt_name() {
    // #289: a command with `user_invocable: false` has no prompt — `prompt_name`
    // is omitted, so a caller seeing `kind: command` without `prompt_name`
    // knows it has no prompt to invoke.
    let m = SkillMatch {
        catalog: "midnight-expert".into(),
        plugin: "compact-dev".into(),
        name: "internal-only".into(),
        kind: EntryKind::Command,
        description: "Internal command.".into(),
        plugin_version: "1.4.0".into(),
        path: "/abs/path/to/commands/internal-only.md".into(),
        score: 0.10,
        prompt_name: None,
    };
    let out = output_with(vec![m]);

    let json = serde_json::to_string(&out).expect("serialise");

    let expected = r#"{"matches":[{"catalog":"midnight-expert","plugin":"compact-dev","name":"internal-only","kind":"command","description":"Internal command.","plugin_version":"1.4.0","path":"/abs/path/to/commands/internal-only.md","score":0.1}],"corpus_size":42,"scoring":"reranked"}"#;

    assert_eq!(
        json, expected,
        "a non-invocable command must omit `prompt_name`",
    );
}

#[test]
fn empty_matches_wire_shape_index_empty() {
    // #285: on an empty result set with an EMPTY index, the optional
    // `no_results_reason` + `hint` fields carry the reindex signal; `scoring`
    // + `corpus_size` are still always present. `reranker_drift` stays absent.
    let out = Output {
        matches: vec![],
        corpus_size: 0,
        scoring: "reranked".into(),
        reranker_drift: None,
        no_results_reason: Some(NoResultsReason::IndexEmpty),
        hint: Some("The index is empty — run `tome reindex`.".into()),
    };
    let json = serde_json::to_string(&out).expect("serialise");
    let expected = r#"{"matches":[],"corpus_size":0,"scoring":"reranked","no_results_reason":"index_empty","hint":"The index is empty — run `tome reindex`."}"#;
    assert_eq!(
        json, expected,
        "empty-index result must carry `no_results_reason: index_empty` + a hint, matches an empty array",
    );
}

#[test]
fn empty_matches_wire_shape_no_match() {
    // #285: on an empty result set with a POPULATED index, the reason is
    // `no_match` and the hint points at rephrasing.
    let out = Output {
        matches: vec![],
        corpus_size: 7,
        scoring: "embedding-similarity".into(),
        reranker_drift: None,
        no_results_reason: Some(NoResultsReason::NoMatch),
        hint: Some("No semantic match — try rephrasing.".into()),
    };
    let json = serde_json::to_string(&out).expect("serialise");
    let expected = r#"{"matches":[],"corpus_size":7,"scoring":"embedding-similarity","no_results_reason":"no_match","hint":"No semantic match — try rephrasing."}"#;
    assert_eq!(
        json, expected,
        "populated-index no-match result must carry `no_results_reason: no_match` + a rephrase hint",
    );
}

#[test]
fn reranker_drift_field_serialises_when_present() {
    // #285: `reranker_drift` rides only when detected — pin its wire position
    // (after `scoring`, before the empty-result fields) and value.
    let m = SkillMatch {
        catalog: "acme".into(),
        plugin: "plug".into(),
        name: "thing".into(),
        kind: EntryKind::Skill,
        description: "desc".into(),
        plugin_version: "1.0.0".into(),
        path: "/abs/SKILL.md".into(),
        score: 0.5,
        prompt_name: None,
    };
    let out = Output {
        matches: vec![m],
        corpus_size: 3,
        scoring: "embedding-similarity".into(),
        reranker_drift: Some("stored=a, configured=b".into()),
        no_results_reason: None,
        hint: None,
    };
    let json = serde_json::to_string(&out).expect("serialise");
    let expected = r#"{"matches":[{"catalog":"acme","plugin":"plug","name":"thing","kind":"skill","description":"desc","plugin_version":"1.0.0","path":"/abs/SKILL.md","score":0.5}],"corpus_size":3,"scoring":"embedding-similarity","reranker_drift":"stored=a, configured=b"}"#;
    assert_eq!(
        json, expected,
        "reranker_drift must serialise after `scoring` when present, and be omitted otherwise",
    );
}
