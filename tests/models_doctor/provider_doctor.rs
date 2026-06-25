//! Phase 12 / US4 (T068) — `tome doctor` provider report (FR-018) +
//! corrupt-remote-index check (FR-017), end-to-end through `assemble_report`.
//!
//! The provider report + reachability + corrupt-index checks are exercised
//! against a real seeded index DB and an on-disk `config.toml`. `--verify`
//! reachability runs through the transport seam (no network). The `--fix`
//! cost-aware behaviour (bundled auto-reindex vs remote print-only) is asserted
//! on the produced `SuggestedFix` records.

use crate::common::{ToolEnv, fabricate_all_registry_models, paths_for};
use tempfile::TempDir;
use tome::doctor;
use tome::index::{self, OpenOptions};
use tome::provider::http::{RawResponse, set_transport_override};
use tome::workspace::{ResolvedScope, ScopeSource};

fn global_scope() -> ResolvedScope {
    ResolvedScope {
        scope: tome::workspace::Scope(tome::workspace::WorkspaceName::global()),
        source: ScopeSource::GlobalFallback,
        project_root: None,
    }
}

fn empty_home() -> TempDir {
    TempDir::new().unwrap()
}

/// Write a `config.toml` to the resolved global-config path.
fn write_config(paths: &tome::paths::Paths, body: &str) {
    std::fs::create_dir_all(&paths.root).unwrap();
    std::fs::write(&paths.global_config_file, body).unwrap();
}

/// Bootstrap an index DB with `meta.embedder_dimension = meta_dim` and one
/// `skill_embeddings` BLOB of `blob_dim` f32s.
fn seed_index_with_dim(
    paths: &tome::paths::Paths,
    meta_dim: Option<usize>,
    blob_dim: Option<usize>,
) {
    let (e, r, s) = tome::commands::plugin::registry_seeds();
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: e,
            reranker: r,
            summariser: s,
            profile: None,
        },
    )
    .expect("bootstrap index");
    if let Some(d) = meta_dim {
        tome::index::meta::write_embedder_dimension(&conn, d).unwrap();
    }
    if let Some(d) = blob_dim {
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        conn.execute(
            "INSERT INTO skills
               (catalog, plugin, name, description, plugin_version, path, content_hash, indexed_at)
             VALUES ('c','p','s','d','0.0.0','/dev/null','h', ?1)",
            rusqlite::params![now],
        )
        .unwrap();
        let skill_id = conn.last_insert_rowid();
        let bytes: Vec<u8> = vec![0u8; d * 4];
        conn.execute(
            "INSERT INTO skill_embeddings (skill_id, embedding) VALUES (?1, ?2)",
            rusqlite::params![skill_id, bytes],
        )
        .unwrap();
    }
}

// ---- Provider report (FR-018) --------------------------------------------

#[test]
fn provider_report_surfaces_configured_remote_provider() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fabricate_all_registry_models(&paths);
    write_config(
        &paths,
        r#"
[providers.openai]
kind = "openai"
api_key = "sk-inline-key"

[summariser]
provider = "openai"
model = "gpt-4o-mini"
"#,
    );
    let home = empty_home();

    // No --verify → reachable is None.
    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert_eq!(report.providers.len(), 1, "one referenced provider");
    let p = &report.providers[0];
    assert_eq!(p.name, "openai");
    assert_eq!(p.kind, "openai");
    assert_eq!(p.capabilities, vec!["summariser"]);
    assert!(p.credential_resolvable, "inline key resolves");
    assert_eq!(p.reachable, None, "reachable absent without --verify");
}

#[test]
fn provider_report_absent_when_no_providers_configured() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fabricate_all_registry_models(&paths);
    let home = empty_home();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert!(
        report.providers.is_empty(),
        "no providers configured → empty provider report (byte-stable wire shape)"
    );
}

#[test]
fn provider_verify_reports_reachable_through_transport_seam() {
    // `--verify` runs one lightweight round-trip per provider. With a reranker
    // (Voyage) provider, the seam returns a valid rerank body → reachable=true.
    // No bundled models are fabricated: the remote round-trip needs none, and
    // skipping them avoids the multi-hundred-MB `--verify` model rehash.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    write_config(
        &paths,
        r#"
[providers.voyage]
kind = "voyage"
api_key = "vk"

[reranker]
provider = "voyage"
model = "rerank-2"
"#,
    );
    let home = empty_home();

    let _g = set_transport_override(|_spec| {
        Ok(RawResponse {
            status: 200,
            retry_after: None,
            body: serde_json::to_vec(&serde_json::json!({
                "results": [{ "index": 0, "relevance_score": 0.9 }]
            }))
            .unwrap(),
        })
    });

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), true).unwrap();
    assert_eq!(report.providers.len(), 1);
    assert_eq!(
        report.providers[0].reachable,
        Some(true),
        "a reachable provider verifies true via the seam"
    );
}

#[test]
fn provider_verify_reports_unreachable_on_transport_failure() {
    // No bundled models fabricated (see the reachable test) — keeps the
    // `--verify` pass off the model-rehash path.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    write_config(
        &paths,
        r#"
[providers.voyage]
kind = "voyage"
api_key = "vk"

[reranker]
provider = "voyage"
model = "rerank-2"
"#,
    );
    let home = empty_home();

    // A persistent 503 exhausts retries → ProviderError → round-trip fails.
    let _g = set_transport_override(|_spec| {
        Ok(RawResponse {
            status: 503,
            retry_after: Some(std::time::Duration::from_secs(0)),
            body: Vec::new(),
        })
    });

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), true).unwrap();
    assert_eq!(
        report.providers[0].reachable,
        Some(false),
        "an unreachable provider verifies false (doctor never crashes)"
    );
}

// ---- Corrupt-remote-index check (FR-017) ---------------------------------

#[test]
fn corrupt_index_finding_for_remote_embedder_is_print_only() {
    // A remote embedder + a dimension mismatch → corrupt-remote-index fix that
    // is NOT auto-fixable (no surprise paid API cost): the command is printed.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fabricate_all_registry_models(&paths);
    seed_index_with_dim(&paths, Some(1024), Some(768));
    write_config(
        &paths,
        r#"
[providers.openai]
kind = "openai"
api_key = "sk-key"

[embedding]
provider = "openai"
model = "text-embedding-3-small"
"#,
    );
    let home = empty_home();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    let fix = report
        .suggested_fixes
        .iter()
        .find(|f| f.diagnosis.contains("corrupt-remote-index"))
        .expect("corrupt-remote-index fix present");
    assert_eq!(fix.subsystem, "index");
    assert!(
        !fix.auto_fixable,
        "a REMOTE embedder corrupt-index fix must be print-only (no paid re-embed)"
    );
    assert_eq!(fix.command, "tome reindex --force");
    assert!(
        fix.diagnosis.contains("768") && fix.diagnosis.contains("1024"),
        "diagnosis names stored + expected dims: {}",
        fix.diagnosis
    );
}

#[test]
fn corrupt_index_finding_for_bundled_embedder_is_auto_fixable() {
    // A bundled embedder (no [embedding] provider) + a dimension mismatch →
    // corrupt-remote-index fix that IS auto-fixable (re-runs reindex --force
    // locally; no API cost). We assert the classification only — running the
    // real reindex needs real ONNX models, out of scope for a fast test.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fabricate_all_registry_models(&paths);
    seed_index_with_dim(&paths, Some(1024), Some(768));
    let home = empty_home();
    // No config.toml → bundled embedder.

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    let fix = report
        .suggested_fixes
        .iter()
        .find(|f| f.diagnosis.contains("corrupt-remote-index"))
        .expect("corrupt-remote-index fix present");
    assert_eq!(fix.subsystem, "index");
    assert!(
        fix.auto_fixable,
        "a BUNDLED embedder corrupt-index fix is auto-fixable (local re-embed)"
    );
}

#[test]
fn corrupt_index_no_finding_when_dims_match() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fabricate_all_registry_models(&paths);
    seed_index_with_dim(&paths, Some(384), Some(384));
    let home = empty_home();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert!(
        report
            .suggested_fixes
            .iter()
            .all(|f| !f.diagnosis.contains("corrupt-remote-index")),
        "matching dims → no corrupt-index finding"
    );
}

#[test]
fn corrupt_index_no_finding_when_no_meta_dim() {
    // Bundled / never-remote-reindexed: no meta.embedder_dimension → the
    // dimension-free bundled storage is fine, no finding.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fabricate_all_registry_models(&paths);
    seed_index_with_dim(&paths, None, Some(384));
    let home = empty_home();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert!(
        report
            .suggested_fixes
            .iter()
            .all(|f| !f.diagnosis.contains("corrupt-remote-index")),
        "absent meta dim → not applicable, no finding"
    );
}
