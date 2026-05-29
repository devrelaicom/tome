//! T047 — Codex native-agent translation (Phase 6 / US1).
//!
//! Drives [`tome::harness::codex::CODEX`] `translate_agent` DIRECTLY and
//! asserts the contract's Codex row (`contracts/agent-translation.md`):
//!
//! * format is `Toml`, filename `<plugin>__<name>.toml`;
//! * the multi-line body lands in a TRIPLE-QUOTED `developer_instructions`
//!   string (FR-033, R-14) — verified by parsing the rendered TOML and
//!   reading the value back, not just substring-matching;
//! * `model` is DROPPED (no Anthropic alias maps to an OpenAI id, FR-034):
//!   absent from output AND present in `dropped_fields`;
//! * a read-only tool posture maps to `sandbox_mode = "read-only"` (FR-036).

use tome::harness::AgentFormat;
use tome::harness::HarnessModule;
use tome::harness::agents::CanonicalAgent;
use tome::harness::codex::CODEX;

/// A read-only agent (allowlist grants no write/edit/execute tool) carrying a
/// multi-line body and `model: opus`.
fn read_only_agent() -> CanonicalAgent {
    CanonicalAgent {
        catalog: "cat".into(),
        plugin: "midnight-expert".into(),
        name: "reviewer".into(),
        description: Some("Reviews code".into()),
        body: "You are a careful reviewer.\nLine two.\nLine three.\n".into(),
        model: Some("opus".into()),
        tools: Some(vec!["Read".into(), "Grep".into()]),
        disallowed_tools: None,
        hooks: None,
        mcp_servers: None,
        permission_mode: None,
    }
}

#[test]
fn emits_toml_with_namespaced_filename() {
    let t = CODEX
        .translate_agent(&read_only_agent(), false)
        .expect("translate");
    assert_eq!(t.format, AgentFormat::Toml, "Codex emits TOML");
    assert_eq!(
        t.filename, "midnight-expert__reviewer.toml",
        "filename is `<plugin>__<name>.toml` (FR-040)",
    );
}

#[test]
fn body_lands_in_triple_quoted_developer_instructions() {
    let agent = read_only_agent();
    let t = CODEX.translate_agent(&agent, false).expect("translate");

    // Triple-quote form is the contract-mandated multi-line basic string.
    assert!(
        t.rendered.contains("developer_instructions = \"\"\""),
        "multi-line body must render as a triple-quoted string:\n{}",
        t.rendered,
    );

    // Parse the rendered TOML and read the value back: the body round-trips
    // verbatim into `developer_instructions`.
    let doc: toml_edit::DocumentMut = t.rendered.parse().expect("rendered Codex TOML parses");
    assert_eq!(
        doc["developer_instructions"].as_str(),
        Some(agent.body.as_str()),
        "developer_instructions holds the body verbatim",
    );
}

#[test]
fn model_is_dropped_and_recorded() {
    let t = CODEX
        .translate_agent(&read_only_agent(), false)
        .expect("translate");

    // Absent from the rendered output: no `model` key at all.
    let doc: toml_edit::DocumentMut = t.rendered.parse().expect("parse");
    assert!(
        doc.get("model").is_none(),
        "Codex never carries an Anthropic-sourced model (FR-034):\n{}",
        t.rendered,
    );
    // Recorded for the doctor surface.
    assert!(
        t.dropped_fields.contains(&"model".to_owned()),
        "dropped model must be recorded; got {:?}",
        t.dropped_fields,
    );
}

#[test]
fn read_only_posture_maps_to_sandbox_mode() {
    let t = CODEX
        .translate_agent(&read_only_agent(), false)
        .expect("translate");
    let doc: toml_edit::DocumentMut = t.rendered.parse().expect("parse");
    assert_eq!(
        doc["sandbox_mode"].as_str(),
        Some("read-only"),
        "read-only tool posture → sandbox_mode = \"read-only\" (FR-036):\n{}",
        t.rendered,
    );
}
