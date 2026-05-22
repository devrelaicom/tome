//! Thin wrapper around [`crate::embedding::download::download_model`]
//! that selects the summariser entry from [`MODEL_REGISTRY`] and
//! threads through the optional byte-progress callback added in F6.
//!
//! F6 ships the seam; the first byte-progress consumer (a real
//! `indicatif::byte_bar`) lands in US4.a's `tome workspace regen-summary`
//! surface. Until then every internal caller passes `None`.
//!
//! The download path inherits the registry's placeholder-checksum
//! guard from [`ModelEntry::has_placeholder_checksum`] — F6's
//! placeholder hash for `qwen2.5-0.5b-instruct` means a stray call
//! here returns `ModelCorrupt` (exit 31) rather than silently
//! installing untrusted bytes.

use crate::embedding::download::download_model;
use crate::embedding::registry::ModelManifest;
use crate::error::TomeError;
use crate::paths::Paths;

use super::registry::summariser_entry;

/// Download the summariser model into `<paths.models_dir>/qwen2.5-0.5b-instruct/`.
///
/// `byte_progress` is an optional `(bytes_so_far, total_bytes)`
/// callback fired once per streamed chunk. The first real consumer is
/// US4.a's regen-summary CLI surface.
pub fn download_summariser_model(
    paths: &Paths,
    byte_progress: Option<&dyn Fn(u64, u64)>,
) -> Result<ModelManifest, TomeError> {
    let entry = summariser_entry();
    download_model(entry, &paths.models_dir, byte_progress)
}
