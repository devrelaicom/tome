//! Consolidated integration-test binary for the `catalog_plugin` surface.
//!
//! The former top-level `tests/<name>.rs` files are submodules under
//! `tests/catalog_plugin/`, sharing ONE compiled + linked binary instead of N. This
//! collapses the per-file static-link + process-spawn overhead to one per
//! group. Test names gain a `<name>::` module prefix, so `cargo test
//! <name>::` still filters by file and `cargo test --test catalog_plugin` runs the group.

mod common;

#[path = "catalog_plugin/catalog_add.rs"]
mod catalog_add;
#[path = "catalog_plugin/catalog_list.rs"]
mod catalog_list;
#[path = "catalog_plugin/catalog_remove.rs"]
mod catalog_remove;
#[path = "catalog_plugin/catalog_remove_cascade.rs"]
mod catalog_remove_cascade;
#[path = "catalog_plugin/catalog_remove_toctou.rs"]
mod catalog_remove_toctou;
#[path = "catalog_plugin/catalog_show.rs"]
mod catalog_show;
#[path = "catalog_plugin/catalog_ssh_roundtrip.rs"]
mod catalog_ssh_roundtrip;
#[path = "catalog_plugin/catalog_update.rs"]
mod catalog_update;
#[path = "catalog_plugin/catalog_update_cross_workspace_reindex.rs"]
mod catalog_update_cross_workspace_reindex;
#[path = "catalog_plugin/catalog_update_reindex.rs"]
mod catalog_update_reindex;
#[path = "catalog_plugin/catalog_workspace_refcount.rs"]
mod catalog_workspace_refcount;
#[path = "catalog_plugin/plugin_cheap_reenable_across_workspaces.rs"]
mod plugin_cheap_reenable_across_workspaces;
#[path = "catalog_plugin/plugin_disable.rs"]
mod plugin_disable;
#[path = "catalog_plugin/plugin_enable.rs"]
mod plugin_enable;
#[path = "catalog_plugin/plugin_interactive.rs"]
mod plugin_interactive;
#[path = "catalog_plugin/plugin_list.rs"]
mod plugin_list;
#[path = "catalog_plugin/plugin_repeated.rs"]
mod plugin_repeated;
#[path = "catalog_plugin/plugin_resolve_from_db.rs"]
mod plugin_resolve_from_db;
#[path = "catalog_plugin/plugin_show.rs"]
mod plugin_show;
#[path = "catalog_plugin/plugin_show_p5.rs"]
mod plugin_show_p5;
#[path = "catalog_plugin/plugin_show_p5_json_shape.rs"]
mod plugin_show_p5_json_shape;
#[path = "catalog_plugin/plugin_show_p6.rs"]
mod plugin_show_p6;
#[path = "catalog_plugin/plugin_show_p6_json_shape.rs"]
mod plugin_show_p6_json_shape;
#[path = "catalog_plugin/plugin_summariser_forward_progress.rs"]
mod plugin_summariser_forward_progress;
#[path = "catalog_plugin/plugin_workspace_skills.rs"]
mod plugin_workspace_skills;
