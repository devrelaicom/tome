//! `tome models test <summariser|embedding|reranker>` — a single, real
//! round-trip against the ACTIVE model for a capability (the configured
//! remote provider, else the bundled local model) plus a success assertion.
//!
//! ## Read-only (index state)
//!
//! `models test` writes NO INDEX state (FR-019): it never reindexes, never
//! writes `meta`, never touches the index DB. A remote embedder establishes its
//! run dimension purely IN MEMORY; the bundled path loads the on-disk model and
//! does one inference. The index reads are `meta.embedder_dimension` (the
//! expected dimension a remote embedding is validated against) plus the active
//! profile — both via `open_read_only`. The one non-index write the remote path
//! may perform is the first-run remote-provider notice sidecar under `~/.tome/`
//! (FR-023, `provider::notice`), printed once before bytes leave the box.
//!
//! ## Actionable failure, never a crash
//!
//! A bundled-local model that the active profile selects but that is NOT on
//! disk surfaces the `build_*` constructor's clean error
//! (`ModelMissing` / `SummariserFailure`), not a panic. A remote failure maps
//! once onto the closed `TomeError` set (`ProviderRequestFailed`/94,
//! `RemoteEmbeddingInvalid`/95, `ProviderConfigInvalid`/93). The command
//! propagates whatever `build_*`/round-trip returns, so the CLI's standard
//! exit-code mapping applies.
//!
//! Spec: `contracts/cli-and-doctor.md` §"`tome models test`", FR-017/019.

use std::time::Instant;

use serde::Serialize;

use crate::cli::{ModelsTestArgs, TestCapability};
use crate::error::TomeError;
use crate::index::query::Candidate;
use crate::output::{self, Mode};
use crate::paths::Paths;
use crate::plugin::identity::EntryKind;
use crate::workspace::ResolvedScope;

/// The fixed probe string the embedding + summariser round-trips use. Short,
/// content-free, and stable so a test is deterministic regardless of provider.
const PROBE_TEXT: &str = "connectivity check";

/// The fixed reranking query the reranker round-trip uses.
const PROBE_QUERY: &str = "test query";

pub fn run(args: ModelsTestArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    // Strict config load: a typo in `[providers]` / `[embedding]` /
    // `[summariser]` / `[reranker]` must fail loudly (exit 5), the same as
    // every other foreground command.
    let cfg = crate::config::load(&paths)?;

    // Static credential pre-flight (issue #291): when the capability is
    // configured to use an EXTERNAL provider whose credential does NOT resolve,
    // fail fast naming the exact `TOME_<NAME>_API_KEY` env var — WITHOUT making
    // a doomed network request that 401s into a deep `ProviderRequestFailed`/94.
    // A missing credential is a config problem → `ProviderConfigInvalid`/93. The
    // bundled-local path resolves to `Ok(None)` and the pre-flight is a no-op.
    crate::provider::credential_preflight(&cfg, capability_of(args.capability))?;

    let outcome = match args.capability {
        TestCapability::Embedding => test_embedding(&cfg, &paths, scope)?,
        TestCapability::Summariser => test_summariser(&cfg, &paths)?,
        TestCapability::Reranker => test_reranker(&cfg, &paths, scope)?,
    };

    match mode {
        Mode::Human => emit_human(&outcome),
        Mode::Json => output::write_json(&outcome),
    }
}

/// Map the CLI `TestCapability` onto the provider-resolution [`Capability`] used
/// by the credential pre-flight. `models test` never exercises the runtime hook
/// chat capability, so there is no `HookPrompt` arm.
fn capability_of(cap: TestCapability) -> crate::provider::Capability {
    match cap {
        TestCapability::Embedding => crate::provider::Capability::Embedding,
        TestCapability::Summariser => crate::provider::Capability::Summariser,
        TestCapability::Reranker => crate::provider::Capability::Reranker,
    }
}

// ---------------------------------------------------------------------------
// Per-capability round-trips.
// ---------------------------------------------------------------------------

/// Embed a fixed string and validate the vector. The `RemoteEmbedder` runs
/// `validate_embedding` inside `embed`, so an empty / non-finite / wrong-dim
/// vector surfaces as `RemoteEmbeddingInvalid`/95 here. The bundled embedder
/// validates structurally by producing the model's fixed-dimension vector.
fn test_embedding(
    cfg: &crate::config::Config,
    paths: &Paths,
    scope: &ResolvedScope,
) -> Result<TestOutcome, TomeError> {
    // The active embedder registry entry the profile selects (bundled path),
    // and the persisted `meta.embedder_dimension` (remote path) — both read
    // from the resolved workspace's index when one exists, read-only.
    let (active_embedder, persisted_dim) = embedding_seed(paths, scope)?;

    let embedder = crate::embedding::build_embedder(cfg, paths, active_embedder, persisted_dim)?;
    let model_kind = ModelKindLabel::for_embedding(cfg);

    let start = Instant::now();
    // `embed` runs the shared `validate_embedding` for the remote path; the
    // bundled path produces the model's vector. Either way a non-empty, finite
    // vector of the model's dimension is the success criterion.
    let vector = embedder.embed(PROBE_TEXT)?;
    let latency_ms = start.elapsed().as_millis() as u64;

    // Defensive: `embed` already guarantees non-empty for the remote path;
    // assert it here too so the bundled path can never silently report a
    // success on a degenerate empty vector.
    if vector.is_empty() {
        return Err(TomeError::RemoteEmbeddingInvalid {
            detail: "embedding round-trip produced an empty vector".to_string(),
        });
    }

    Ok(TestOutcome {
        capability: "embedding",
        model_kind: model_kind.as_str(),
        model: embedder.model_name().to_owned(),
        success: true,
        latency_ms,
        detail: TestDetail::Embedding {
            dimension: vector.len(),
        },
    })
}

/// Summarise a tiny fixed input. The summariser errors on an empty short/long
/// (`SummariserFailureKind::OutputEmpty`, exit 24), so a successful return is
/// the success assertion; we additionally assert non-empty defensively.
fn test_summariser(cfg: &crate::config::Config, paths: &Paths) -> Result<TestOutcome, TomeError> {
    // `tighter_timeout = false`: this is a foreground diagnostic, use the full
    // provider timeout (not the post-commit trigger's tighter bound).
    let summariser = crate::summarise::build_summariser(cfg, paths, false)?;
    let model_kind = ModelKindLabel::for_summariser(cfg);

    let input = probe_summary_input();
    // Use the effective long cap so the round-trip matches production framing.
    let long_max = cfg
        .summariser
        .long_max_chars
        .unwrap_or(crate::summarise::LONG_MAX_CHARS);

    let start = Instant::now();
    let out = summariser.summarise(&input, long_max)?;
    let latency_ms = start.elapsed().as_millis() as u64;

    // The trait impl already rejects an empty short/long; assert again so a
    // future impl that relaxes that can't slip a degenerate success through.
    if out.short.trim().is_empty() {
        return Err(TomeError::SummariserFailure {
            kind: crate::error::SummariserFailureKind::OutputEmpty {
                which: crate::error::ShortOrLong::Short,
            },
        });
    }
    if out.long.trim().is_empty() {
        return Err(TomeError::SummariserFailure {
            kind: crate::error::SummariserFailureKind::OutputEmpty {
                which: crate::error::ShortOrLong::Long,
            },
        });
    }

    Ok(TestOutcome {
        capability: "summariser",
        model_kind: model_kind.as_str(),
        model: summary_model_label(cfg),
        success: true,
        latency_ms,
        detail: TestDetail::Summariser {
            short_chars: out.short.chars().count(),
            long_chars: out.long.chars().count(),
        },
    })
}

/// Rerank a small fixed candidate set. A non-empty scored ordering over the
/// set is the success assertion. The bundled reranker scores the documents;
/// the remote reranker maps each result index back to the input candidate.
fn test_reranker(
    cfg: &crate::config::Config,
    paths: &Paths,
    scope: &ResolvedScope,
) -> Result<TestOutcome, TomeError> {
    // The active reranker registry entry the profile selects (bundled path).
    let active_reranker = active_reranker_entry(paths, scope)?;
    let reranker = crate::embedding::build_reranker(cfg, paths, active_reranker)?;
    let model_kind = ModelKindLabel::for_reranker(cfg);

    let candidates = probe_candidates();
    let n = candidates.len();

    let start = Instant::now();
    let scored = reranker.rerank(PROBE_QUERY, candidates)?;
    let latency_ms = start.elapsed().as_millis() as u64;

    if scored.is_empty() {
        return Err(TomeError::RerankingFailure(
            "reranker round-trip returned no scored candidates".to_string(),
        ));
    }

    Ok(TestOutcome {
        capability: "reranker",
        model_kind: model_kind.as_str(),
        model: reranker.model_name().to_owned(),
        success: true,
        latency_ms,
        detail: TestDetail::Reranker {
            candidates: n,
            scored: scored.len(),
            top_name: scored.first().map(|s| s.candidate.name.clone()),
        },
    })
}

// ---------------------------------------------------------------------------
// Read-only meta resolution (the only reads `models test` performs).
// ---------------------------------------------------------------------------

/// Resolve `(active embedder registry entry, persisted meta.embedder_dimension)`
/// for the embedding round-trip. Both come from the resolved workspace's index
/// when it exists (read-only); on a fresh install (no DB) the default profile's
/// embedder is used and the persisted dim is `None`.
fn embedding_seed(
    paths: &Paths,
    scope: &ResolvedScope,
) -> Result<
    (
        &'static crate::embedding::registry::ModelEntry,
        Option<usize>,
    ),
    TomeError,
> {
    let _ = scope; // dimension is a global index property; the scope picks the DB path
    if paths.index_db.is_file() {
        let conn = crate::index::open_read_only(&paths.index_db)?;
        let entry = crate::index::meta::active_embedder(&conn)?;
        let persisted = crate::index::meta::read_embedder_dimension(&conn)?;
        Ok((entry, persisted))
    } else {
        Ok((
            crate::embedding::profile::embedder_for(crate::embedding::Profile::DEFAULT),
            None,
        ))
    }
}

/// The active reranker registry entry the resolved scope's profile selects.
/// Read-only meta resolution; default profile on a fresh install.
fn active_reranker_entry(
    paths: &Paths,
    scope: &ResolvedScope,
) -> Result<&'static crate::embedding::registry::ModelEntry, TomeError> {
    let _ = scope;
    if paths.index_db.is_file() {
        let conn = crate::index::open_read_only(&paths.index_db)?;
        crate::index::meta::active_reranker(&conn)
    } else {
        Ok(crate::embedding::profile::reranker_for(
            crate::embedding::Profile::DEFAULT,
        ))
    }
}

// ---------------------------------------------------------------------------
// Probe inputs (fixed, content-free, deterministic).
// ---------------------------------------------------------------------------

/// A tiny fixed summariser input — one plugin with one skill — so the round-trip
/// has real content to compress without depending on any workspace state.
fn probe_summary_input() -> crate::summarise::PluginSummariesInput {
    crate::summarise::PluginSummariesInput {
        plugins: vec![crate::summarise::PluginSummaryItem {
            catalog: "test".to_string(),
            plugin: "connectivity".to_string(),
            description: "A connectivity-check plugin used by `tome models test`.".to_string(),
            skills: vec![crate::summarise::SkillSummaryItem {
                name: "ping".to_string(),
                description: "Verify the summariser is reachable and returns text.".to_string(),
            }],
        }],
    }
}

/// A small fixed candidate set for the reranker round-trip. Three identifiable
/// candidates is enough to prove a scored ordering is returned.
fn probe_candidates() -> Vec<Candidate> {
    ["alpha", "bravo", "charlie"]
        .iter()
        .enumerate()
        .map(|(i, name)| Candidate {
            skill_id: i as i64,
            catalog: "test".to_string(),
            plugin: "connectivity".to_string(),
            name: (*name).to_string(),
            kind: EntryKind::Skill,
            description: format!("connectivity-check candidate {name}"),
            plugin_version: "0.0.0".to_string(),
            path: format!("/dev/null/{name}"),
            distance: 0.0,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Output records.
// ---------------------------------------------------------------------------

/// Whether the active model is a remote provider (with its kind) or the bundled
/// local model.
enum ModelKindLabel {
    Bundled,
    Remote(crate::config::ProviderKind),
}

impl ModelKindLabel {
    fn for_embedding(cfg: &crate::config::Config) -> Self {
        Self::from_provider_kind(
            cfg.embedding
                .provider
                .as_deref()
                .and_then(|name| cfg.providers.get(name).map(|e| e.kind)),
        )
    }

    fn for_summariser(cfg: &crate::config::Config) -> Self {
        Self::from_provider_kind(
            cfg.summariser
                .provider
                .as_deref()
                .and_then(|name| cfg.providers.get(name).map(|e| e.kind)),
        )
    }

    fn for_reranker(cfg: &crate::config::Config) -> Self {
        Self::from_provider_kind(
            cfg.reranker
                .provider
                .as_deref()
                .and_then(|name| cfg.providers.get(name).map(|e| e.kind)),
        )
    }

    fn from_provider_kind(kind: Option<crate::config::ProviderKind>) -> Self {
        match kind {
            Some(k) => Self::Remote(k),
            None => Self::Bundled,
        }
    }

    /// The wire/human label: `"bundled"` or `"remote:<kind>"`.
    fn as_str(&self) -> String {
        match self {
            ModelKindLabel::Bundled => "bundled".to_string(),
            ModelKindLabel::Remote(kind) => format!("remote:{}", kind.as_str()),
        }
    }
}

/// The summariser's model identity for the report. The bundled `Summariser`
/// trait deliberately doesn't surface model identity, so derive it from config:
/// `"<provider>/<model>"` on the remote path, else the bundled registry name.
fn summary_model_label(cfg: &crate::config::Config) -> String {
    match (
        cfg.summariser.provider.as_deref(),
        cfg.summariser.model.as_deref(),
    ) {
        (Some(provider), Some(model)) => format!("{provider}/{model}"),
        _ => crate::summarise::registry::summariser_entry()
            .name
            .to_string(),
    }
}

/// The `--json` / human outcome record for one `models test` invocation.
#[derive(Debug, Serialize)]
struct TestOutcome {
    /// `"summariser"` | `"embedding"` | `"reranker"`.
    capability: &'static str,
    /// `"bundled"` | `"remote:<kind>"`.
    model_kind: String,
    /// The model identity (`<provider>/<model>` for remote, registry name for
    /// bundled).
    model: String,
    /// Always `true` when this record is produced — a failed round-trip
    /// propagates an `Err` and never reaches here.
    success: bool,
    /// Round-trip wall-clock latency in whole milliseconds.
    latency_ms: u64,
    /// The per-capability detail.
    #[serde(flatten)]
    detail: TestDetail,
}

/// Per-capability detail fields, flattened into the outcome record.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
enum TestDetail {
    Embedding {
        dimension: usize,
    },
    Summariser {
        short_chars: usize,
        long_chars: usize,
    },
    Reranker {
        candidates: usize,
        scored: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        top_name: Option<String>,
    },
}

fn emit_human(outcome: &TestOutcome) -> Result<(), TomeError> {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let tick = crate::presentation::colour::success("✓");
    writeln!(
        out,
        "{tick} {} ok — {} ({}), {} ms",
        outcome.capability, outcome.model, outcome.model_kind, outcome.latency_ms,
    )?;
    match &outcome.detail {
        TestDetail::Embedding { dimension } => {
            writeln!(out, "  dimension: {dimension} (non-empty, finite)")?;
        }
        TestDetail::Summariser {
            short_chars,
            long_chars,
        } => {
            writeln!(
                out,
                "  short: {short_chars} chars, long: {long_chars} chars"
            )?;
        }
        TestDetail::Reranker {
            candidates,
            scored,
            top_name,
        } => {
            writeln!(out, "  reranked {scored}/{candidates} candidates")?;
            if let Some(name) = top_name {
                writeln!(out, "  top: {name}")?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, ProviderEntry, ProviderKind, Secret};
    use crate::provider::http::{RawResponse, set_transport_override};

    // --- the round-trip helpers operate over the transport seam (remote path) ---

    fn config_remote_embedding(kind: ProviderKind) -> Config {
        let mut config = Config::default();
        config.providers.insert(
            "p".to_string(),
            ProviderEntry {
                kind,
                base_url: None,
                api_key: Some(Secret::from("sk-key".to_string())),
            },
        );
        config.embedding.provider = Some("p".to_string());
        config.embedding.model = Some("embed-model".to_string());
        config
    }

    fn ok_embedding(values: &[f32]) -> RawResponse {
        RawResponse {
            status: 200,
            retry_after: None,
            body: serde_json::to_vec(&serde_json::json!({
                "data": [{ "index": 0, "embedding": values }]
            }))
            .unwrap(),
        }
    }

    #[test]
    fn embedding_round_trip_reports_dimension_and_finite() {
        let _g = set_transport_override(|_spec| Ok(ok_embedding(&[0.1, 0.2, 0.3, 0.4])));
        let dir = tempfile::TempDir::new().unwrap();
        let paths = Paths::from_root(dir.path().to_path_buf());
        let cfg = config_remote_embedding(ProviderKind::Openai);
        let scope = crate::workspace::ResolvedScope::global_fallback();

        let outcome = test_embedding(&cfg, &paths, &scope).expect("embedding round-trip ok");
        assert_eq!(outcome.capability, "embedding");
        assert_eq!(outcome.model_kind, "remote:openai");
        assert_eq!(outcome.model, "p/embed-model");
        assert!(outcome.success);
        match outcome.detail {
            TestDetail::Embedding { dimension } => assert_eq!(dimension, 4),
            other => panic!("expected Embedding detail, got {other:?}"),
        }
    }

    #[test]
    fn embedding_round_trip_surfaces_invalid_remote_vector() {
        // An empty remote embedding fails closed → RemoteEmbeddingInvalid/95.
        let _g = set_transport_override(|_spec| Ok(ok_embedding(&[])));
        let dir = tempfile::TempDir::new().unwrap();
        let paths = Paths::from_root(dir.path().to_path_buf());
        let cfg = config_remote_embedding(ProviderKind::Openai);
        let scope = crate::workspace::ResolvedScope::global_fallback();

        let err = test_embedding(&cfg, &paths, &scope).expect_err("empty embedding must fail");
        assert_eq!(err.exit_code(), 95);
    }

    #[test]
    fn embedding_round_trip_writes_no_meta() {
        // The round-trip must not write `meta.embedder_dimension` (read-only,
        // FR-019). It does not even open the DB for writing. Assert no index DB
        // file is created by the round-trip on a fresh root.
        let _g = set_transport_override(|_spec| Ok(ok_embedding(&[0.5, 0.5, 0.5])));
        let dir = tempfile::TempDir::new().unwrap();
        let paths = Paths::from_root(dir.path().to_path_buf());
        let cfg = config_remote_embedding(ProviderKind::Openai);
        let scope = crate::workspace::ResolvedScope::global_fallback();

        let _ = test_embedding(&cfg, &paths, &scope).unwrap();
        assert!(
            !paths.index_db.is_file(),
            "models test must not create or write the index DB"
        );
    }

    #[test]
    fn reranker_round_trip_reports_scored_ordering() {
        let _g = set_transport_override(|_spec| {
            Ok(RawResponse {
                status: 200,
                retry_after: None,
                body: serde_json::to_vec(&serde_json::json!({
                    "results": [
                        { "index": 1, "relevance_score": 0.9 },
                        { "index": 0, "relevance_score": 0.5 },
                        { "index": 2, "relevance_score": 0.1 },
                    ]
                }))
                .unwrap(),
            })
        });
        let dir = tempfile::TempDir::new().unwrap();
        let paths = Paths::from_root(dir.path().to_path_buf());
        let mut cfg = Config::default();
        cfg.providers.insert(
            "vp".to_string(),
            ProviderEntry {
                kind: ProviderKind::Voyage,
                base_url: None,
                api_key: Some(Secret::from("vk".to_string())),
            },
        );
        cfg.reranker.provider = Some("vp".to_string());
        cfg.reranker.model = Some("rerank-2".to_string());
        let scope = crate::workspace::ResolvedScope::global_fallback();

        let outcome = test_reranker(&cfg, &paths, &scope).expect("reranker round-trip ok");
        assert_eq!(outcome.capability, "reranker");
        assert_eq!(outcome.model_kind, "remote:voyage");
        assert_eq!(outcome.model, "vp/rerank-2");
        match outcome.detail {
            TestDetail::Reranker {
                candidates,
                scored,
                top_name,
            } => {
                assert_eq!(candidates, 3);
                assert_eq!(scored, 3);
                // Highest score (index 1 = "bravo").
                assert_eq!(top_name.as_deref(), Some("bravo"));
            }
            other => panic!("expected Reranker detail, got {other:?}"),
        }
    }

    #[test]
    fn summariser_round_trip_reports_char_counts() {
        // A remote summariser returns short + long via the transport seam.
        let _g = set_transport_override(|spec| {
            // openai chat-completions shape: choices[0].message.content. The
            // summariser issues a short then a long request; return a non-empty
            // body for both.
            let _ = spec;
            Ok(RawResponse {
                status: 200,
                retry_after: None,
                body: serde_json::to_vec(&serde_json::json!({
                    "choices": [{ "message": { "content": "a summary of the workspace" } }]
                }))
                .unwrap(),
            })
        });
        let dir = tempfile::TempDir::new().unwrap();
        let paths = Paths::from_root(dir.path().to_path_buf());
        let mut cfg = Config::default();
        cfg.providers.insert(
            "p".to_string(),
            ProviderEntry {
                kind: ProviderKind::Openai,
                base_url: None,
                api_key: Some(Secret::from("sk-key".to_string())),
            },
        );
        cfg.summariser.provider = Some("p".to_string());
        cfg.summariser.model = Some("gpt-4o-mini".to_string());

        let outcome = test_summariser(&cfg, &paths).expect("summariser round-trip ok");
        assert_eq!(outcome.capability, "summariser");
        assert_eq!(outcome.model_kind, "remote:openai");
        assert_eq!(outcome.model, "p/gpt-4o-mini");
        match outcome.detail {
            TestDetail::Summariser {
                short_chars,
                long_chars,
            } => {
                assert!(short_chars > 0, "short must be non-empty");
                assert!(long_chars > 0, "long must be non-empty");
            }
            other => panic!("expected Summariser detail, got {other:?}"),
        }
    }

    #[test]
    fn model_kind_label_bundled_when_no_provider() {
        let cfg = Config::default();
        assert_eq!(ModelKindLabel::for_embedding(&cfg).as_str(), "bundled");
        assert_eq!(ModelKindLabel::for_summariser(&cfg).as_str(), "bundled");
        assert_eq!(ModelKindLabel::for_reranker(&cfg).as_str(), "bundled");
    }

    // --- Issue #291: credential pre-flight (no doomed network call) ----------

    /// Serialises tests mutating `TOME_<NAME>_API_KEY` (process-global env) with
    /// the transport override (also process-global via `set_transport_override`).
    static PREFLIGHT_ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn config_remote_embedding_no_key() -> Config {
        let mut config = Config::default();
        config.providers.insert(
            "p".to_string(),
            ProviderEntry {
                kind: ProviderKind::Openai,
                base_url: None,
                api_key: None, // no inline key
            },
        );
        config.embedding.provider = Some("p".to_string());
        config.embedding.model = Some("embed-model".to_string());
        config
    }

    #[test]
    fn preflight_errors_93_before_any_network_call() {
        let _env = PREFLIGHT_ENV_MUTEX
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // SAFETY: guarded by PREFLIGHT_ENV_MUTEX.
        unsafe {
            std::env::remove_var("TOME_P_API_KEY");
        }
        // A transport override that PANICS if reached — proving the pre-flight
        // fails BEFORE any request is made.
        let _t = set_transport_override(|_spec| {
            panic!("network request must not be made when the credential is absent");
        });
        let cfg = config_remote_embedding_no_key();

        let err =
            crate::provider::credential_preflight(&cfg, capability_of(TestCapability::Embedding))
                .expect_err("missing credential must error");
        // A missing credential is a config problem → 93, NOT a request failure/94.
        assert_eq!(err.exit_code(), 93);
        let msg = err.to_string();
        assert!(
            msg.contains("TOME_P_API_KEY"),
            "must name the exact env var: {msg}"
        );
    }

    #[test]
    fn preflight_passes_when_inline_key_present() {
        let _env = PREFLIGHT_ENV_MUTEX
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // SAFETY: guarded by PREFLIGHT_ENV_MUTEX.
        unsafe {
            std::env::remove_var("TOME_P_API_KEY");
        }
        // Inline key present → pre-flight passes (the round-trip would proceed).
        let cfg = config_remote_embedding(ProviderKind::Openai);
        assert!(
            crate::provider::credential_preflight(&cfg, capability_of(TestCapability::Embedding))
                .is_ok(),
        );
    }

    #[test]
    fn preflight_noop_for_bundled_default() {
        // No provider configured → bundled path → pre-flight is a no-op for
        // every capability.
        let cfg = Config::default();
        for cap in [
            TestCapability::Embedding,
            TestCapability::Summariser,
            TestCapability::Reranker,
        ] {
            assert!(
                crate::provider::credential_preflight(&cfg, capability_of(cap)).is_ok(),
                "{cap:?}"
            );
        }
    }

    #[test]
    fn capability_of_maps_all_cli_variants() {
        use crate::provider::Capability;
        assert_eq!(
            capability_of(TestCapability::Embedding),
            Capability::Embedding
        );
        assert_eq!(
            capability_of(TestCapability::Summariser),
            Capability::Summariser
        );
        assert_eq!(
            capability_of(TestCapability::Reranker),
            Capability::Reranker
        );
    }
}
