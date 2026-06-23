//! T268 / FR-450 — composition-reference forms are **TOML string values
//! containing brackets**, not TOML table headers.
//!
//! `harnesses = ["[workspace]", "[global]"]` is valid (array of strings).
//! `harnesses = [[workspaces.foo]]` is NOT — that's a TOML array-of-tables
//! header, and the field is typed `Option<Vec<String>>`, so serde refuses.
//! This test pins the refusal so a future refactor doesn't accidentally
//! widen the field type and silently accept the table-header form.
//!
//! Note: `parse_global` was removed in Task 2 / fix-4. The global harness
//! layer is now `HarnessConfig` parsed directly via `toml::from_str`.

use tome::config::HarnessConfig;
use tome::settings::parser;

#[test]
fn harnesses_as_table_header_array_fails_to_parse_global() {
    // `[[workspaces.foo]]` reads as: start an array-of-tables under
    // key path `workspaces.foo`. The TOML parser will treat the body
    // that follows (nothing here) as the (zero-th) table entry, and
    // the eventual deserialisation against `HarnessConfig` either
    // fails on the unknown `workspaces` key (deny_unknown_fields) or
    // on a structure mismatch — either way it must NOT succeed.
    //
    // Task 2 / fix-4: was `parser::parse_global(body)`. Now parsed
    // directly as `HarnessConfig`; the error no longer says "global"
    // (that was the ParseError wrapper layer), but the parse must still
    // fail.
    let body = r#"
harnesses = [[workspaces.foo]]
"#;
    let err = toml::from_str::<HarnessConfig>(body).expect_err("must reject array-of-tables shape");
    let rendered = err.to_string();
    // The exact toml::de::Error text varies; just assert the parse fails.
    assert!(
        !rendered.is_empty(),
        "error message must be non-empty: {rendered}"
    );
}

#[test]
fn harnesses_as_table_header_array_fails_to_parse_workspace() {
    // Same shape mismatch under the workspace settings layer.
    let body = r#"
name = "foo"
harnesses = [[workspaces.bar]]
"#;
    let err = parser::parse_workspace(body).expect_err("must reject array-of-tables shape");
    let rendered = err.to_string();
    assert!(
        rendered.to_lowercase().contains("workspace"),
        "error must name the workspace layer: {rendered}"
    );
}

#[test]
fn harnesses_as_table_header_array_fails_to_parse_project_marker() {
    let body = r#"
workspace = "foo"
harnesses = [[workspaces.bar]]
"#;
    let err = parser::parse_project_marker(body).expect_err("must reject array-of-tables shape");
    let rendered = err.to_string();
    assert!(
        rendered.to_lowercase().contains("project marker"),
        "error must name the project marker layer: {rendered}"
    );
}

#[test]
fn harnesses_as_array_of_strings_parses_cleanly() {
    // Counter-test: the bracket-containing forms used as TOML strings
    // (the actual contract per FR-450) must parse cleanly.
    // Task 2 / fix-4: was `parser::parse_global(body)`. Now parsed
    // directly as `HarnessConfig` with the `enabled` field.
    let body = r#"
enabled = ["[workspace]", "[global]", "[workspaces.foo]", "!cursor", "claude-code"]
"#;
    let parsed: HarnessConfig = toml::from_str(body).expect("must parse string array");
    let list = parsed.enabled.expect("declared");
    assert_eq!(list.len(), 5);
    assert_eq!(list[0], "[workspace]");
    assert_eq!(list[1], "[global]");
    assert_eq!(list[2], "[workspaces.foo]");
    assert_eq!(list[3], "!cursor");
    assert_eq!(list[4], "claude-code");
}
