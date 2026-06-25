//! Voyage AI wire shapes (embeddings + reranking).
//!
//! Phase 12 / US2 lands the **embedding** path. Voyage's `/embeddings`
//! endpoint shares the OpenAI success shape (`{ "data": [{ "embedding":
//! [..] }] }`), so the request shaping + response parsing is delegated to
//! [`crate::provider::openai`] — the ONE embeddings SSOT — with the only
//! divergence being the output-dimension body field (`output_dimension` vs
//! OpenAI's `dimensions`). Auth (`Authorization: Bearer`) is placed by
//! [`crate::provider::http`] for kind=voyage. Reranking (its own `/rerank`
//! shape) lands in US3, not here. Response structs stay LENIENT (FR-021).

use crate::provider::config::ResolvedProvider;
use crate::provider::error::ProviderError;
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
