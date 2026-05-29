//! T049 — OpenCode native-agent translation (Phase 6 / US1).
//!
//! Drives [`tome::harness::opencode::OPENCODE`] `translate_agent` DIRECTLY
//! and asserts the contract's OpenCode row + specifics
//! (`contracts/agent-translation.md` § OpenCode specifics):
//!
//! * `mode: subagent` (source agents are subagents);
//! * the displayed / registered name is ALWAYS the filename-derived
//!   `<plugin>__<name>` (FR-042 — the prefix cannot be hidden), even when
//!   `clashes = false`;
//! * `model: opus` maps to the same-vendor `anthropic/claude-opus-4.7`;
//! * `description` is required — the FR-035 fallback chain: source
//!   description → first non-empty trimmed body line → placeholder.

use tome::harness::HarnessModule;
use tome::harness::agents::CanonicalAgent;
use tome::harness::opencode::OPENCODE;

/// Base agent with NO source description (drives the FR-035 fallback) and a
/// read-only tool posture.
fn base(name: &str, body: &str) -> CanonicalAgent {
    CanonicalAgent {
        catalog: "cat".into(),
        plugin: "midnight-expert".into(),
        name: name.into(),
        description: None,
        body: body.into(),
        model: Some("opus".into()),
        tools: Some(vec!["Read".into()]),
        disallowed_tools: None,
        hooks: None,
        mcp_servers: None,
        permission_mode: None,
    }
}

#[test]
fn mode_defaults_to_subagent() {
    let t = OPENCODE
        .translate_agent(&base("reviewer", "First line.\n"), false)
        .expect("translate");
    assert!(
        t.rendered.contains("mode: subagent"),
        "OpenCode agents default to mode: subagent:\n{}",
        t.rendered,
    );
}

#[test]
fn displayed_name_is_filename_derived_even_without_clash() {
    // `clashes = false`, yet OpenCode derives the name from the filename, so
    // the registered name is ALWAYS `<plugin>__<name>` (FR-042).
    let t = OPENCODE
        .translate_agent(&base("reviewer", "First line.\n"), false)
        .expect("translate");
    assert_eq!(
        t.displayed_name, "midnight-expert__reviewer",
        "OpenCode displayed name is filename-derived regardless of clash",
    );
    assert_eq!(t.filename, "midnight-expert__reviewer.md");
}

#[test]
fn model_opus_maps_to_anthropic_same_vendor_id() {
    let t = OPENCODE
        .translate_agent(&base("reviewer", "First line.\n"), false)
        .expect("translate");
    assert!(
        t.rendered.contains("model: anthropic/claude-opus-4.7"),
        "opus → opencode same-vendor anthropic id (FR-037):\n{}",
        t.rendered,
    );
}

#[test]
fn description_falls_back_to_first_non_empty_body_line() {
    // No source description → first non-empty, trimmed body line.
    let t = OPENCODE
        .translate_agent(
            &base("reviewer", "\n   First real line.   \nSecond line.\n"),
            false,
        )
        .expect("translate");
    assert!(
        t.rendered.contains("description: First real line."),
        "description falls back to the first non-empty trimmed body line:\n{}",
        t.rendered,
    );
}

#[test]
fn description_falls_back_to_placeholder_when_body_empty() {
    // No source description AND an empty body → documented placeholder.
    let t = OPENCODE
        .translate_agent(&base("solo", ""), false)
        .expect("translate");
    assert!(
        t.rendered.contains("Agent solo (no description provided)."),
        "empty body → documented placeholder description:\n{}",
        t.rendered,
    );
}

#[test]
fn source_description_wins_over_body_fallback() {
    // When the source DOES carry a description, it takes precedence over the
    // body-line fallback (top of the FR-035 precedence chain).
    let agent = CanonicalAgent {
        description: Some("Explicit source description".into()),
        ..base("reviewer", "Body line that must NOT be used.\n")
    };
    let t = OPENCODE.translate_agent(&agent, false).expect("translate");
    assert!(
        t.rendered
            .contains("description: Explicit source description"),
        "source description wins over the body fallback:\n{}",
        t.rendered,
    );
    // The body line lands verbatim in the FILE BODY (every source body does),
    // but it must NOT be promoted into the `description:` key — the source
    // description takes precedence over the body-line fallback (FR-035).
    assert!(
        !t.rendered
            .contains("description: Body line that must NOT be used."),
        "the body fallback must not fire when a source description exists:\n{}",
        t.rendered,
    );
}
