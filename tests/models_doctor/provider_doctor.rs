//! Phase 12 / US4 (T068) — `tome doctor` provider report (FR-018) +
//! corrupt-remote-index check (FR-017), end-to-end through `assemble_report`.
//!
//! The provider report + reachability + corrupt-index checks are exercised
//! against a real seeded index DB and an on-disk `config.toml`. `--verify`
//! reachability runs through the transport seam (no network). The `--fix`
//! cost-aware behaviour (bundled auto-reindex vs remote print-only) is asserted
//! on the produced `SuggestedFix` records.

use crate::common::{
    ToolEnv, fabricate_all_registry_models, fabricate_installed_models, paths_for,
};
use tempfile::TempDir;
use tome::doctor;
use tome::doctor::DoctorClassification;
use tome::embedding::MODEL_REGISTRY;
use tome::index::{self, OpenOptions};
use tome::provider::http::{RawResponse, set_transport_override};
use tome::workspace::{ResolvedScope, ScopeSource};

/// Fabricate every registry model on disk EXCEPT the ones whose name matches
/// `skip` — so the skipped capability's bundled model is genuinely absent and
/// `check_model` would (without the issue-#499 provider-aware skip) report it
/// `missing`. Used to prove that a provider-configured capability reads
/// `not_applicable` even with no bundled model present, while the other two
/// capabilities stay `ok`.
fn fabricate_all_except(paths: &tome::paths::Paths, skip: &[&str]) {
    let entries: Vec<&tome::embedding::ModelEntry> = MODEL_REGISTRY
        .iter()
        .filter(|e| !skip.contains(&e.name))
        .collect();
    fabricate_installed_models(paths, &entries);
}

/// The registry name of the active-default embedder / reranker / summariser
/// (the entries `assemble_report` checks on a fresh install with no index DB).
fn active_default_names() -> (&'static str, &'static str, &'static str) {
    use tome::embedding::Profile;
    (
        tome::embedding::profile::embedder_for(Profile::DEFAULT).name,
        tome::embedding::profile::reranker_for(Profile::DEFAULT).name,
        tome::summarise::registry::summariser_entry().name,
    )
}

fn global_scope() -> ResolvedScope {
    ResolvedScope {
        scope: tome::workspace::Scope(tome::workspace::WorkspaceName::global()),
        source: ScopeSource::GlobalFallback,
        project_root: None,
        overridden_project_marker: None,
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

    // Read-only discipline (FR-124, mirroring `models_test_writes_no_stored_state`):
    // `assemble_report` must NOT mutate the index DB. Snapshot the DB file's exact
    // bytes (and size) BEFORE the report; assert they are byte-identical AFTER.
    // A full-content comparison is strictly stronger than a hash and needs no
    // extra dev-dep.
    let db_before = std::fs::read(&paths.index_db).expect("read index DB before report");
    let len_before = db_before.len();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();

    let db_after = std::fs::read(&paths.index_db).expect("read index DB after report");
    assert_eq!(
        db_after.len(),
        len_before,
        "assemble_report must not change the index DB size (read-only)"
    );
    assert!(
        db_after == db_before,
        "assemble_report must leave the index DB byte-identical (read-only, FR-124)"
    );

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

// ---- Issue #499: provider-served capability → bundled model not_applicable --

#[test]
fn embedding_provider_makes_bundled_embedder_not_applicable() {
    // A configured embedding provider means the bundled embedder is genuinely
    // unnecessary. Leave the embedder absent (fabricate the other two) so the
    // pre-fix behaviour would be `missing` → Unhealthy. After the fix the row
    // must read `not_applicable`, `overall` must NOT be Unhealthy, and there
    // must be no "download model" fix for the embedder.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    let (embedder_name, _rr, _s) = active_default_names();
    fabricate_all_except(&paths, &[embedder_name]);
    write_config(
        &paths,
        r#"
[providers.openai]
kind = "openai"
api_key = "sk-inline-key"

[embedding]
provider = "openai"
model = "text-embedding-3-small"
"#,
    );
    let home = empty_home();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();

    assert_eq!(
        report.embedder.state, "not_applicable",
        "embedding provider configured → bundled embedder is not_applicable, not missing"
    );
    assert_ne!(
        report.embedder.state, "missing",
        "the unused bundled embedder must not be reported missing"
    );
    assert_ne!(
        report.overall,
        DoctorClassification::Unhealthy,
        "a not_applicable embedder must not flip overall to unhealthy: {:?}",
        report.overall,
    );
    assert!(
        report
            .suggested_fixes
            .iter()
            .all(|f| f.subsystem != "embedder"),
        "no embedder download fix when a provider serves embedding",
    );
}

#[test]
fn reranker_provider_makes_bundled_reranker_not_applicable() {
    // A configured reranker provider → the bundled reranker is unnecessary.
    // Leave the reranker absent (fabricate the other two). Pre-fix this row
    // would be `missing` → Degraded; post-fix it is `not_applicable` and does
    // not degrade the verdict, with no download fix.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    let (_emb, reranker_name, _s) = active_default_names();
    fabricate_all_except(&paths, &[reranker_name]);
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

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();

    assert_eq!(
        report.reranker.state, "not_applicable",
        "reranker provider configured → bundled reranker is not_applicable, not missing"
    );
    // Embedder + summariser are on disk → the only capability at risk is the
    // reranker, which is now not_applicable → overall stays ok.
    assert_eq!(
        report.overall,
        DoctorClassification::Ok,
        "a not_applicable reranker (others healthy) leaves overall ok: {:?}",
        report.overall,
    );
    assert!(
        report
            .suggested_fixes
            .iter()
            .all(|f| f.subsystem != "reranker"),
        "no reranker download fix when a provider serves reranking",
    );
}

#[test]
fn summariser_provider_makes_bundled_summariser_not_applicable() {
    // A configured summariser provider → the bundled summariser is
    // unnecessary. Leave the summariser absent (fabricate the other two).
    // Pre-fix this row would be `missing` → Degraded; post-fix it is
    // `not_applicable` with no download fix and overall stays ok.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    let (_emb, _rr, summariser_name) = active_default_names();
    fabricate_all_except(&paths, &[summariser_name]);
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

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();

    assert_eq!(
        report.summariser.state, "not_applicable",
        "summariser provider configured → bundled summariser is not_applicable, not missing"
    );
    assert_eq!(
        report.overall,
        DoctorClassification::Ok,
        "a not_applicable summariser (others healthy) leaves overall ok: {:?}",
        report.overall,
    );
    assert!(
        report
            .suggested_fixes
            .iter()
            .all(|f| f.subsystem != "summariser"),
        "no summariser download fix when a provider serves summarisation",
    );
}

#[test]
fn all_capabilities_provider_served_with_no_bundled_models_is_ok() {
    // Every capability points at a remote provider and NO bundled model is on
    // disk. All three rows read `not_applicable`, no model download fixes are
    // emitted, and overall is ok — the pure "BYOM everything" case.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    // Deliberately fabricate nothing.
    write_config(
        &paths,
        r#"
[providers.openai]
kind = "openai"
api_key = "sk-inline-key"

[providers.voyage]
kind = "voyage"
api_key = "vk"

[embedding]
provider = "openai"
model = "text-embedding-3-small"

[reranker]
provider = "voyage"
model = "rerank-2"

[summariser]
provider = "openai"
model = "gpt-4o-mini"
"#,
    );
    let home = empty_home();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();

    assert_eq!(report.embedder.state, "not_applicable");
    assert_eq!(report.reranker.state, "not_applicable");
    assert_eq!(report.summariser.state, "not_applicable");
    assert_eq!(
        report.overall,
        DoctorClassification::Ok,
        "all capabilities provider-served, no bundled models → overall ok: {:?}",
        report.overall,
    );
    for cap in ["embedder", "reranker", "summariser"] {
        assert!(
            report.suggested_fixes.iter().all(|f| f.subsystem != cap),
            "no `{cap}` download fix when a provider serves it",
        );
    }
}

#[test]
fn no_provider_still_reports_missing_bundled_model() {
    // Regression guard: without any provider, the bundled-model check is
    // unchanged — an absent embedder is still `missing` (→ Unhealthy) with a
    // download fix. This pins that the #499 skip is provider-GATED, not a
    // blanket suppression.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    let (embedder_name, _rr, _s) = active_default_names();
    fabricate_all_except(&paths, &[embedder_name]);
    // No config.toml → bundled everything.
    let home = empty_home();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();

    assert_eq!(
        report.embedder.state, "missing",
        "no provider → absent bundled embedder is still missing"
    );
    assert_eq!(
        report.overall,
        DoctorClassification::Unhealthy,
        "a missing embedder (no provider) is still unhealthy"
    );
    assert!(
        report
            .suggested_fixes
            .iter()
            .any(|f| f.subsystem == "embedder"),
        "a missing bundled embedder still gets a download fix",
    );
}
