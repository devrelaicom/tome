//! Consolidated integration-test binary for the `workspace` surface.
//!
//! The former top-level `tests/<name>.rs` files are submodules under
//! `tests/workspace/`, sharing ONE compiled + linked binary instead of N. This
//! collapses the per-file static-link + process-spawn overhead to one per
//! group. Test names gain a `<name>::` module prefix, so `cargo test
//! <name>::` still filters by file and `cargo test --test workspace` runs the group.

mod common;

#[path = "workspace/atomic_dir.rs"]
mod atomic_dir;
#[path = "workspace/atomicity.rs"]
mod atomicity;
#[path = "workspace/atomicity_enable.rs"]
mod atomicity_enable;
#[path = "workspace/bounded_reads.rs"]
mod bounded_reads;
#[path = "workspace/concurrency.rs"]
mod concurrency;
#[path = "workspace/paths_phase2.rs"]
mod paths_phase2;
#[path = "workspace/paths_phase3.rs"]
mod paths_phase3;
#[path = "workspace/workspace_commands.rs"]
mod workspace_commands;
#[path = "workspace/workspace_info.rs"]
mod workspace_info;
#[path = "workspace/workspace_init.rs"]
mod workspace_init;
#[path = "workspace/workspace_init_json_shape.rs"]
mod workspace_init_json_shape;
#[path = "workspace/workspace_list.rs"]
mod workspace_list;
#[path = "workspace/workspace_name.rs"]
mod workspace_name;
#[path = "workspace/workspace_regen_summary.rs"]
mod workspace_regen_summary;
#[path = "workspace/workspace_regen_summary_json_shape.rs"]
mod workspace_regen_summary_json_shape;
#[path = "workspace/workspace_remove.rs"]
mod workspace_remove;
#[path = "workspace/workspace_remove_cascade.rs"]
mod workspace_remove_cascade;
#[path = "workspace/workspace_remove_json_shape.rs"]
mod workspace_remove_json_shape;
#[path = "workspace/workspace_rename.rs"]
mod workspace_rename;
#[path = "workspace/workspace_rename_json_shape.rs"]
mod workspace_rename_json_shape;
#[path = "workspace/workspace_resolution.rs"]
mod workspace_resolution;
#[path = "workspace/workspace_sync.rs"]
mod workspace_sync;
#[path = "workspace/workspace_toml_control_chars.rs"]
mod workspace_toml_control_chars;
#[path = "workspace/workspace_use_atomicity.rs"]
mod workspace_use_atomicity;
#[path = "workspace/workspace_use_binding.rs"]
mod workspace_use_binding;
#[path = "workspace/workspace_use_claude_code_e2e.rs"]
mod workspace_use_claude_code_e2e;
#[path = "workspace/workspace_use_concurrent.rs"]
mod workspace_use_concurrent;
#[path = "workspace/workspace_use_cross_product.rs"]
mod workspace_use_cross_product;
#[path = "workspace/workspace_use_forward_progress.rs"]
mod workspace_use_forward_progress;
#[cfg(unix)]
#[path = "workspace/workspace_use_json_shape.rs"]
mod workspace_use_json_shape;
