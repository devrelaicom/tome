//! T048 — Cursor native-agent translation (Phase 6 / US1).
//!
//! Drives [`tome::harness::cursor::CURSOR`] `translate_agent` DIRECTLY and
//! asserts the contract's Cursor row (`contracts/agent-translation.md`):
//!
//! * Markdown + YAML emission (`MarkdownYaml`), filename
//!   `<plugin>__<name>.md`;
//! * an unsupported field is DROPPED and RECORDED in `dropped_fields`.
//!   Cursor carries `name` / `description` / `tools` but has no carrier for
//!   `model` (no enumerated same-vendor Anthropic id yet, FR-034) nor for the
//!   privileged fields — all of those drop. We assert both the model drop and
//!   a privileged-field drop.

use tome::harness::AgentFormat;
use tome::harness::HarnessModule;
use tome::harness::agents::CanonicalAgent;
use tome::harness::cursor::CURSOR;

fn agent() -> CanonicalAgent {
    CanonicalAgent {
        catalog: "cat".into(),
        plugin: "midnight-expert".into(),
        name: "reviewer".into(),
        description: Some("Reviews code".into()),
        body: "You review.\n".into(),
        model: Some("opus".into()),
        tools: Some(vec!["Read".into(), "Grep".into()]),
        disallowed_tools: None,
        // A privileged field with no Cursor carrier — must drop + record.
        hooks: Some(serde_json::json!({"PreToolUse": []})),
        mcp_servers: None,
        permission_mode: None,
    }
}

#[test]
fn emits_markdown_yaml_with_namespaced_filename() {
    let t = CURSOR.translate_agent(&agent(), false).expect("translate");
    assert_eq!(t.format, AgentFormat::MarkdownYaml, "Cursor is MD+YAML");
    assert_eq!(t.filename, "midnight-expert__reviewer.md");
    assert!(
        t.rendered.starts_with("---\n"),
        "Markdown+YAML frontmatter header:\n{}",
        t.rendered,
    );
    // The carried-through fields are present.
    assert!(t.rendered.contains("name: reviewer"));
    assert!(t.rendered.contains("tools:"));
}

#[test]
fn unsupported_model_is_dropped_and_recorded() {
    let t = CURSOR.translate_agent(&agent(), false).expect("translate");
    assert!(
        !t.rendered.contains("model:"),
        "Cursor drops model (no enumerated same-vendor id, FR-034):\n{}",
        t.rendered,
    );
    assert!(
        t.dropped_fields.contains(&"model".to_owned()),
        "dropped model must be recorded; got {:?}",
        t.dropped_fields,
    );
}

#[test]
fn unsupported_privileged_field_is_dropped_and_recorded() {
    let t = CURSOR.translate_agent(&agent(), false).expect("translate");
    // The privileged `hooks` blob has no Cursor carrier.
    assert!(
        !t.rendered.contains("hooks"),
        "Cursor drops the privileged hooks field:\n{}",
        t.rendered,
    );
    assert!(
        t.dropped_fields.contains(&"hooks".to_owned()),
        "dropped privileged field must be recorded; got {:?}",
        t.dropped_fields,
    );
}
