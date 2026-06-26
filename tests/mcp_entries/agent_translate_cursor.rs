//! T048 — Cursor native-agent translation (Phase 6 / US1).
//!
//! Drives [`tome::harness::cursor::CURSOR`] `translate_agent` DIRECTLY and
//! asserts the contract's Cursor row (`contracts/agent-translation.md`):
//!
//! * Markdown + YAML emission (`MarkdownYaml`), filename
//!   `<plugin>__<name>.md`;
//! * `tools:` is DROPPED (Cursor has no tools field) and recorded in
//!   `dropped_fields`;
//! * a pinned `model` maps to `inherit` (Cursor's proprietary ids are not in
//!   the registry; `inherit` preserves intent) and is EMITTED, not dropped;
//! * the privileged fields have no Cursor carrier and drop.

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
    let reg = tome::model_registry::test_registry();
    let t = CURSOR
        .translate_agent(&agent(), false, &reg)
        .expect("translate");
    assert_eq!(t.format, AgentFormat::MarkdownYaml, "Cursor is MD+YAML");
    assert_eq!(t.filename, "midnight-expert__reviewer.md");
    assert!(
        t.rendered.starts_with("---\n"),
        "Markdown+YAML frontmatter header:\n{}",
        t.rendered,
    );
    // The carried-through fields are present.
    assert!(t.rendered.contains("name: reviewer"));
    assert!(!t.rendered.contains("tools:"), "Cursor has no tools field");
}

#[test]
fn pinned_model_becomes_inherit() {
    let reg = tome::model_registry::test_registry();
    let t = CURSOR
        .translate_agent(&agent(), false, &reg)
        .expect("translate");
    assert!(
        t.rendered.contains("model: inherit"),
        "Cursor: pinned model → inherit (proprietary ids not in registry, FR-034):\n{}",
        t.rendered,
    );
    assert!(
        !t.dropped_fields.contains(&"model".to_owned()),
        "model maps to inherit, so it is NOT in dropped_fields; got {:?}",
        t.dropped_fields,
    );
}

#[test]
fn unsupported_privileged_field_is_dropped_and_recorded() {
    let reg = tome::model_registry::test_registry();
    let t = CURSOR
        .translate_agent(&agent(), false, &reg)
        .expect("translate");
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

/// T-2 / C-2: an indeterminate posture (no `tools`, no `disallowedTools`)
/// emits NO `readonly` key and records no harness target-name drop.
#[test]
fn indeterminate_posture_omits_readonly_key() {
    let agent = CanonicalAgent {
        tools: None,
        disallowed_tools: None,
        hooks: None,
        ..agent()
    };
    let reg = tome::model_registry::test_registry();
    let t = CURSOR
        .translate_agent(&agent, false, &reg)
        .expect("translate");
    assert!(
        !t.rendered.contains("readonly:"),
        "indeterminate posture must inherit Cursor's default (no readonly key):\n{}",
        t.rendered,
    );
    // C-2: the harness target name is never recorded as a dropped field.
    assert!(
        !t.dropped_fields.contains(&"readonly".to_owned()),
        "harness target name must NOT appear in dropped_fields; got {:?}",
        t.dropped_fields,
    );
}

/// T-2 / C-2: an explicit not-read-only allowlist emits NO `readonly` key.
/// Cursor has no `tools` field — the allowlist is DROPPED and recorded in
/// `dropped_fields`. The `readonly` harness target name is never recorded.
#[test]
fn not_read_only_allowlist_omits_readonly_and_keeps_tools() {
    let agent = CanonicalAgent {
        tools: Some(vec!["Read".into(), "Edit".into()]),
        disallowed_tools: None,
        hooks: None,
        ..agent()
    };
    let reg = tome::model_registry::test_registry();
    let t = CURSOR
        .translate_agent(&agent, false, &reg)
        .expect("translate");
    assert!(
        !t.rendered.contains("readonly:"),
        "not-read-only allowlist must not assert readonly:\n{}",
        t.rendered,
    );
    // Cursor has no tools field — the allowlist is dropped.
    assert!(
        !t.rendered.contains("tools:"),
        "Cursor has no tools field; allowlist must be dropped:\n{}",
        t.rendered,
    );
    // The source `tools` field IS recorded in dropped_fields.
    assert!(
        t.dropped_fields.contains(&"tools".to_owned()),
        "`tools` must be recorded in dropped_fields; got {:?}",
        t.dropped_fields,
    );
    // C-2: the harness target name is never recorded as a dropped field.
    assert!(
        !t.dropped_fields.contains(&"readonly".to_owned()),
        "harness target name must NOT appear in dropped_fields; got {:?}",
        t.dropped_fields,
    );
}
