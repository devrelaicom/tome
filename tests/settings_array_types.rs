//! T268 / FR-450 — composition-reference forms are **TOML string values
//! containing brackets**, not TOML table headers.
//!
//! `harnesses = ["[workspace]", "[global]"]` is valid (array of strings).
//! `harnesses = [[workspaces.foo]]` is NOT — that's a TOML array-of-tables
//! header, and the field is typed `Option<Vec<String>>`, so serde refuses.
//! This test pins the refusal so a future refactor doesn't accidentally
//! widen the field type and silently accept the table-header form.

use tome::settings::parser;

#[test]
fn harnesses_as_table_header_array_fails_to_parse_global() {
    // `[[workspaces.foo]]` reads as: start an array-of-tables under
    // key path `workspaces.foo`. The TOML parser will treat the body
    // that follows (nothing here) as the (zero-th) table entry, and
    // the eventual deserialisation against `GlobalSettings` either
    // fails on the unknown `workspaces` key (deny_unknown_fields) or
    // on a structure mismatch — either way it must NOT succeed.
    let body = r#"
harnesses = [[workspaces.foo]]
"#;
    let err = parser::parse_global(body).expect_err("must reject array-of-tables shape");
    let rendered = err.to_string();
    // The error must be discoverable at the global-settings layer (so
    // the caller's path wrap is meaningful). The exact toml::de::Error
    // text varies across versions, so we assert on stable substrings.
    assert!(
        rendered.to_lowercase().contains("global"),
        "error must name the global layer: {rendered}"
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
    let body = r#"
harnesses = ["[workspace]", "[global]", "[workspaces.foo]", "!cursor", "claude-code"]
"#;
    let parsed = parser::parse_global(body).expect("must parse string array");
    let list = parsed.harnesses.expect("declared");
    assert_eq!(list.len(), 5);
    assert_eq!(list[0], "[workspace]");
    assert_eq!(list[1], "[global]");
    assert_eq!(list[2], "[workspaces.foo]");
    assert_eq!(list[3], "!cursor");
    assert_eq!(list[4], "claude-code");
}
