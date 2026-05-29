//! Phase 6 / US5 — byte-stable JSON wire pins for the five new doctor
//! records (T132). Mirrors the Phase 4/5 pin style in `tests/doctor_json.rs`:
//! each `*Report` is constructed directly and serialised, the exact wire shape
//! asserted. These records are emit-only `Serialize` types consumed by `jq`
//! and by harnesses, so a field rename / order drift must break loudly
//! (NFR-011).

use serde_json::{Value, json};
use tome::doctor::report::{
    AgentHarnessEntry, AgentsReport, CatalogPlugin, DroppedFieldEntry, GuardrailsFileEntry,
    GuardrailsReport, HookEventEntry, HookPluginEntry, HooksReport, PersonaEntry, PersonaReport,
    PrivilegeAgentEntry, PrivilegeEscalationReport, PrivilegePluginEntry,
};

// ---------------------------------------------------------------------------
// HooksReport
// ---------------------------------------------------------------------------

#[test]
fn hooks_report_wire_shape_is_byte_stable() {
    let report = HooksReport {
        plugins: vec![HookPluginEntry {
            catalog: "acme".to_owned(),
            plugin: "plug".to_owned(),
            contributed: vec![HookEventEntry {
                event: "PostToolUse".to_owned(),
                count: 1,
            }],
            missing: vec![HookEventEntry {
                event: "PreToolUse".to_owned(),
                count: 2,
            }],
        }],
    };

    // Top-level field order + nested key order are the contract.
    let json = serde_json::to_string(&report).expect("serialise");
    assert_eq!(
        json,
        r#"{"plugins":[{"catalog":"acme","plugin":"plug","contributed":[{"event":"PostToolUse","count":1}],"missing":[{"event":"PreToolUse","count":2}]}]}"#,
    );
}

#[test]
fn hooks_report_empty_wire_shape() {
    let report = HooksReport { plugins: vec![] };
    let json = serde_json::to_string(&report).expect("serialise");
    assert_eq!(json, r#"{"plugins":[]}"#);
}

// ---------------------------------------------------------------------------
// GuardrailsReport
// ---------------------------------------------------------------------------

#[test]
fn guardrails_report_wire_shape_is_byte_stable() {
    let report = GuardrailsReport {
        files: vec![GuardrailsFileEntry {
            path: std::path::PathBuf::from("/proj/CLAUDE.md"),
            present: vec![
                CatalogPlugin {
                    catalog: "acme".to_owned(),
                    plugin: "plug".to_owned(),
                },
                CatalogPlugin {
                    catalog: "gone".to_owned(),
                    plugin: "ghost".to_owned(),
                },
            ],
            orphaned: vec![CatalogPlugin {
                catalog: "gone".to_owned(),
                plugin: "ghost".to_owned(),
            }],
            suppressed: vec![CatalogPlugin {
                catalog: "acme".to_owned(),
                plugin: "plug".to_owned(),
            }],
        }],
    };

    let json = serde_json::to_value(&report).expect("serialise");
    let expected: Value = json!({
        "files": [
            {
                "path": "/proj/CLAUDE.md",
                "present": [
                    {"catalog": "acme", "plugin": "plug"},
                    {"catalog": "gone", "plugin": "ghost"}
                ],
                "orphaned": [
                    {"catalog": "gone", "plugin": "ghost"}
                ],
                "suppressed": [
                    {"catalog": "acme", "plugin": "plug"}
                ]
            }
        ]
    });
    assert_eq!(json, expected);
}

// ---------------------------------------------------------------------------
// AgentsReport
// ---------------------------------------------------------------------------

#[test]
fn agents_report_wire_shape_is_byte_stable() {
    let report = AgentsReport {
        harnesses: vec![AgentHarnessEntry {
            harness: "claude-code".to_owned(),
            present: vec!["plug__reviewer.md".to_owned()],
            orphaned: vec!["ghost__gone.md".to_owned()],
            dropped_fields: vec![DroppedFieldEntry {
                agent: "plug__reviewer".to_owned(),
                fields: vec!["model".to_owned(), "color".to_owned()],
            }],
        }],
    };

    let json = serde_json::to_string(&report).expect("serialise");
    assert_eq!(
        json,
        r#"{"harnesses":[{"harness":"claude-code","present":["plug__reviewer.md"],"orphaned":["ghost__gone.md"],"dropped_fields":[{"agent":"plug__reviewer","fields":["model","color"]}]}]}"#,
    );
}

// ---------------------------------------------------------------------------
// PrivilegeEscalationReport
// ---------------------------------------------------------------------------

#[test]
fn privilege_escalation_report_wire_shape_is_byte_stable() {
    let report = PrivilegeEscalationReport {
        plugins: vec![PrivilegePluginEntry {
            catalog: "acme".to_owned(),
            plugin: "plug".to_owned(),
            agents: vec![PrivilegeAgentEntry {
                name: "reviewer".to_owned(),
                fields: vec![
                    "hooks".to_owned(),
                    "mcpServers".to_owned(),
                    "permissionMode".to_owned(),
                ],
            }],
        }],
    };

    let json = serde_json::to_string(&report).expect("serialise");
    assert_eq!(
        json,
        r#"{"plugins":[{"catalog":"acme","plugin":"plug","agents":[{"name":"reviewer","fields":["hooks","mcpServers","permissionMode"]}]}]}"#,
    );
}

// ---------------------------------------------------------------------------
// PersonaReport
// ---------------------------------------------------------------------------

#[test]
fn persona_report_wire_shape_is_byte_stable() {
    let report = PersonaReport {
        personas: vec![
            PersonaEntry {
                catalog: "acme".to_owned(),
                plugin: "plug".to_owned(),
                agent_name: "reviewer".to_owned(),
                resolved_persona_name: "reviewer-persona".to_owned(),
                clash_prefixed: false,
            },
            PersonaEntry {
                catalog: "acme".to_owned(),
                plugin: "plug-b".to_owned(),
                agent_name: "reviewer".to_owned(),
                resolved_persona_name: "plug-b-reviewer-persona".to_owned(),
                clash_prefixed: true,
            },
        ],
        drop_persona: "drop-persona".to_owned(),
    };

    let json = serde_json::to_value(&report).expect("serialise");
    let expected: Value = json!({
        "personas": [
            {
                "catalog": "acme",
                "plugin": "plug",
                "agent_name": "reviewer",
                "resolved_persona_name": "reviewer-persona",
                "clash_prefixed": false
            },
            {
                "catalog": "acme",
                "plugin": "plug-b",
                "agent_name": "reviewer",
                "resolved_persona_name": "plug-b-reviewer-persona",
                "clash_prefixed": true
            }
        ],
        "drop_persona": "drop-persona"
    });
    assert_eq!(json, expected);

    // The reserved drop-persona name is always present.
    assert_eq!(json["drop_persona"], "drop-persona");
}
