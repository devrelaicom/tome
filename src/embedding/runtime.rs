//! Inference runtime hook.
//!
//! `fastembed` v4 owns the ONNX Runtime lifecycle internally: the first
//! `TextEmbedding::try_new*` call initialises the global `ort` environment,
//! and subsequent calls reuse it. We do not link `ort` directly (it is a
//! transitive dependency only); this module exists so callers have a
//! single place to surface inference-init failures via the closed error set.
//!
//! When the foundational phase needs to short-circuit init errors before a
//! model is touched (e.g. for `tome status` output), [`ensure_initialised`]
//! returns `Ok(())` as a no-op placeholder. The first real
//! `FastembedEmbedder::load` then carries the actual init failure path.
//! If a future direct dependency on `ort` is added, the body of this
//! function becomes the lazy init point.

use crate::error::TomeError;

/// No-op placeholder for inference-runtime initialisation. See the module
/// docs for why this is currently a stub.
pub fn ensure_initialised() -> Result<(), TomeError> {
    Ok(())
}
