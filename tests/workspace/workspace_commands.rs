//! Phase 3 / US3 — per-command scope isolation cross-product.
//!
//! The Phase 3 isolation contract was: each workspace owns its own
//! `<workspace>/.tome/config.toml` + `index.db`; `--global` mutations
//! land in `${XDG_CONFIG_HOME}/tome/`. Phase 4 / F2a collapses both
//! cases onto a single global `config.toml` + central `index.db`; the
//! per-workspace isolation discipline migrates to the central DB's
//! `workspace_catalogs` / `workspace_skills` junction tables (F11).
//!
//! Every assertion below was written against the Phase 3 layout and
//! cannot survive the path collapse. They are kept as `#[ignore]`d
//! markers so F11 can rewrite each against the new isolation
//! mechanism without losing the regression coverage list.

#[test]
#[ignore = "F11: per-workspace isolation moves to workspace_catalogs/workspace_skills junction tables"]
fn catalog_add_lands_in_workspace_only_not_global() {}

#[test]
#[ignore = "F11: per-workspace isolation moves to workspace_catalogs/workspace_skills junction tables"]
fn catalog_add_with_global_flag_lands_in_global_only() {}

#[test]
#[ignore = "F11: per-workspace isolation moves to workspace_catalogs/workspace_skills junction tables"]
fn plugin_enable_writes_workspace_index_only() {}

#[test]
#[ignore = "F11: per-workspace isolation moves to workspace_catalogs/workspace_skills junction tables"]
fn plugin_enable_with_global_flag_writes_global_index_only() {}

#[test]
#[ignore = "F11: per-workspace isolation moves to workspace_catalogs/workspace_skills junction tables"]
fn plugin_list_in_workspace_reads_workspace_db() {}

#[test]
#[ignore = "F11: per-workspace isolation moves to workspace_catalogs/workspace_skills junction tables"]
fn plugin_list_with_global_flag_reads_global_db() {}

#[test]
#[ignore = "F11: per-workspace isolation moves to workspace_catalogs/workspace_skills junction tables"]
fn workspace_paths_object_partitions_index_db_lock_and_config_file() {}

#[test]
#[ignore = "F11: per-workspace isolation moves to workspace_catalogs/workspace_skills junction tables"]
fn fresh_workspace_init_bootstraps_dot_tome_on_first_write() {}

#[test]
#[ignore = "F11: per-workspace isolation moves to workspace_catalogs/workspace_skills junction tables"]
fn catalog_list_inside_workspace_walk_finds_workspace_catalog() {}
