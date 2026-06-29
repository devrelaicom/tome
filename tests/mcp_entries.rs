//! Consolidated integration-test binary for the `mcp_entries` surface.
//!
//! The former top-level `tests/<name>.rs` files are submodules under
//! `tests/mcp_entries/`, sharing ONE compiled + linked binary instead of N. This
//! collapses the per-file static-link + process-spawn overhead to one per
//! group. Test names gain a `<name>::` module prefix, so `cargo test
//! <name>::` still filters by file and `cargo test --test mcp_entries` runs the group.

mod common;

#[path = "mcp_entries/agent_naming_clash.rs"]
mod agent_naming_clash;
#[path = "mcp_entries/agent_path_traversal.rs"]
mod agent_path_traversal;
#[path = "mcp_entries/agent_privilege.rs"]
mod agent_privilege;
#[path = "mcp_entries/agent_removal.rs"]
mod agent_removal;
#[path = "mcp_entries/agent_translate_claude_code.rs"]
mod agent_translate_claude_code;
#[path = "mcp_entries/agent_translate_codex.rs"]
mod agent_translate_codex;
#[path = "mcp_entries/agent_translate_cursor.rs"]
mod agent_translate_cursor;
#[path = "mcp_entries/agent_translate_gemini.rs"]
mod agent_translate_gemini;
#[path = "mcp_entries/agent_translate_opencode.rs"]
mod agent_translate_opencode;
#[path = "mcp_entries/entry_e2e.rs"]
mod entry_e2e;
#[path = "mcp_entries/entry_e2e_p6.rs"]
mod entry_e2e_p6;
#[path = "mcp_entries/entry_kind_agent_indexing.rs"]
mod entry_kind_agent_indexing;
#[path = "mcp_entries/entry_kind_indexing.rs"]
mod entry_kind_indexing;
#[path = "mcp_entries/exit_codes_e2e_mcp.rs"]
mod exit_codes_e2e_mcp;
#[path = "mcp_entries/live_sync.rs"]
mod live_sync;
#[path = "mcp_entries/mcp_config_clash.rs"]
mod mcp_config_clash;
#[path = "mcp_entries/mcp_config_create.rs"]
mod mcp_config_create;
#[path = "mcp_entries/mcp_config_preserve_order.rs"]
mod mcp_config_preserve_order;
#[path = "mcp_entries/mcp_config_remove.rs"]
mod mcp_config_remove;
#[path = "mcp_entries/mcp_config_update.rs"]
mod mcp_config_update;
#[path = "mcp_entries/mcp_get_skill_info.rs"]
mod mcp_get_skill_info;
#[path = "mcp_entries/mcp_get_skill_info_json_shape.rs"]
mod mcp_get_skill_info_json_shape;
#[path = "mcp_entries/mcp_input_length_caps.rs"]
mod mcp_input_length_caps;
#[path = "mcp_entries/mcp_lifecycle.rs"]
mod mcp_lifecycle;
#[path = "mcp_entries/mcp_log_format.rs"]
mod mcp_log_format;
#[path = "mcp_entries/mcp_prompts.rs"]
mod mcp_prompts;
#[path = "mcp_entries/mcp_prompts_get_error_json_shape.rs"]
mod mcp_prompts_get_error_json_shape;
#[path = "mcp_entries/mcp_prompts_get_json_shape.rs"]
mod mcp_prompts_get_json_shape;
#[path = "mcp_entries/mcp_prompts_list_json_shape.rs"]
mod mcp_prompts_list_json_shape;
#[path = "mcp_entries/mcp_search_skills_json_shape.rs"]
mod mcp_search_skills_json_shape;
#[path = "mcp_entries/mcp_search_skills_truncation.rs"]
mod mcp_search_skills_truncation;
#[path = "mcp_entries/mcp_server.rs"]
mod mcp_server;
#[path = "mcp_entries/mcp_tool_description.rs"]
mod mcp_tool_description;
#[path = "mcp_entries/personas.rs"]
mod personas;
#[path = "mcp_entries/personas_collision.rs"]
mod personas_collision;
#[path = "mcp_entries/personas_startup_scope.rs"]
mod personas_startup_scope;
#[path = "mcp_entries/prompt_collision.rs"]
mod prompt_collision;
#[path = "mcp_entries/prompt_collision_global.rs"]
mod prompt_collision_global;
#[path = "mcp_entries/prompt_naming.rs"]
mod prompt_naming;
#[path = "mcp_entries/substitution_arguments.rs"]
mod substitution_arguments;
#[path = "mcp_entries/substitution_builtins.rs"]
mod substitution_builtins;
#[path = "mcp_entries/substitution_data_dir.rs"]
mod substitution_data_dir;
#[path = "mcp_entries/substitution_env.rs"]
mod substitution_env;
#[path = "mcp_entries/substitution_pipeline.rs"]
mod substitution_pipeline;
#[path = "mcp_entries/substitution_skeleton.rs"]
mod substitution_skeleton;
