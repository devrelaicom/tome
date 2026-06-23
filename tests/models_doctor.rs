//! Consolidated integration-test binary for the `models_doctor` surface.
//!
//! The former top-level `tests/<name>.rs` files are submodules under
//! `tests/models_doctor/`, sharing ONE compiled + linked binary instead of N. This
//! collapses the per-file static-link + process-spawn overhead to one per
//! group. Test names gain a `<name>::` module prefix, so `cargo test
//! <name>::` still filters by file and `cargo test --test models_doctor` runs the group.

mod common;

#[path = "models_doctor/doctor.rs"]
mod doctor;
#[path = "models_doctor/doctor_detected_uninstalled.rs"]
mod doctor_detected_uninstalled;
#[path = "models_doctor/doctor_fix_p4.rs"]
mod doctor_fix_p4;
#[path = "models_doctor/doctor_json.rs"]
mod doctor_json;
#[path = "models_doctor/doctor_mcp_states_p11.rs"]
mod doctor_mcp_states_p11;
#[path = "models_doctor/doctor_orphan_tmp_cleanup.rs"]
mod doctor_orphan_tmp_cleanup;
#[path = "models_doctor/doctor_outside_project.rs"]
mod doctor_outside_project;
#[path = "models_doctor/doctor_p4.rs"]
mod doctor_p4;
#[path = "models_doctor/doctor_p5.rs"]
mod doctor_p5;
#[path = "models_doctor/doctor_p6.rs"]
mod doctor_p6;
#[path = "models_doctor/doctor_p6_json_shape.rs"]
mod doctor_p6_json_shape;
#[path = "models_doctor/doctor_read_only_by_default.rs"]
mod doctor_read_only_by_default;
#[path = "models_doctor/doctor_readonly_schema.rs"]
mod doctor_readonly_schema;
#[path = "models_doctor/doctor_subsystem_serialize.rs"]
mod doctor_subsystem_serialize;
#[path = "models_doctor/doctor_verify_by_default.rs"]
mod doctor_verify_by_default;
#[path = "models_doctor/model_download.rs"]
mod model_download;
#[path = "models_doctor/model_download_complete.rs"]
mod model_download_complete;
#[path = "models_doctor/model_registry_invariant.rs"]
mod model_registry_invariant;
#[path = "models_doctor/models_download.rs"]
mod models_download;
#[path = "models_doctor/models_list.rs"]
mod models_list;
#[path = "models_doctor/models_remove.rs"]
mod models_remove;
#[path = "models_doctor/reranker_cpu_inference.rs"]
mod reranker_cpu_inference;
#[path = "models_doctor/summariser_cache.rs"]
mod summariser_cache;
#[path = "models_doctor/summariser_forward_progress.rs"]
mod summariser_forward_progress;
#[path = "models_doctor/summariser_real.rs"]
mod summariser_real;
#[path = "models_doctor/summariser_registry_no_placeholder.rs"]
mod summariser_registry_no_placeholder;
#[path = "models_doctor/summariser_stub.rs"]
mod summariser_stub;
#[path = "models_doctor/summariser_triggers.rs"]
mod summariser_triggers;
#[path = "models_doctor/summariser_triggers_end_to_end.rs"]
mod summariser_triggers_end_to_end;
