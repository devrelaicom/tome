//! Voyage AI wire shapes (embeddings + reranking).
//!
//! Phase 12 / US2 lands the **embedding** path; US3 adds **reranking**.
//! Voyage's `/embeddings` endpoint shares the OpenAI success shape
//! (`{ "data": [{ "embedding": [..] }] }`), so the request shaping + response
//! parsing is delegated to [`crate::provider::openai`] — the ONE embeddings
//! SSOT — with the only divergence being the output-dimension body field
//! (`output_dimension` vs OpenAI's `dimensions`). Reranking has its own
//! `/rerank` shape ([`rerank`]). Auth (`Authorization: Bearer`) is placed by
//! [`crate::provider::http`] for kind=voyage. Response structs stay LENIENT —
//! they must NOT reject unknown response fields (FR-021, enforced by the
//! file-scoped lenient-modules gate under `tests/harness_settings/`).

use serde::{Deserialize, Serialize};

use crate::provider::config::ResolvedProvider;
use crate::provider::error::{ProviderError, ProviderErrorKind};
use crate::provider::http;
use crate::provider::openai::{self, EmbeddingDimensionsField};

/// Embed exactly one text against Voyage's `/embeddings` endpoint. Voyage uses
/// the OpenAI embeddings success shape, so this delegates to the shared
/// [`openai::embed_with_dimensions_field`] path, mapping the optional
/// authoritative dimension onto Voyage's `output_dimension` body field. Returns
/// the RAW vector — content validation is centralised in the shared validator
/// ([`crate::embedding::remote::validate_embedding`]).
///
/// FR-019 / contract note: Voyage embeddings can default to int8/binary dtypes;
/// an integer-typed JSON value round-trips into the response `Vec<f32>` and is
/// then caught by the shared finite/dimension validator — we don't need a
/// dtype-specific parser here.
pub fn embed_one(
    resolved: &ResolvedProvider,
    text: &str,
    dimensions: Option<u32>,
) -> Result<Vec<f32>, ProviderError> {
    openai::embed_with_dimensions_field(
        resolved,
        text,
        dimensions,
        EmbeddingDimensionsField::Voyage,
    )
}

// ---------------------------------------------------------------------------
// Reranking — `POST {base_url}/rerank` (Voyage only in v1).
// ---------------------------------------------------------------------------

/// The rerank request body. `return_documents: false` keeps the response
/// minimal — we only need the index→score mapping, never the echoed documents.
///
/// Shape note for a FUTURE Cohere/Jina addition: those providers use the SAME
/// request/response wire, differing only in the top-N field name (`top_n` vs
/// Voyage's `top_k`) and the host/path. To add one, parameterise the field name
/// the same way [`EmbeddingDimensionsField`] parameterises the embed dimension
/// key — do NOT fork this struct.
#[derive(Debug, Serialize)]
struct RerankRequest<'a> {
    model: &'a str,
    query: &'a str,
    documents: &'a [String],
    top_k: usize,
    return_documents: bool,
}

/// The rerank success response. LENIENT — only the two fields we read. An
/// OpenAI-style `error` object can ride a 200 on some compatible gateways; a
/// present `error` with no usable `results` ⇒ `BadRequest` (FR-013b).
#[derive(Debug, Deserialize)]
struct RerankResponse {
    #[serde(default)]
    results: Vec<RerankResult>,
    #[serde(default)]
    error: Option<ApiError>,
}

#[derive(Debug, Deserialize)]
struct RerankResult {
    /// Index INTO THE REQUEST `documents` array — the caller maps this back to
    /// its own `Vec<Candidate>` by this index, NEVER positionally.
    index: usize,
    relevance_score: f32,
}

/// A provider-side error object (`{ "message": "...", ... }`). LENIENT.
#[derive(Debug, Deserialize)]
struct ApiError {
    #[serde(default)]
    message: Option<String>,
}

/// Rerank `documents` against `query` via Voyage's `/rerank` endpoint, returning
/// `(index, relevance_score)` pairs in the order Voyage returns them (Voyage
/// already sorts by descending score, but the caller re-sorts defensively so the
/// final ordering never depends on the provider's sort stability).
///
/// `index` is the position INTO the input `documents` slice; the caller
/// ([`crate::embedding::remote::RemoteReranker`]) maps it back to its
/// `Vec<Candidate>` — NEVER positionally — and treats an out-of-range index as a
/// clear error (the contract guarantees `0 <= index < documents.len()`, but a
/// hostile/buggy provider must not be trusted).
///
/// `top_k` is how many ranked results to return (the caller passes
/// `documents.len()` to rerank ALL candidates). Returns the RAW pairs;
/// validation (out-of-range index detection) is the caller's job, mirroring how
/// embedding content validation is centralised in the shared validator.
pub fn rerank(
    resolved: &ResolvedProvider,
    query: &str,
    documents: &[String],
    top_k: usize,
) -> Result<Vec<(usize, f32)>, ProviderError> {
    let body = serde_json::to_value(RerankRequest {
        model: &resolved.model,
        query,
        documents,
        top_k,
        return_documents: false,
    })
    .map_err(|e| {
        ProviderError::new(
            &resolved.name,
            ProviderErrorKind::BadRequest,
            false,
            format!("failed to serialise rerank request: {e}"),
        )
    })?;

    let value = http::request_with_retry(resolved, "/rerank", &body)?;
    extract_rerank_results(&resolved.name, value)
}

/// Extract the `(index, score)` pairs from a parsed rerank response, detecting
/// the 200-with-error envelope (FR-013b). Returns the RAW pairs in Voyage's
/// order.
fn extract_rerank_results(
    provider: &str,
    value: serde_json::Value,
) -> Result<Vec<(usize, f32)>, ProviderError> {
    let response: RerankResponse = serde_json::from_value(value).map_err(|e| {
        ProviderError::new(
            provider,
            ProviderErrorKind::MalformedResponse,
            false,
            format!("rerank response did not match the expected shape: {e}"),
        )
    })?;

    // A 200-with-error envelope and no usable results ⇒ BadRequest, never a
    // silent empty (which would degrade to "candidates in arbitrary order").
    if response.results.is_empty()
        && let Some(err) = response.error
    {
        let detail = err
            .message
            .unwrap_or_else(|| "provider returned an error object on a 2xx".to_string());
        return Err(ProviderError::new(
            provider,
            ProviderErrorKind::BadRequest,
            false,
            detail,
        ));
    }

    Ok(response
        .results
        .into_iter()
        .map(|r| (r.index, r.relevance_score))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, ProviderEntry, ProviderKind, Secret};
    use crate::provider::config::{Capability, resolve};
    use crate::provider::http::{RawResponse, set_transport_override};

    /// A resolved Voyage reranker connection through the real `resolve` path.
    fn resolved_reranker() -> ResolvedProvider {
        let mut config = Config::default();
        config.providers.insert(
            "vp".to_string(),
            ProviderEntry {
                kind: ProviderKind::Voyage,
                base_url: None,
                api_key: Some(Secret::from("voyage-key".to_string())),
            },
        );
        config.reranker.provider = Some("vp".to_string());
        config.reranker.model = Some("rerank-2".to_string());
        resolve(&config, Capability::Reranker).unwrap().unwrap()
    }

    fn ok_rerank(results: serde_json::Value) -> RawResponse {
        RawResponse {
            status: 200,
            retry_after: None,
            body: serde_json::to_vec(&serde_json::json!({ "results": results })).unwrap(),
        }
    }

    #[test]
    fn rerank_shapes_request_to_rerank_endpoint() {
        let _g = set_transport_override(|spec| {
            assert!(
                spec.url.ends_with("/rerank"),
                "rerank must POST /rerank: {}",
                spec.url
            );
            assert!(
                spec.headers
                    .iter()
                    .any(|(k, v)| k == "Authorization" && v == "Bearer voyage-key"),
                "rerank must carry a Bearer header"
            );
            let body: serde_json::Value = serde_json::from_slice(&spec.body).unwrap();
            assert_eq!(body["model"], serde_json::json!("rerank-2"));
            assert_eq!(body["query"], serde_json::json!("find the alpha"));
            assert_eq!(body["return_documents"], serde_json::json!(false));
            assert_eq!(body["top_k"], serde_json::json!(2));
            let docs = body["documents"].as_array().unwrap();
            assert_eq!(docs.len(), 2);
            Ok(ok_rerank(serde_json::json!([
                { "index": 1, "relevance_score": 0.9 },
                { "index": 0, "relevance_score": 0.2 },
            ])))
        });
        let r = resolved_reranker();
        let docs = vec!["doc-a".to_string(), "doc-b".to_string()];
        let pairs = rerank(&r, "find the alpha", &docs, 2).unwrap();
        // Returned in the provider's order, NOT positional.
        assert_eq!(pairs, vec![(1, 0.9), (0, 0.2)]);
    }

    #[test]
    fn rerank_200_error_envelope_is_bad_request() {
        let _g = set_transport_override(|_spec| {
            Ok(RawResponse {
                status: 200,
                retry_after: None,
                body: serde_json::to_vec(&serde_json::json!({
                    "error": { "message": "rerank model not available" }
                }))
                .unwrap(),
            })
        });
        let r = resolved_reranker();
        let docs = vec!["doc-a".to_string()];
        let err = rerank(&r, "q", &docs, 1).unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::BadRequest);
        assert!(err.redacted_detail.contains("rerank model not available"));
    }

    #[test]
    fn rerank_unreachable_surfaces_provider_error() {
        // A 5xx exhausting retries → Unreachable/94 (retryable). The caller
        // maps this to ProviderRequestFailed/94, never a silent unranked result.
        let _g = set_transport_override(|_spec| {
            Ok(RawResponse {
                status: 503,
                retry_after: Some(std::time::Duration::from_secs(0)),
                body: Vec::new(),
            })
        });
        let r = resolved_reranker();
        let docs = vec!["doc-a".to_string()];
        let err = rerank(&r, "q", &docs, 1).unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::Unreachable);
        assert_eq!(err.into_tome_error().exit_code(), 94);
    }
}
