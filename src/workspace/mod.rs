//! Workspace context. Phase 3 introduced project-bound `.tome/` markers
//! and a per-workspace SQLite index; Phase 4 collapses that model to a
//! single central database (indexed by workspace name in
//! `workspace_skills` / `workspace_catalogs` junction tables) plus a
//! thin `<project>/.tome/config.toml` binding pointer.
//!
//! The `inventory` module (Phase 3's opt-in `workspaces.txt` registry)
//! is gone in Phase 4 — research §R-11 documents the move to the
//! `workspace_projects` table as sole source of truth for bindings.

pub mod info;
pub mod init;
pub mod name;
pub mod resolution;
pub mod scope;

pub use info::{ModelIdentity, ScopeKind, WorkspaceInfo};
pub use init::{InitOutcome, init};
pub use name::WorkspaceName;
pub use scope::{ResolvedScope, Scope, ScopeSource};
