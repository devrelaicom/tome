//! `WorkspaceName` — a validated workspace identifier.
//!
//! F2a ships the **minimal** shape required to compile the new
//! [`Paths`](crate::paths::Paths) accessors that take `&WorkspaceName`.
//! The full version (FR-347 validation, `parse`, `Serialize`/`Deserialize`,
//! reserved-name discipline) lands in slice F10 alongside the `Scope`
//! reshape.
//!
//! Until F10, callers cannot construct a `WorkspaceName` outside this
//! crate (no public constructor) — F2a uses only the privileged
//! [`WorkspaceName::global`] sentinel for internal wiring. F8 extends
//! the type with **skeleton-grade** `Serialize`/`Deserialize` impls so
//! the `src/settings/` parser can round-trip workspace names through
//! TOML; the inner string is wrapped/unwrapped verbatim without
//! validation. F10 swaps the inner deserialisation for the validating
//! `parse(&str) -> Result<Self, TomeError>`.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorkspaceName(pub(crate) String);

impl WorkspaceName {
    /// The privileged default workspace name, seeded on first bootstrap.
    pub const GLOBAL: &'static str = "global";

    /// The privileged default workspace as a value. Used by the Phase 4
    /// path accessors before [`WorkspaceName::parse`] (F10) lands.
    pub fn global() -> Self {
        Self(Self::GLOBAL.to_owned())
    }

    /// Borrow the underlying string. Always valid by construction.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// **F8 skeleton constructor** — wraps an arbitrary string without
    /// validation. Documented hidden because it is intended for serde
    /// internals + integration-test fixtures only; F10's `parse(&str)`
    /// becomes the validating public constructor.
    #[doc(hidden)]
    pub fn from_string_unvalidated(s: String) -> Self {
        Self(s)
    }
}

/// **F8 skeleton impl** — deserialises a TOML string into a `WorkspaceName`
/// **without validation**. F10 replaces this with `parse(&str)` semantics
/// that enforce FR-347 (length cap, character set, reserved-name rules).
/// Callers in F8 should NOT rely on the absence of validation; the
/// serde-driven settings parser exists so US3 can layer in additional
/// invariants without re-plumbing the trait bound.
impl<'de> Deserialize<'de> for WorkspaceName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        // TODO(F10): replace with `WorkspaceName::parse(&raw)` to enforce
        // FR-347 character/length/reserved-name rules at the serde
        // boundary.
        Ok(WorkspaceName(raw))
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
