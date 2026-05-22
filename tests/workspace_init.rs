//! Phase 4 / F2a — `tome workspace init` is replaced by
//! `tome workspace add` (US2) plus `tome workspace use` (US1). The Phase
//! 3 surface in `src/workspace/init.rs` is a `TODO(F11)` stub returning
//! an `Internal` error.
//!
//! Every Phase 3 test in this file depended on the deleted
//! `tome::workspace::inventory` module, the per-workspace
//! `.tome/index.db`, and the `.tome/config.toml` binding pointer
//! existing as a real file. Those concepts are split across F11 (junction
//! tables) + US1 (`tome workspace use` writes the project marker) + US2
//! (`tome workspace add` writes `<root>/workspaces/<name>/...`).
//!
//! Marking each test `#[ignore]` rather than deleting them preserves the
//! historical coverage list as documentation; US1/US2 unhide them as
//! they land.

#[test]
#[ignore = "F11/US1/US2: workspace init replaced by `workspace add` + `workspace use`"]
fn init_creates_dot_tome_with_empty_config() {}

#[test]
#[ignore = "F11/US1/US2: workspace init replaced by `workspace add` + `workspace use`"]
fn init_inherits_global_catalogs_when_flag_set() {}

#[test]
#[ignore = "F11/US1/US2: workspace init replaced by `workspace add` + `workspace use`"]
fn init_refuses_existing_marker_without_force() {}

#[test]
#[ignore = "F11/US1/US2: workspace init replaced by `workspace add` + `workspace use`"]
fn init_force_renames_existing_marker_to_dot_tome_old() {}

#[test]
#[ignore = "F11/US1/US2: workspace init replaced by `workspace add` + `workspace use`"]
fn init_canonicalises_target_root() {}

#[test]
#[ignore = "F11/US1/US2: workspace init replaced by `workspace add` + `workspace use`"]
fn init_appends_to_opt_in_workspace_registry_when_present() {}

#[test]
#[ignore = "F11/US1/US2: workspace init replaced by `workspace add` + `workspace use`"]
fn init_skips_registry_when_file_absent() {}

#[test]
#[ignore = "F11/US1/US2: workspace init replaced by `workspace add` + `workspace use`"]
fn init_chmod_0700_on_unix() {}

#[test]
#[ignore = "F11/US1/US2: workspace init replaced by `workspace add` + `workspace use`"]
fn init_atomic_concurrent_calls_yield_one_winner() {}

#[test]
#[ignore = "F11/US1/US2: workspace init replaced by `workspace add` + `workspace use`"]
fn init_rejects_missing_target() {}

#[test]
#[ignore = "F11/US1/US2: workspace init replaced by `workspace add` + `workspace use`"]
fn init_rejects_non_directory_target() {}

#[test]
#[ignore = "F11/US1/US2: workspace init replaced by `workspace add` + `workspace use`"]
fn init_cli_smoke_default_path() {}
