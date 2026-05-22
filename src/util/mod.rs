//! Cross-cutting utility helpers shared across capability modules.
//!
//! Phase 4 introduces `src/util/` as the home for small, focused helpers
//! that aren't a natural fit for any one capability module. The atomic
//! populated-directory helper (`atomic_dir`) is the first inhabitant; it
//! lifts the Phase 3 `workspace::init` pattern (stage, populate, rename)
//! into a reusable shape for Phase 4's `workspace add`, `workspace
//! rename`, and `workspace use` (project marker creation).
//!
//! Sync-only — `tests/sync_boundary.rs` enforces the constitution's sync
//! discipline on this tree. The single async island lives elsewhere.

pub mod atomic_dir;

pub use atomic_dir::{land_directory, land_directory_with_replace};
