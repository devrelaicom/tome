//! Consolidated integration-test binary for the `index_query_misc` surface.
//!
//! The former top-level `tests/<name>.rs` files are submodules under
//! `tests/index_query_misc/`, sharing ONE compiled + linked binary instead of N. This
//! collapses the per-file static-link + process-spawn overhead to one per
//! group. Test names gain a `<name>::` module prefix, so `cargo test
//! <name>::` still filters by file and `cargo test --test index_query_misc` runs the group.

mod common;

#[path = "index_query_misc/embedding_stub.rs"]
mod embedding_stub;
#[path = "index_query_misc/error_messages.rs"]
mod error_messages;
#[path = "index_query_misc/exit_codes.rs"]
mod exit_codes;
#[path = "index_query_misc/exit_codes_e2e.rs"]
mod exit_codes_e2e;
#[path = "index_query_misc/frontmatter.rs"]
mod frontmatter;
#[path = "index_query_misc/frontmatter_p5_fields.rs"]
mod frontmatter_p5_fields;
#[path = "index_query_misc/index_lock.rs"]
mod index_lock;
#[path = "index_query_misc/index_schema_bootstrap.rs"]
mod index_schema_bootstrap;
#[path = "index_query_misc/manifest_strictness.rs"]
mod manifest_strictness;
#[path = "index_query_misc/no_directories_imports.rs"]
mod no_directories_imports;
#[path = "index_query_misc/no_phase3_paths.rs"]
mod no_phase3_paths;
#[path = "index_query_misc/path_validation.rs"]
mod path_validation;
#[path = "index_query_misc/query.rs"]
mod query;
#[path = "index_query_misc/readme_smoke.rs"]
mod readme_smoke;
#[path = "index_query_misc/reindex.rs"]
mod reindex;
#[path = "index_query_misc/schema_migration_e2e.rs"]
mod schema_migration_e2e;
#[path = "index_query_misc/schema_migration_p6.rs"]
mod schema_migration_p6;
#[path = "index_query_misc/schema_migration_v3.rs"]
mod schema_migration_v3;
#[path = "index_query_misc/schema_migrations.rs"]
mod schema_migrations;
#[path = "index_query_misc/scrubbing.rs"]
mod scrubbing;
#[path = "index_query_misc/search_knn_recall.rs"]
mod search_knn_recall;
#[path = "index_query_misc/search_knn_recall_realmodel.rs"]
mod search_knn_recall_realmodel;
#[path = "index_query_misc/security_hardening.rs"]
mod security_hardening;
#[path = "index_query_misc/status.rs"]
mod status;
#[cfg(unix)]
#[path = "index_query_misc/symlink_intermediate_guard.rs"]
mod symlink_intermediate_guard;
#[path = "index_query_misc/version_output.rs"]
mod version_output;
