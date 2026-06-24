//! Phase 12 / US1 (T022) — provider-summariser trigger semantics (FR-027).
//!
//! Two posture assertions, both with a `RemoteSummariser` driven over the
//! transport seam (no network):
//!
//! - **Foreground** `regen-summary` fails LOUD: a provider failure surfaces as
//!   exit 94 (`ProviderRequestFailed`). Exercised via the DI seam
//!   `run_with_summariser` which propagates errors.
//! - **Auto-trigger** `regenerate_for_trigger` degrades a provider
//!   timeout/error to a non-fatal `warn!` and returns `Ok(())` — the
//!   post-commit summariser must never abort the triggering command (e.g.
//!   `tome plugin enable`). Exercised by installing the remote summariser via
//!   `SUMMARISER_OVERRIDE` and asserting the production trigger returns Ok.

use crate::common::{
    lifecycle_paths, stub_embedder_seed, stub_reranker_seed, stub_summariser_seed,
};
use std::sync::Arc;
use tempfile::TempDir;
use tome::config::{Config, ProviderEntry, ProviderKind, Secret};
use tome::index::{self, OpenOptions};
use tome::output::Mode;
use tome::paths::Paths;
use tome::provider::config::{Capability, resolve};
use tome::provider::http::{RawResponse, set_transport_override};
use tome::summarise::trigger::SummariserOverrideGuard;
use tome::summarise::{RemoteSummariser, Summariser, regenerate_for_trigger};
use tome::workspace::{self, WorkspaceName};

/// Build a `RemoteSummariser` pointed at an openai-kind provider through the
/// real `resolve` path.
fn remote_summariser() -> RemoteSummariser {
    let mut config = Config::default();
    config.providers.insert(
        "p".to_string(),
        ProviderEntry {
            kind: ProviderKind::Openai,
            base_url: None,
            api_key: Some(Secret::from("sk-key".to_string())),
        },
    );
    config.summariser.provider = Some("p".to_string());
    config.summariser.model = Some("gpt-4o-mini".to_string());
    let resolved = resolve(&config, Capability::Summariser)
        .expect("resolve ok")
        .expect("provider referenced");
    RemoteSummariser::new(resolved)
}

/// Seed a workspace with one enabled skill so the summariser input is non-empty.
fn seed_workspace(paths: &Paths, workspace_name: &str) {
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
            profile: None,
        },
    )
    .expect("open central DB");
    let workspace_id: i64 = conn
        .query_row(
            "SELECT id FROM workspaces WHERE name = ?1",
            rusqlite::params![workspace_name],
            |row| row.get(0),
        )
        .expect("lookup workspace_id");
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    conn.execute(
        "INSERT INTO skills
           (catalog, plugin, name, description, plugin_version, path, content_hash, indexed_at)
         VALUES ('cat', 'plug', 's1', 'd', '0.0.0', '/dev/null', 'h', ?1)",
        rusqlite::params![now],
    )
    .unwrap();
    let skill_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![workspace_id, skill_id, now],
    )
    .unwrap();
}

#[test]
fn foreground_regen_summary_fails_loud_on_provider_failure() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    let ws = WorkspaceName::parse("mine").unwrap();
    workspace::init::init(ws.clone(), false, &paths).unwrap();
    seed_workspace(&paths, "mine");

    // A 503 that exhausts retries → ProviderError{Unreachable} → 94.
    let _guard = set_transport_override(|_spec| {
        Ok(RawResponse {
            status: 503,
            retry_after: Some(std::time::Duration::from_secs(0)),
            body: Vec::new(),
        })
    });
    let summariser = remote_summariser();
    let err = tome::commands::workspace::regen_summary::run_with_summariser(
        &ws,
        &summariser,
        &paths,
        Mode::Human,
    )
    .expect_err("foreground regen-summary must fail loud on provider failure");
    assert_eq!(
        err.exit_code(),
        94,
        "foreground provider failure must surface exit 94: {err:?}"
    );
}

#[test]
fn auto_trigger_degrades_provider_timeout_to_ok() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    let ws = WorkspaceName::parse("mine").unwrap();
    workspace::init::init(ws.clone(), false, &paths).unwrap();
    seed_workspace(&paths, "mine");

    // The transport seam injects a persistent timeout; the remote summariser
    // surfaces ProviderError{Timeout} → 94; the production trigger DEGRADES it.
    let _transport =
        set_transport_override(|_spec| Err(tome::provider::http::TransportFailure::Timeout));

    // Install the remote summariser via the trigger's DI slot so the production
    // `regenerate_for_trigger` uses it instead of the bundled LlamaSummariser.
    let summariser: Arc<dyn Summariser> = Arc::new(remote_summariser());
    let _override = SummariserOverrideGuard::install(summariser);

    // FR-027: the post-commit trigger must NOT abort — it degrades to Ok(()).
    let result = regenerate_for_trigger(&ws, &paths);
    assert!(
        result.is_ok(),
        "auto-trigger must degrade a provider timeout to Ok(()), got {result:?}"
    );
}
