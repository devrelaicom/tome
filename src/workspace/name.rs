//! `WorkspaceName` — a validated workspace identifier.
//!
//! Phase 4 / Slice F10 lands the validating [`WorkspaceName::parse`] —
//! every input that flows through serde, env vars, CLI flags, or
//! project markers passes through it. The character set, length cap,
//! and reserved-name rules come from FR-347.
//!
//! Rules:
//! - 1..=64 chars from `[a-zA-Z0-9_-]`.
//! - Must not begin or end with `-` or `_`.
//! - Must not be `.`, `..`, or empty.
//!
//! The privileged `"global"` name passes [`WorkspaceName::parse`] but is
//! flagged by [`WorkspaceName::is_reserved`]; the workspace-lifecycle
//! commands (US2's `tome workspace add` / `rename`) refuse it explicitly
//! so a user can never shadow the privileged default.

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

use crate::error::TomeError;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorkspaceName(String);

impl WorkspaceName {
    /// The privileged default workspace name, seeded on first bootstrap.
    pub const GLOBAL: &'static str = "global";

    /// Maximum allowed length, in chars.
    pub const MAX_LEN: usize = 64;

    /// Validate `s` per FR-347. On success returns a freshly-allocated
    /// owned `WorkspaceName`. On failure emits
    /// [`TomeError::WorkspaceNameInvalid`] (exit code 15) with a
    /// human-readable `reason` for the human/error envelope.
    pub fn parse(s: &str) -> Result<Self, TomeError> {
        if s.is_empty() {
            return Err(invalid(s, "is empty"));
        }
        if s == "." {
            return Err(invalid(s, "is reserved literal `.`"));
        }
        if s == ".." {
            return Err(invalid(s, "is reserved literal `..`"));
        }
        let len = s.chars().count();
        if len > Self::MAX_LEN {
            return Err(invalid(
                s,
                &format!("is {len} chars; max is {}", Self::MAX_LEN),
            ));
        }
        for (idx, ch) in s.chars().enumerate() {
            if !is_allowed_char(ch) {
                return Err(invalid(
                    s,
                    &format!("contains invalid character `{ch}` at position {idx}"),
                ));
            }
        }
        // `unwrap()` here is safe: `s.is_empty()` already returned above,
        // so `chars()` yields at least one element.
        let first = s.chars().next().unwrap();
        if first == '-' || first == '_' {
            return Err(invalid(
                s,
                &format!("begins with `{first}`; must be a letter or digit"),
            ));
        }
        let last = s.chars().next_back().unwrap();
        if last == '-' || last == '_' {
            return Err(invalid(
                s,
                &format!("ends with `{last}`; must be a letter or digit"),
            ));
        }
        Ok(Self(s.to_owned()))
    }

    /// Borrow the underlying string. Always valid by construction.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// True iff the name is the privileged default (`"global"`). The
    /// workspace-lifecycle commands consult this to refuse user-driven
    /// creation / rename / deletion of the reserved name.
    pub fn is_reserved(&self) -> bool {
        self.0 == Self::GLOBAL
    }

    /// The privileged default workspace. Infallible because the inner
    /// string is a compile-time constant that satisfies [`Self::parse`].
    pub fn global() -> Self {
        Self(Self::GLOBAL.to_owned())
    }
}

fn invalid(name: &str, reason: &str) -> TomeError {
    TomeError::WorkspaceNameInvalid {
        name: name.to_owned(),
        reason: reason.to_owned(),
    }
}

fn is_allowed_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'
}

impl<'de> Deserialize<'de> for WorkspaceName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        WorkspaceName::parse(&raw).map_err(|e| D::Error::custom(e.to_string()))
    }
}

impl Serialize for WorkspaceName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_names_parse() {
        for ok in [
            "a",
            "Foo",
            "foo-bar_baz",
            "abc123",
            "global",
            "X",
            "a-b-c",
            "z9",
            "_no", // will fail — leading underscore
        ]
        .iter()
        .take(8)
        {
            assert!(WorkspaceName::parse(ok).is_ok(), "{ok:?} should parse");
        }
    }

    #[test]
    fn max_length_boundary() {
        let ok = "a".repeat(64);
        assert!(WorkspaceName::parse(&ok).is_ok());
        let too_long = "a".repeat(65);
        let err = WorkspaceName::parse(&too_long).unwrap_err();
        match err {
            TomeError::WorkspaceNameInvalid { reason, .. } => {
                assert!(reason.contains("65"), "{reason}");
            }
            other => panic!("expected WorkspaceNameInvalid, got {other:?}"),
        }
    }

    #[test]
    fn rejects_empty_and_dot_literals() {
        for bad in ["", ".", ".."] {
            let err = WorkspaceName::parse(bad).unwrap_err();
            assert!(matches!(err, TomeError::WorkspaceNameInvalid { .. }));
            assert_eq!(err.exit_code(), 15);
        }
    }

    #[test]
    fn rejects_leading_underscore_or_dash() {
        for bad in ["_foo", "-foo"] {
            let err = WorkspaceName::parse(bad).unwrap_err();
            assert!(matches!(err, TomeError::WorkspaceNameInvalid { .. }));
        }
    }

    #[test]
    fn rejects_trailing_underscore_or_dash() {
        for bad in ["foo_", "foo-"] {
            let err = WorkspaceName::parse(bad).unwrap_err();
            assert!(matches!(err, TomeError::WorkspaceNameInvalid { .. }));
        }
    }

    #[test]
    fn rejects_invalid_chars() {
        for bad in ["foo/bar", "foo bar", "foo.bar", "föo", "a*b"] {
            let err = WorkspaceName::parse(bad).unwrap_err();
            assert!(
                matches!(err, TomeError::WorkspaceNameInvalid { .. }),
                "{bad:?}",
            );
        }
    }

    #[test]
    fn reserved_name_parses_but_is_flagged() {
        let n = WorkspaceName::parse("global").unwrap();
        assert!(n.is_reserved());
        let other = WorkspaceName::parse("not-global").unwrap();
        assert!(!other.is_reserved());
    }

    #[test]
    fn global_constructor_is_reserved() {
        let n = WorkspaceName::global();
        assert!(n.is_reserved());
        assert_eq!(n.as_str(), "global");
    }
}
