//! T-M6: byte-stable JSON wire-shape pins for `tome harness list`.
//!
//! Pins both modes of [`tome::commands::harness::list::HarnessListOutcome`]:
//!
//! - `Effective` — used when `tome harness list` is invoked with no
//!   positional argument; the resolver returns the effective
//!   composition. Each `EffectiveEntry` carries `name` + `source_chain`
//!   (mixed bracketed-and-plain notation per
//!   `contracts/settings-composition.md`).
//! - `AsWritten` — used when `tome harness list <workspace>` is invoked
//!   with a positional workspace name; the workspace's directly-declared
//!   list is reported verbatim (no composition expansion).
//!
//! The enum is `#[serde(tag = "mode", rename_all = "snake_case")]` —
//! internally tagged with a `mode` discriminator. The discriminator key
//! is part of the wire contract; pin it explicitly here so a future
//! refactor that switches to externally-tagged or untagged form breaks
//! loudly.
//!
//! Wire shape is pinned by `contracts/harness-commands.md` § `list`.

use tome::commands::harness::list::{EffectiveEntry, HarnessListOutcome};

#[test]
fn effective_entry_json_wire_shape_is_byte_stable() {
    let entry = EffectiveEntry {
        name: "claude-code".to_owned(),
        source_chain: vec!["project".to_owned(), "[global]".to_owned()],
    };
    let json = serde_json::to_string(&entry).expect("serialise");
    assert_eq!(
        json,
        r#"{"name":"claude-code","source_chain":["project","[global]"]}"#,
    );
}

#[test]
fn effective_entry_field_order_is_pinned() {
    let entry = EffectiveEntry {
        name: "x".to_owned(),
        source_chain: vec!["project".to_owned()],
    };
    let json = serde_json::to_string(&entry).expect("serialise");
    let name_idx = json.find("\"name\"").expect("name field present");
    let chain_idx = json.find("\"source_chain\"").expect("source_chain present");
    assert!(name_idx < chain_idx, "name must come before source_chain");
}

#[test]
fn harness_list_effective_json_wire_shape_is_byte_stable() {
    let outcome = HarnessListOutcome::Effective {
        harnesses: vec![
            EffectiveEntry {
                name: "claude-code".to_owned(),
                source_chain: vec!["project".to_owned()],
            },
            EffectiveEntry {
                name: "codex".to_owned(),
                source_chain: vec!["workspace".to_owned(), "[global]".to_owned()],
            },
        ],
        excluded: vec!["cursor".to_owned()],
    };
    let json = serde_json::to_string(&outcome).expect("serialise");
    assert_eq!(
        json,
        r#"{"mode":"effective","harnesses":[{"name":"claude-code","source_chain":["project"]},{"name":"codex","source_chain":["workspace","[global]"]}],"excluded":["cursor"]}"#,
    );
}

#[test]
fn harness_list_as_written_json_wire_shape_is_byte_stable() {
    let outcome = HarnessListOutcome::AsWritten {
        workspace: "my-ws".to_owned(),
        harnesses: vec!["claude-code".to_owned(), "codex".to_owned()],
    };
    let json = serde_json::to_string(&outcome).expect("serialise");
    assert_eq!(
        json,
        r#"{"mode":"as_written","workspace":"my-ws","harnesses":["claude-code","codex"]}"#,
    );
}

#[test]
fn harness_list_mode_discriminator_is_present_for_both_variants() {
    // Internally-tagged enums put the discriminator as the first key;
    // assert both modes serialise with `mode` as the leading field.
    let effective = HarnessListOutcome::Effective {
        harnesses: Vec::new(),
        excluded: Vec::new(),
    };
    let effective_json = serde_json::to_string(&effective).unwrap();
    assert!(
        effective_json.starts_with(r#"{"mode":"effective""#),
        "effective json={effective_json}",
    );

    let as_written = HarnessListOutcome::AsWritten {
        workspace: String::new(),
        harnesses: Vec::new(),
    };
    let as_written_json = serde_json::to_string(&as_written).unwrap();
    assert!(
        as_written_json.starts_with(r#"{"mode":"as_written""#),
        "as_written json={as_written_json}",
    );
}

#[test]
fn harness_list_effective_empty_collections_serialise() {
    // Empty harnesses + empty excluded still emit the keys (not skipped)
    // — downstream consumers read `excluded.is_empty()` rather than
    // checking key absence.
    let outcome = HarnessListOutcome::Effective {
        harnesses: Vec::new(),
        excluded: Vec::new(),
    };
    let json = serde_json::to_string(&outcome).unwrap();
    assert_eq!(json, r#"{"mode":"effective","harnesses":[],"excluded":[]}"#,);
}
