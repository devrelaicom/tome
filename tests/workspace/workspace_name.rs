//! Phase 4 / F10: `WorkspaceName::parse` enforces FR-347 at every input
//! boundary (CLI flag, env var, project marker, settings TOML). Every
//! invalid form maps to `TomeError::WorkspaceNameInvalid` (exit 15);
//! reserved-name semantics are surfaced via `is_reserved()`.

use serde::Deserialize;
use tome::error::TomeError;
use tome::workspace::WorkspaceName;

// ---- Valid forms --------------------------------------------------------

#[test]
fn accepts_simple_names() {
    for ok in ["a", "Foo", "foo-bar_baz", "abc123", "X", "global", "z9"] {
        assert!(
            WorkspaceName::parse(ok).is_ok(),
            "{ok:?} should parse, got {:?}",
            WorkspaceName::parse(ok).err(),
        );
    }
}

#[test]
fn accepts_max_length_64_chars() {
    let max = "a".repeat(64);
    assert!(WorkspaceName::parse(&max).is_ok());
}

#[test]
fn accepts_internal_dashes_and_underscores() {
    for ok in ["a-b-c", "a_b_c", "a-b_c", "x1y2-z3"] {
        assert!(WorkspaceName::parse(ok).is_ok(), "{ok:?}");
    }
}

// ---- Invalid forms ------------------------------------------------------

fn assert_invalid(name: &str) -> String {
    let err = WorkspaceName::parse(name).unwrap_err();
    assert_eq!(err.exit_code(), 15, "{name:?} should exit 15");
    match err {
        TomeError::WorkspaceNameInvalid { name: n, reason } => {
            assert_eq!(n, name);
            reason
        }
        other => panic!("expected WorkspaceNameInvalid for {name:?}, got {other:?}"),
    }
}

#[test]
fn rejects_empty_string() {
    let reason = assert_invalid("");
    assert!(reason.contains("empty"), "{reason}");
}

#[test]
fn rejects_dot_literal() {
    let reason = assert_invalid(".");
    assert!(reason.contains("`.`"), "{reason}");
}

#[test]
fn rejects_double_dot_literal() {
    let reason = assert_invalid("..");
    assert!(reason.contains("`..`"), "{reason}");
}

#[test]
fn rejects_length_over_64() {
    let too_long = "a".repeat(65);
    let reason = assert_invalid(&too_long);
    assert!(reason.contains("65"), "{reason}");
    assert!(reason.contains("64"), "{reason}");
}

#[test]
fn rejects_leading_dash() {
    let reason = assert_invalid("-foo");
    assert!(reason.contains("begins with"), "{reason}");
}

#[test]
fn rejects_leading_underscore() {
    let reason = assert_invalid("_foo");
    assert!(reason.contains("begins with"), "{reason}");
}

#[test]
fn rejects_trailing_dash() {
    let reason = assert_invalid("foo-");
    assert!(reason.contains("ends with"), "{reason}");
}

#[test]
fn rejects_trailing_underscore() {
    let reason = assert_invalid("foo_");
    assert!(reason.contains("ends with"), "{reason}");
}

#[test]
fn rejects_slash() {
    let reason = assert_invalid("foo/bar");
    assert!(reason.contains("invalid character"), "{reason}");
    assert!(reason.contains("`/`"), "{reason}");
}

#[test]
fn rejects_space() {
    let reason = assert_invalid("foo bar");
    assert!(reason.contains("invalid character"), "{reason}");
}

#[test]
fn rejects_non_ascii() {
    let reason = assert_invalid("föo");
    assert!(reason.contains("invalid character"), "{reason}");
}

#[test]
fn rejects_dot_in_middle() {
    let reason = assert_invalid("a.b");
    assert!(reason.contains("invalid character"), "{reason}");
}

// ---- Reserved-name semantics -------------------------------------------

#[test]
fn global_name_parses_but_is_reserved() {
    let n = WorkspaceName::parse("global").unwrap();
    assert!(n.is_reserved());
    assert_eq!(n.as_str(), "global");
}

#[test]
fn non_reserved_names_are_not_reserved() {
    for ok in ["foo", "global-2", "global_2", "myproj"] {
        let n = WorkspaceName::parse(ok).unwrap();
        assert!(!n.is_reserved(), "{ok:?} should not be reserved");
    }
}

#[test]
fn global_constructor_matches_constant() {
    assert_eq!(WorkspaceName::global().as_str(), WorkspaceName::GLOBAL);
    assert!(WorkspaceName::global().is_reserved());
}

// ---- TOML round-trip ----------------------------------------------------

#[derive(Debug, Deserialize)]
struct Wrap {
    workspace: WorkspaceName,
}

#[test]
fn toml_deserialise_accepts_valid_name() {
    let parsed: Wrap = toml::from_str("workspace = \"foo-bar\"").expect("valid toml");
    assert_eq!(parsed.workspace.as_str(), "foo-bar");
}

#[test]
fn toml_deserialise_rejects_invalid_name() {
    let err = toml::from_str::<Wrap>("workspace = \"bad name\"").expect_err("invalid");
    // The serde error wraps our reason string.
    let msg = err.to_string();
    assert!(msg.contains("bad name") || msg.contains("invalid"), "{msg}");
}

#[test]
fn toml_deserialise_rejects_dot_literal() {
    let err = toml::from_str::<Wrap>("workspace = \".\"").expect_err("invalid");
    let msg = err.to_string();
    assert!(msg.contains('.') || msg.contains("reserved"), "{msg}");
}
