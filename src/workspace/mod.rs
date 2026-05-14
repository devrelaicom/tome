//! Workspace context. Phase 3 introduces the notion of a project-local
//! Tome install marked by a `.tome/` directory at a workspace root, in
//! addition to the existing global install under XDG dirs.
//!
//! Resolution (slice F3), `tome workspace info` / `tome workspace init`
//! (US2), and the opt-in `${state_dir}/workspaces.txt` registry
//! (research §R-15) all live under this module.

pub mod info;
pub mod init;
pub mod inventory;
pub mod resolution;
pub mod scope;

pub use info::{ModelIdentity, ScopeKind, WorkspaceInfo};
pub use init::{InitOutcome, init};
pub use scope::{ResolvedScope, Scope, ScopeSource};
