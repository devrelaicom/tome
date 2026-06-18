//! Consolidated integration-test binary for the `harness_settings` surface.
//!
//! The former top-level `tests/<name>.rs` files are submodules under
//! `tests/harness_settings/`, sharing ONE compiled + linked binary instead of N. This
//! collapses the per-file static-link + process-spawn overhead to one per
//! group. Test names gain a `<name>::` module prefix, so `cargo test
//! <name>::` still filters by file and `cargo test --test harness_settings` runs the group.

mod common;

#[path = "harness_settings/codex_session_hook.rs"]
mod codex_session_hook;
#[path = "harness_settings/guardrails_marker_injection.rs"]
mod guardrails_marker_injection;
#[path = "harness_settings/guardrails_render.rs"]
mod guardrails_render;
#[path = "harness_settings/guardrails_suppression.rs"]
mod guardrails_suppression;
#[path = "harness_settings/harness_bare.rs"]
mod harness_bare;
#[path = "harness_settings/harness_info.rs"]
mod harness_info;
#[cfg(unix)]
#[path = "harness_settings/harness_json_shape.rs"]
mod harness_json_shape;
#[path = "harness_settings/harness_list_as_written.rs"]
mod harness_list_as_written;
#[path = "harness_settings/harness_list_effective.rs"]
mod harness_list_effective;
#[path = "harness_settings/harness_list_json_shape.rs"]
mod harness_list_json_shape;
#[path = "harness_settings/harness_module_claude_code.rs"]
mod harness_module_claude_code;
#[path = "harness_settings/harness_modules.rs"]
mod harness_modules;
#[path = "harness_settings/harness_open_plugins_bundle.rs"]
mod harness_open_plugins_bundle;
#[path = "harness_settings/harness_p11_pins.rs"]
mod harness_p11_pins;
#[path = "harness_settings/harness_remove_scope.rs"]
mod harness_remove_scope;
#[path = "harness_settings/harness_skeleton.rs"]
mod harness_skeleton;
#[path = "harness_settings/harness_sync.rs"]
mod harness_sync;
#[path = "harness_settings/harness_sync_mass_delete_safeguard.rs"]
mod harness_sync_mass_delete_safeguard;
#[path = "harness_settings/harness_sync_p6_first_error.rs"]
mod harness_sync_p6_first_error;
#[path = "harness_settings/harness_sync_p6_idempotence.rs"]
mod harness_sync_p6_idempotence;
#[path = "harness_settings/harness_sync_stub.rs"]
mod harness_sync_stub;
#[path = "harness_settings/harness_trait_p6.rs"]
mod harness_trait_p6;
#[path = "harness_settings/harness_ts_plugin_shim.rs"]
mod harness_ts_plugin_shim;
#[path = "harness_settings/harness_use_manual_mcp.rs"]
mod harness_use_manual_mcp;
#[path = "harness_settings/harness_use_scope.rs"]
mod harness_use_scope;
#[path = "harness_settings/harness_use_selection.rs"]
mod harness_use_selection;
#[path = "harness_settings/hooks_merge.rs"]
mod hooks_merge;
#[path = "harness_settings/hooks_rewrite.rs"]
mod hooks_rewrite;
#[path = "harness_settings/rules_file_block_in_existing.rs"]
mod rules_file_block_in_existing;
#[path = "harness_settings/rules_file_claude_correction.rs"]
mod rules_file_claude_correction;
#[path = "harness_settings/rules_file_standalone.rs"]
mod rules_file_standalone;
#[path = "harness_settings/rules_opencode_inline.rs"]
mod rules_opencode_inline;
#[path = "harness_settings/session_start.rs"]
mod session_start;
#[path = "harness_settings/session_start_hook.rs"]
mod session_start_hook;
#[path = "harness_settings/settings_array_types.rs"]
mod settings_array_types;
#[path = "harness_settings/settings_bad_exclusion.rs"]
mod settings_bad_exclusion;
#[path = "harness_settings/settings_composition.rs"]
mod settings_composition;
#[path = "harness_settings/settings_composition_resolves_to_as_written.rs"]
mod settings_composition_resolves_to_as_written;
#[path = "harness_settings/settings_cycle_detection.rs"]
mod settings_cycle_detection;
#[path = "harness_settings/settings_harness_not_supported.rs"]
mod settings_harness_not_supported;
#[path = "harness_settings/settings_p6.rs"]
mod settings_p6;
#[path = "harness_settings/settings_priority.rs"]
mod settings_priority;
#[path = "harness_settings/settings_skeleton.rs"]
mod settings_skeleton;
#[path = "harness_settings/settings_unknown_workspace_resolver.rs"]
mod settings_unknown_workspace_resolver;
#[path = "harness_settings/settings_workspace_ref_outside_project.rs"]
mod settings_workspace_ref_outside_project;
#[path = "harness_settings/sync_algorithm.rs"]
mod sync_algorithm;
#[path = "harness_settings/sync_boundary.rs"]
mod sync_boundary;
#[path = "harness_settings/sync_idempotence.rs"]
mod sync_idempotence;
#[path = "harness_settings/sync_outcome_json_shape.rs"]
mod sync_outcome_json_shape;
