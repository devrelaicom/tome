//! T046 / T053 — Claude Code native-agent translation (Phase 6 / US1).
//!
//! Drives [`tome::harness::claude_code::CLAUDE_CODE`] `translate_agent`
//! DIRECTLY (no sync, no DB): build a `CanonicalAgent` carrying the full
//! canonical vocabulary including the three privileged fields
//! (`hooks` / `mcpServers` / `permissionMode`) and assert the contract's
//! Claude Code row (`contracts/agent-translation.md` § Per-harness emission
//! table + § Field mapping):
//!
//! * format is `MarkdownYaml`, filename `<plugin>__<name>.md`;
//! * the body lands in the FILE BODY (after the closing `---`);
//! * `name` / `description` / `model` / `tools` land in the YAML
//!   frontmatter;
//! * the three privileged fields ARE passed through by default (FR-050 — the
//!   Claude Code capability advantage; US5's `strip_plugin_agent_privileges`
//!   is NOT applied here), with the privilege-passthrough default asserted
//!   explicitly.
//!
//! T053 (placeholder): a byte-stable JSON wire-shape pin for the agent
//! `dropped_fields` data AS IT EXISTS NOW. The US5 doctor `DroppedFieldEntry`
//! record has not landed yet; this pin freezes the current shape so US5 can
//! evolve it deliberately rather than silently. See the test comment.

use serde::Serialize;
use tome::harness::AgentFormat;
use tome::harness::HarnessModule;
use tome::harness::agents::CanonicalAgent;
use tome::harness::claude_code::CLAUDE_CODE;

/// A fully-populated canonical agent: model + tools + the three privileged
/// fields, plus a multi-line body to assert body placement.
fn full_agent() -> CanonicalAgent {
    CanonicalAgent {
        catalog: "cat".into(),
        plugin: "midnight-expert".into(),
        name: "reviewer".into(),
        description: Some("Reviews Compact code".into()),
        body: "You are a careful reviewer.\nBe thorough.\n".into(),
        model: Some("opus".into()),
        tools: Some(vec!["Read".into(), "Grep".into()]),
        disallowed_tools: None,
        hooks: Some(serde_json::json!({"PreToolUse": [{"matcher": "Bash"}]})),
        mcp_servers: Some(serde_json::json!({"foo": {"command": "x"}})),
        permission_mode: Some("ask".into()),
    }
}

/// Split a rendered Markdown+YAML agent file into `(frontmatter, body)` on
/// the second `---` delimiter line. Asserting against the two halves
/// separately keeps the "in frontmatter" vs "in file body" placement claims
/// unambiguous.
fn split_frontmatter_body(rendered: &str) -> (String, String) {
    // The render primitive always begins with `---\n<yaml>---\n<body>`.
    let after_open = rendered
        .strip_prefix("---\n")
        .expect("rendered file opens with a frontmatter delimiter");
    let close = after_open
        .find("\n---\n")
        .expect("rendered file has a closing frontmatter delimiter");
    let frontmatter = &after_open[..close + 1];
    let body = &after_open[close + "\n---\n".len()..];
    (frontmatter.to_owned(), body.to_owned())
}

// ---------------------------------------------------------------------------
// T046
// ---------------------------------------------------------------------------

#[test]
fn emits_markdown_yaml_with_namespaced_filename() {
    let reg = tome::model_registry::test_registry();
    let t = CLAUDE_CODE
        .translate_agent(&full_agent(), false, &reg)
        .expect("translate");

    assert_eq!(
        t.format,
        AgentFormat::MarkdownYaml,
        "Claude Code is MD+YAML"
    );
    assert_eq!(
        t.filename, "midnight-expert__reviewer.md",
        "filename is the `<plugin>__<name>.md` provenance form (FR-040)",
    );
    assert_eq!(
        t.displayed_name, "reviewer",
        "no clash → clean displayed name",
    );
}

#[test]
fn body_lands_in_file_body_not_frontmatter() {
    let reg = tome::model_registry::test_registry();
    let agent = full_agent();
    let t = CLAUDE_CODE
        .translate_agent(&agent, false, &reg)
        .expect("translate");
    let (frontmatter, body) = split_frontmatter_body(&t.rendered);

    assert!(
        body.contains("You are a careful reviewer.") && body.contains("Be thorough."),
        "the system-prompt body lands in the file body:\n{body}",
    );
    assert_eq!(
        body, agent.body,
        "body is reproduced verbatim after the frontmatter",
    );
    assert!(
        !frontmatter.contains("You are a careful reviewer."),
        "the body must NOT leak into the YAML frontmatter:\n{frontmatter}",
    );
}

#[test]
fn name_description_model_tools_in_frontmatter() {
    let reg = tome::model_registry::test_registry();
    let t = CLAUDE_CODE
        .translate_agent(&full_agent(), false, &reg)
        .expect("translate");
    let (frontmatter, _body) = split_frontmatter_body(&t.rendered);

    assert!(
        frontmatter.contains("name: reviewer"),
        "name in frontmatter:\n{frontmatter}",
    );
    assert!(
        frontmatter.contains("description: Reviews Compact code"),
        "description in frontmatter:\n{frontmatter}",
    );
    // `model: opus` is same-vendor and passes through verbatim (FR-037).
    assert!(
        frontmatter.contains("model: opus"),
        "model (same-vendor alias) in frontmatter:\n{frontmatter}",
    );
    assert!(
        frontmatter.contains("tools:") && frontmatter.contains("Read"),
        "tools allowlist in frontmatter:\n{frontmatter}",
    );
}

#[test]
fn privileged_fields_pass_through_by_default() {
    // FR-050: the three privileged fields are a Claude Code-only capability
    // advantage and are passed through by DEFAULT in US1 — the
    // `strip_plugin_agent_privileges` suppression is US5 and is NOT applied
    // here. Assert the default behaviour explicitly.
    let reg = tome::model_registry::test_registry();
    let t = CLAUDE_CODE
        .translate_agent(&full_agent(), false, &reg)
        .expect("translate");
    let (frontmatter, _body) = split_frontmatter_body(&t.rendered);

    assert!(
        frontmatter.contains("hooks:"),
        "hooks passed through by default (FR-050):\n{frontmatter}",
    );
    assert!(
        frontmatter.contains("mcpServers:"),
        "mcpServers passed through by default (FR-050):\n{frontmatter}",
    );
    assert!(
        frontmatter.contains("permissionMode: ask"),
        "permissionMode passed through by default (FR-050):\n{frontmatter}",
    );

    // Nothing was dropped — every canonical field has a Claude Code carrier.
    assert!(
        t.dropped_fields.is_empty(),
        "no field dropped for the canonical vendor; got {:?}",
        t.dropped_fields,
    );
}

// ---------------------------------------------------------------------------
// T053 — byte-stable JSON wire-shape pin for `dropped_fields` (PLACEHOLDER)
// ---------------------------------------------------------------------------

/// PLACEHOLDER mirror of the agent dropped-fields wire data as it exists in
/// US1: a flat `Vec<String>` of the canonical field names dropped during
/// translation. `TranslatedAgent` is not itself `Serialize`, so this small
/// local mirror pins the shape the US5 doctor surface will consume.
///
/// When US5 lands `DroppedFieldEntry` (a richer per-field record with a
/// reason / harness), THIS pin must be updated deliberately — that is the
/// whole point of freezing the current shape now. Do not delete this test
/// without porting the assertion into the US5 doctor wire-shape pin.
#[derive(Serialize)]
struct DroppedFieldsWirePlaceholder<'a> {
    dropped_fields: &'a [String],
}

#[test]
fn dropped_fields_wire_shape_placeholder_is_byte_stable() {
    // Use a Claude Code translation that DOES drop a field so the pinned
    // shape is non-empty and meaningful. `model: inherit` drops everywhere
    // (the model alias table, FR-037), so it is the natural single-drop case
    // for the canonical vendor (which otherwise drops nothing).
    let agent = CanonicalAgent {
        model: Some("inherit".into()),
        ..full_agent()
    };
    let reg = tome::model_registry::test_registry();
    let t = CLAUDE_CODE
        .translate_agent(&agent, false, &reg)
        .expect("translate");
    assert_eq!(
        t.dropped_fields,
        vec!["model".to_owned()],
        "inherit drops the model field for the canonical vendor",
    );

    let wire = DroppedFieldsWirePlaceholder {
        dropped_fields: &t.dropped_fields,
    };
    let json = serde_json::to_string(&wire).expect("serialise placeholder");
    // Byte-stable pin. US5's `DroppedFieldEntry` will change this — that
    // change must be a deliberate edit to THIS literal.
    assert_eq!(json, r#"{"dropped_fields":["model"]}"#);
}
