//! F-MODEL-FILES positional-drift guard (Phase 7 beta hardening).
//!
//! `download_model` fetches the primary artefact from `entry.source_url` and
//! each non-primary file (`entry.files[1..]`) from `entry.aux_urls`, zipping
//! the two slices positionally. That contract is only sound if there is
//! exactly one aux URL per non-primary file:
//!
//! ```text
//! entry.files.len() == 1 + entry.aux_urls.len()
//! ```
//!
//! This fast (non-network) test asserts the invariant for EVERY registry
//! entry, so a future edit that adds a file without its URL (or vice versa)
//! fails in normal CI rather than silently shipping an incomplete download.

use tome::embedding::registry::MODEL_REGISTRY;

#[test]
fn every_entry_has_one_aux_url_per_non_primary_file() {
    for entry in MODEL_REGISTRY {
        assert!(
            !entry.files.is_empty(),
            "registry entry `{}` declares no files; the primary artefact is mandatory",
            entry.name,
        );
        assert_eq!(
            entry.files.len(),
            1 + entry.aux_urls.len(),
            "registry entry `{}` must have exactly one aux URL per non-primary file \
             (files={:?}, aux_urls={:?}); positional zip in download_model would drift",
            entry.name,
            entry.files,
            entry.aux_urls,
        );
    }
}
