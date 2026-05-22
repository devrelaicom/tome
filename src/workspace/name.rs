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
//! [`WorkspaceName::global`] sentinel for internal wiring. F10 adds
//! `parse(&str) -> Result<Self, TomeError>` plus the serde impls.

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorkspaceName(String);

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
}
