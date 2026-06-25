//! `RemoteEmbedder` — the BYOK/BYOM embedder (Phase 12 / US2) — plus the ONE
//! shared content-validation routine and the remote-vs-bundled construction
//! helpers.
//!
//! ## The load-bearing safety invariant
//!
//! The pre-mortem's defining failure mode is *silently indexing a
//! well-formed-but-wrong remote embedding*. The defence is a single validator
//! ([`validate_embedding`]) called from EXACTLY one place — inside
//! [`RemoteEmbedder::embed`]. Because every consumer (the index-time
//! `enable`/`reindex`/`catalog update` write paths, the query-time
//! `query::pipeline`, on BOTH the CLI and the MCP `search_skills` surface) embeds
//! through the `Embedder` trait's `embed()`, routing validation through the
//! trait method guarantees the SAME checks run at every production point. A
//! validation failure returns `Err(RemoteEmbeddingInvalid)` (exit 95 on the CLI;
//! a clear tool error on MCP), which — at write time — rolls back the enclosing
//! SQLite transaction, so nothing reaches `skill_embeddings` and the index is
//! left unchanged (FR-015).
//!
//! ## Sync-only
//!
//! Everything here is synchronous (`reqwest::blocking` underneath, via
//! `provider::http`). `tests/harness_settings/sync_boundary.rs` greps this tree;
//! nothing under `src/embedding/` may reach the async runtime.
//!
//! ## NFR-006 (bundled byte-identity)
//!
//! [`build_embedder`] / [`embedder_seed`] take the BUNDLED branch byte-for-byte
//! identically to the pre-Phase-12 path when no `[embedding]` provider is
//! configured: the existing `FastembedEmbedder::load(active_embedder, …)` and
//! the registry `MetaSeed`. In particular the bundled path NEVER writes
//! `meta.embedder_dimension` (a new meta row would change stored artefacts) —
//! that key is written only on the remote reindex path.

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::config::{Config, ProviderKind};
use crate::error::TomeError;
use crate::index::MetaSeed;
use crate::paths::Paths;
use crate::provider::config::ResolvedProvider;
use crate::provider::error::{ProviderError, ProviderErrorKind};
use crate::provider::{Capability, voyage};

use super::Embedder;
use super::fastembed::FastembedEmbedder;

/// The model-version sentinel recorded for ANY remote embedder. A remote model
/// has no pinned registry version; `"external"` is the stable identity token so
/// drift detection still fires on a `model_name` change (data-model §RemoteEmbedder).
pub const REMOTE_EMBEDDER_VERSION: &str = "external";

/// The ONE shared content-validation routine for a remote embedding (FR-015).
///
/// Checks, in order:
/// 1. non-empty,
/// 2. all values finite (no NaN / +∞ / −∞),
/// 3. when `expected_dim` is `Some(d)`: `vec.len() == d`.
///
/// A failure returns [`TomeError::RemoteEmbeddingInvalid`] (exit 95 on the CLI;
/// the same `TomeError` becomes a clear MCP tool error). The `detail` names the
/// specific check that failed so an operator can diagnose without seeing the
/// vector. This routine is the SINGLE point every remote embedding flows
/// through — today it is called from [`RemoteEmbedder::embed`] (and, once US4
/// lands, also the `tome models test embedding` round-trip), so the index path,
/// the query path, the CLI, and the MCP server can never diverge.
///
/// When `expected_dim` is `None` (no `[embedding] dimensions` set and no
/// persisted `meta.embedder_dimension` yet — the first embed of a fresh
/// reindex) the dimension check is skipped here; the caller
/// ([`RemoteEmbedder::embed`]) then ESTABLISHES the dimension from this first
/// successful vector so every subsequent embed in the run is asserted against it.
pub fn validate_embedding(vec: &[f32], expected_dim: Option<usize>) -> Result<(), TomeError> {
    if vec.is_empty() {
        return Err(TomeError::RemoteEmbeddingInvalid {
            detail: "remote embedding is empty (zero-length vector)".to_string(),
        });
    }
    if let Some(idx) = vec.iter().position(|f| !f.is_finite()) {
        return Err(TomeError::RemoteEmbeddingInvalid {
            detail: format!(
                "remote embedding contains a non-finite value (NaN/Inf) at index {idx}"
            ),
        });
    }
    if let Some(d) = expected_dim
        && vec.len() != d
    {
        return Err(TomeError::RemoteEmbeddingInvalid {
            detail: format!(
                "remote embedding dimension {} does not match the expected {d} \
                 (run `tome reindex --force` if you changed the embedding model)",
                vec.len()
            ),
        });
    }
    Ok(())
}

/// An [`Embedder`] backed by a remote provider's `/embeddings` endpoint
/// (OpenAI-compatible or Voyage). Each [`embed`](RemoteEmbedder::embed) makes
/// exactly one single-text HTTP request (FR-011), then runs the shared
/// [`validate_embedding`] before returning the vector.
///
/// `expected_dim` is an `AtomicUsize` (0 = "unset") rather than a `Cell` because
/// the [`Embedder`] trait requires `Send + Sync` and `Cell` is not `Sync`. It is
/// seeded from `[embedding] dimensions` (authoritative) OR a persisted
/// `meta.embedder_dimension` OR left unset; on the first successful embed of a
/// run with no seed, the established length is stored so the remainder of the
/// run is asserted against a single consistent dimension.
pub struct RemoteEmbedder {
    resolved: ResolvedProvider,
    /// `"<provider-name>/<model>"` — the stable `model_name()` identity.
    name: String,
    /// The provider kind, fixing which per-kind `embed_one` shapes the request.
    kind: ProviderKind,
    /// The authoritative `[embedding] dimensions`, if the user pinned one. When
    /// set it ALWAYS wins (it is passed on the wire AND validated); when unset
    /// the run establishes its own via `expected_dim`.
    requested_dim: Option<u32>,
    /// The dimension asserted for the rest of the run. `0` = unset (establish on
    /// first embed). Atomic so the type stays `Send + Sync`.
    expected_dim: AtomicUsize,
}

impl RemoteEmbedder {
    /// Construct from a resolved provider connection and the optional seed
    /// dimension. `seed_dim` is the `[embedding] dimensions` value if set, else a
    /// persisted `meta.embedder_dimension` (read at the construction site), else
    /// `None` (a fresh reindex establishes it).
    ///
    /// `requested_dim` (the `[embedding] dimensions` field) is tracked
    /// separately so it can be sent on the wire; when only a persisted dimension
    /// is known we validate against it but do NOT request it (the model already
    /// agreed to it on the reindex that persisted it).
    pub fn new(
        resolved: ResolvedProvider,
        requested_dim: Option<u32>,
        seed_dim: Option<usize>,
    ) -> Self {
        let name = format!("{}/{}", resolved.name, resolved.model);
        let kind = resolved.kind;
        Self {
            resolved,
            name,
            kind,
            requested_dim,
            expected_dim: AtomicUsize::new(seed_dim.unwrap_or(0)),
        }
    }

    /// The dimension this embedder is asserting (or established during the run),
    /// `None` if still unset. The reindex path reads this AFTER its first
    /// successful embed to persist `meta.embedder_dimension` (FR-015a).
    pub fn established_dimension(&self) -> Option<usize> {
        match self.expected_dim.load(Ordering::SeqCst) {
            0 => None,
            d => Some(d),
        }
    }

    /// Dispatch one single-text embedding request to the kind-appropriate
    /// per-kind module. openai/voyage share the same `/embeddings` shape; the
    /// only divergence (the output-dimension field name) is handled inside
    /// `voyage::embed_one`. A kind that `resolve()` rejects for the embedding
    /// capability (anthropic/gemini) is unreachable through the supported config
    /// path; fail closed with a `BadRequest` rather than panic.
    fn embed_one_remote(&self, text: &str) -> Result<Vec<f32>, ProviderError> {
        match self.kind {
            ProviderKind::Openai => {
                crate::provider::openai::embed_one(&self.resolved, text, self.requested_dim)
            }
            ProviderKind::Voyage => voyage::embed_one(&self.resolved, text, self.requested_dim),
            ProviderKind::Anthropic | ProviderKind::Gemini => Err(ProviderError::new(
                &self.resolved.name,
                ProviderErrorKind::BadRequest,
                false,
                "this provider kind is not a valid embedding provider",
            )),
        }
    }
}

impl Embedder for RemoteEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, TomeError> {
        // ONE single-text request (FR-011). A provider failure maps once
        // (`into_tome_error`) onto the closed TomeError set (94 for request
        // failures; the single-embedding-count check inside `embed_one` surfaces
        // a structural mismatch as MalformedResponse/94).
        let vector = self
            .embed_one_remote(text)
            .map_err(ProviderError::into_tome_error)?;

        // The dimension to assert: the run's established value (if any).
        let expected = self.established_dimension();

        // THE load-bearing validation. Fail-closed: at write time this Err rolls
        // back the enclosing transaction (nothing written); at query time it
        // surfaces as 95 (CLI) / a clear tool error (MCP).
        validate_embedding(&vector, expected)?;

        // Establish the run's dimension from the first valid vector when no seed
        // was provided, so every later embed in the run is checked against a
        // single consistent length. `compare_exchange` keeps the first writer's
        // value if two threads race (the embed loops are sequential today, but
        // the atomic keeps the invariant robust).
        if expected.is_none() {
            let _ = self.expected_dim.compare_exchange(
                0,
                vector.len(),
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
        }

        Ok(vector)
    }

    fn model_name(&self) -> &str {
        &self.name
    }

    fn model_version(&self) -> &str {
        REMOTE_EMBEDDER_VERSION
    }

    /// Surface the run-established dimension through the trait so the reindex
    /// path (which holds a `Box<dyn Embedder>`) can persist it. Delegates to the
    /// inherent accessor of the same name.
    fn established_dimension(&self) -> Option<usize> {
        RemoteEmbedder::established_dimension(self)
    }
}

// ---------------------------------------------------------------------------
// Remote-vs-bundled construction helpers (T047). The single chokepoint every
// embedder-constructing site routes through, so "remote vs bundled" is decided
// in exactly one place and the bundled path stays byte-identical (NFR-006).
// ---------------------------------------------------------------------------

/// Build the embedder a command should use: a [`RemoteEmbedder`] when
/// `[embedding] provider` references a configured provider, else the bundled
/// [`FastembedEmbedder`] for the active profile's registry entry.
///
/// - On the REMOTE branch the one-time first-run notice
///   ([`crate::provider::notice::notify_remote_use`]) fires (off-box text), and
///   the embedder is seeded with `expected_dim = [embedding] dimensions` (if
///   set) OR `persisted_dim` (a `meta.embedder_dimension` the caller read) OR
///   `None` (a fresh reindex establishes it).
/// - On the BUNDLED branch the behaviour and artefacts are IDENTICAL to today:
///   `active_embedder` resolves the active profile's registry entry and
///   `FastembedEmbedder::load` is called against its on-disk model dir.
///
/// `persisted_dim` is the `meta.embedder_dimension` the caller read from the
/// index (or `None` if absent / on the bundled path where it is unused).
pub fn build_embedder(
    cfg: &Config,
    paths: &Paths,
    active_embedder: &'static crate::embedding::registry::ModelEntry,
    persisted_dim: Option<usize>,
) -> Result<Box<dyn Embedder>, TomeError> {
    match crate::provider::resolve(cfg, Capability::Embedding)? {
        Some(resolved) => {
            crate::provider::notice::notify_remote_use(paths, &resolved.name);
            let requested_dim = cfg.embedding.dimensions;
            // The dimensions knob wins as the seed; else the persisted value.
            let seed_dim = requested_dim.map(|d| d as usize).or(persisted_dim);
            Ok(Box::new(RemoteEmbedder::new(
                resolved,
                requested_dim,
                seed_dim,
            )))
        }
        None => {
            let dir = paths.model_path(active_embedder.name)?;
            Ok(Box::new(FastembedEmbedder::load(active_embedder, &dir)?))
        }
    }
}

/// The drift-detection / `meta`-seed identity for the ACTIVE embedder:
/// `("<provider>/<model>", "external")` when an `[embedding]` provider is
/// configured, else the bundled registry entry's `(name, version)`.
///
/// This is the SSOT for "which embedder identity does the index believe in" —
/// the seed `query`/`reindex` pass to `detect_drift` and the seed
/// `plugin enable`/`catalog update` pass to `guard_embedder_drift` BOTH derive
/// from here, so switching `[embedding]` model surfaces as drift on every path.
/// A resolve failure (malformed reference) propagates — the same 93 the rest of
/// the command would hit — rather than silently falling back to bundled.
pub fn embedder_seed(
    cfg: &Config,
    active_embedder: &'static crate::embedding::registry::ModelEntry,
) -> Result<MetaSeed, TomeError> {
    match crate::provider::resolve(cfg, Capability::Embedding)? {
        Some(resolved) => Ok(MetaSeed {
            name: format!("{}/{}", resolved.name, resolved.model),
            version: REMOTE_EMBEDDER_VERSION.to_string(),
        }),
        None => Ok(MetaSeed {
            name: active_embedder.name.to_owned(),
            version: active_embedder.version.to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- validate_embedding: the load-bearing fail-closed checks --------------

    #[test]
    fn validate_rejects_empty() {
        let err = validate_embedding(&[], None).unwrap_err();
        assert_eq!(err.exit_code(), 95);
        assert!(err.to_string().contains("empty"), "{err}");
    }

    #[test]
    fn validate_rejects_nan() {
        let err = validate_embedding(&[0.1, f32::NAN, 0.3], Some(3)).unwrap_err();
        assert_eq!(err.exit_code(), 95);
        assert!(err.to_string().contains("non-finite"), "{err}");
    }

    #[test]
    fn validate_rejects_positive_infinity() {
        let err = validate_embedding(&[0.1, f32::INFINITY], Some(2)).unwrap_err();
        assert_eq!(err.exit_code(), 95);
    }

    #[test]
    fn validate_rejects_negative_infinity() {
        let err = validate_embedding(&[f32::NEG_INFINITY], Some(1)).unwrap_err();
        assert_eq!(err.exit_code(), 95);
    }

    #[test]
    fn validate_rejects_wrong_dimension() {
        let err = validate_embedding(&[0.1, 0.2, 0.3], Some(4)).unwrap_err();
        assert_eq!(err.exit_code(), 95);
        let msg = err.to_string();
        assert!(msg.contains('3') && msg.contains('4'), "{msg}");
    }

    #[test]
    fn validate_accepts_correct_dimension() {
        assert!(validate_embedding(&[0.1, 0.2, 0.3], Some(3)).is_ok());
    }

    #[test]
    fn validate_skips_dimension_check_when_expected_none() {
        // No expected dim yet (fresh reindex, first embed): only empty/finite
        // are enforced; any non-empty finite length passes.
        assert!(validate_embedding(&[0.1, 0.2, 0.3, 0.4, 0.5], None).is_ok());
    }

    #[test]
    fn validate_rejects_empty_even_when_dim_unset() {
        // An empty vector is rejected regardless of the dimension knob.
        assert_eq!(validate_embedding(&[], None).unwrap_err().exit_code(), 95);
    }

    // --- RemoteEmbedder: validation runs through the trait method --------------

    use crate::config::{ProviderEntry, Secret};
    use crate::provider::config::{Capability, resolve};
    use crate::provider::http::{RawResponse, set_transport_override};

    fn remote(kind: ProviderKind, dimensions: Option<u32>, seed: Option<usize>) -> RemoteEmbedder {
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
        config.embedding.dimensions = dimensions;
        let resolved = resolve(&config, Capability::Embedding).unwrap().unwrap();
        RemoteEmbedder::new(resolved, dimensions, seed)
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
    fn embed_returns_validated_vector_and_establishes_dimension() {
        let _g = set_transport_override(|_spec| Ok(ok_embedding(&[0.1, 0.2, 0.3, 0.4])));
        let e = remote(ProviderKind::Openai, None, None);
        assert_eq!(e.established_dimension(), None, "unset before first embed");
        let v = e.embed("hello").unwrap();
        assert_eq!(v, vec![0.1, 0.2, 0.3, 0.4]);
        assert_eq!(
            e.established_dimension(),
            Some(4),
            "first embed establishes the run dimension"
        );
    }

    #[test]
    fn embed_rejects_wrong_dimension_against_seed() {
        // Seeded to 5; the provider returns 4 → RemoteEmbeddingInvalid/95.
        let _g = set_transport_override(|_spec| Ok(ok_embedding(&[0.1, 0.2, 0.3, 0.4])));
        let e = remote(ProviderKind::Openai, Some(5), Some(5));
        let err = e.embed("hello").unwrap_err();
        assert_eq!(err.exit_code(), 95);
    }

    #[test]
    fn embed_rejects_out_of_range_numeric_fail_closed() {
        // A JSON number that overflows `f32` (`1e400`) is rejected during
        // deserialisation into the `Vec<f32>` response shape → MalformedResponse
        // (94), BEFORE the validator. The point is that it fails CLOSED: nothing
        // is returned, so no corrupt vector reaches the index. (The validator's
        // finite check — NaN/±Inf — is exercised directly by the
        // `validate_rejects_*` unit tests above; JSON has no NaN/Inf literal, so
        // a non-finite f32 can only originate from an out-of-range numeric, and
        // serde rejects that at parse time.)
        let _g = set_transport_override(|_spec| {
            Ok(RawResponse {
                status: 200,
                retry_after: None,
                body: br#"{"data":[{"embedding":[0.1, 1e400]}]}"#.to_vec(),
            })
        });
        let e = remote(ProviderKind::Openai, None, None);
        let err = e.embed("hello").unwrap_err();
        let code = err.exit_code();
        assert!(
            code == 94 || code == 95,
            "out-of-range numeric must fail closed (94 or 95), got {code}: {err}"
        );
        assert_eq!(
            e.established_dimension(),
            None,
            "a rejected embedding must NOT establish a run dimension"
        );
    }

    #[test]
    fn embed_rejects_empty_embedding() {
        let _g = set_transport_override(|_spec| Ok(ok_embedding(&[])));
        let e = remote(ProviderKind::Openai, None, None);
        let err = e.embed("hello").unwrap_err();
        assert_eq!(err.exit_code(), 95);
    }

    #[test]
    fn embed_rejects_multi_embedding_response() {
        // data.len() != 1 → MalformedResponse/94 (structural FR-011 contract,
        // not a content failure). It still fails closed — nothing is returned.
        let _g = set_transport_override(|_spec| {
            Ok(RawResponse {
                status: 200,
                retry_after: None,
                body: serde_json::to_vec(&serde_json::json!({
                    "data": [
                        { "embedding": [0.1, 0.2] },
                        { "embedding": [0.3, 0.4] }
                    ]
                }))
                .unwrap(),
            })
        });
        let e = remote(ProviderKind::Openai, None, None);
        let err = e.embed("hello").unwrap_err();
        assert_eq!(
            err.exit_code(),
            94,
            "multi-embedding is a malformed response"
        );
    }

    #[test]
    fn model_name_is_provider_slash_model_and_version_external() {
        let e = remote(ProviderKind::Openai, None, None);
        assert_eq!(e.model_name(), "p/embed-model");
        assert_eq!(e.model_version(), "external");
    }

    #[test]
    fn voyage_kind_uses_embedding_path() {
        // Voyage embeds through the shared openai shape; confirm a valid round
        // trip works (the request-field divergence is exercised in the
        // integration suite via the transport seam).
        let _g = set_transport_override(|_spec| Ok(ok_embedding(&[0.5, 0.5])));
        let e = remote(ProviderKind::Voyage, None, None);
        let v = e.embed("doc").unwrap();
        assert_eq!(v, vec![0.5, 0.5]);
    }

    // --- embedder_seed: identity reflects remote vs bundled -------------------

    #[test]
    fn embedder_seed_remote_is_provider_slash_model_external() {
        let mut config = Config::default();
        config.providers.insert(
            "vp".to_string(),
            ProviderEntry {
                kind: ProviderKind::Voyage,
                base_url: None,
                api_key: None,
            },
        );
        config.embedding.provider = Some("vp".to_string());
        config.embedding.model = Some("voyage-3".to_string());
        let bundled = crate::embedding::profile::embedder_for(crate::embedding::Profile::DEFAULT);
        let seed = embedder_seed(&config, bundled).unwrap();
        assert_eq!(seed.name, "vp/voyage-3");
        assert_eq!(seed.version, "external");
    }

    #[test]
    fn embedder_seed_bundled_is_registry_identity() {
        let config = Config::default();
        let bundled = crate::embedding::profile::embedder_for(crate::embedding::Profile::DEFAULT);
        let seed = embedder_seed(&config, bundled).unwrap();
        assert_eq!(seed.name, bundled.name);
        assert_eq!(seed.version, bundled.version);
    }
}
