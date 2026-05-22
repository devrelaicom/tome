//! Atomic, SIGINT-aware model artefact downloader.
//!
//! Workflow (FR-020a, research §R5):
//!
//! 1. Create a sibling `.partial/` directory next to the target model dir.
//! 2. Stream the HTTP body chunk-by-chunk through a `Sha256` while writing
//!    to `<.partial>/<filename>`. After every chunk, peek the global
//!    cancellation flag (FR-053); on cancel, abort and remove the
//!    `.partial/` tree.
//! 3. On EOF, hex-compare the streaming digest against the registry's
//!    pinned hash. Mismatch → `ModelChecksumMismatch` (exit 32) and remove
//!    `.partial/`.
//! 4. `fsync` the file, then rename the `.partial/` directory to its final
//!    name. The rename is the atomicity boundary — readers either see the
//!    old directory (or none) or the new one, never a half-extracted state.
//! 5. Write `manifest.json` atomically via `tempfile::NamedTempFile`.
//!
//! Network and IO errors map to [`TomeError::Io`] (exit 7); checksum failures
//! map to [`TomeError::ModelChecksumMismatch`] (exit 32); a placeholder
//! registry hash maps to [`TomeError::ModelCorrupt`] with a clear remediation
//! pointer (exit 31).

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use time::OffsetDateTime;

use crate::catalog::git;
use crate::embedding::registry::{ModelEntry, ModelManifest};
use crate::error::TomeError;

const STREAM_CHUNK_SIZE: usize = 64 * 1024;

/// Download `entry` into `model_root`. The final installed location is
/// `model_root/<entry.name>/...` and the manifest path is
/// `model_root/<entry.name>/manifest.json`.
///
/// The HTTP `entry.source_url` MUST point at the **primary** artefact (the
/// ONNX model file). Other files in `entry.files` are not downloaded here;
/// future per-file downloads will reuse this same atomic-rename strategy.
///
/// `byte_progress` is an optional callback invoked once after every
/// streamed chunk with `(bytes_so_far, total_bytes)`. `total_bytes` is
/// the pinned `entry.size_bytes`; the network's `Content-Length` is not
/// consulted (it can disagree with the registry pin for redirected URLs
/// and the registry pin is authoritative anyway). Callers that don't
/// want progress pass `None` and inherit the F2-era spinner-only UX.
/// Phase 4 / F6 introduces this seam; first byte-progress consumer
/// lands in US4.a's summariser download surface.
pub fn download_model(
    entry: &ModelEntry,
    model_root: &Path,
    byte_progress: Option<&dyn Fn(u64, u64)>,
) -> Result<ModelManifest, TomeError> {
    if entry.has_placeholder_checksum() {
        return Err(TomeError::ModelCorrupt {
            model: entry.name.to_owned(),
            detail: "registry checksum is an unverified placeholder; \
                     this Tome build cannot install models until the registry is pinned"
                .to_owned(),
        });
    }

    let final_dir = model_root.join(entry.name);
    let partial_dir = final_dir.with_extension("partial");
    let primary_filename = entry.files.first().copied().unwrap_or("model.onnx");

    if partial_dir.exists() {
        std::fs::remove_dir_all(&partial_dir).map_err(TomeError::Io)?;
    }
    std::fs::create_dir_all(&partial_dir).map_err(TomeError::Io)?;

    // Run the full pipeline inside a single closure so a failure at any
    // step (stream, verify, rename, manifest write) is followed by partial
    // cleanup. Without this, a checksum mismatch — which is detected
    // *after* `stream_to_partial` returns Ok — would leak the .partial dir.
    let pipeline = || -> Result<ModelManifest, TomeError> {
        let observed_hash =
            stream_to_partial(entry, &partial_dir.join(primary_filename), byte_progress)?;
        verify_checksum(entry, &observed_hash)?;
        if final_dir.exists() {
            std::fs::remove_dir_all(&final_dir).map_err(TomeError::Io)?;
        }
        std::fs::rename(&partial_dir, &final_dir).map_err(TomeError::Io)?;
        write_manifest(entry, &final_dir)
    };

    match pipeline() {
        Ok(manifest) => Ok(manifest),
        Err(err) => {
            // Best effort: the partial dir may already have been renamed
            // (e.g. if `write_manifest` failed) — in that case the remove
            // is a no-op and the user is left with a renamed dir + missing
            // manifest, which `tome status` flags as Corrupt on next open.
            let _ = std::fs::remove_dir_all(&partial_dir);
            Err(err)
        }
    }
}

fn stream_to_partial(
    entry: &ModelEntry,
    dest: &Path,
    byte_progress: Option<&dyn Fn(u64, u64)>,
) -> Result<String, TomeError> {
    // `reqwest::Error::Display` reproduces the failing URL verbatim, which
    // can include presigned-URL query parameters carrying credentials. Run
    // both the error message and the (non-error) status-line message
    // through the credential scrubber before they reach `TomeError`.
    let mut response = reqwest::blocking::get(entry.source_url).map_err(|e| {
        TomeError::Io(std::io::Error::other(scrub_for_diag(&format!(
            "HTTP get failed: {e}"
        ))))
    })?;

    if !response.status().is_success() {
        return Err(TomeError::Io(std::io::Error::other(scrub_for_diag(
            &format!("HTTP {} fetching {}", response.status(), entry.source_url),
        ))));
    }

    let mut file = File::create(dest).map_err(TomeError::Io)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; STREAM_CHUNK_SIZE];
    let total = entry.size_bytes;
    let mut written: u64 = 0;

    loop {
        if git::was_cancelled() {
            return Err(TomeError::Interrupted);
        }
        let n = response.read(&mut buf).map_err(TomeError::Io)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(TomeError::Io)?;
        written = written.saturating_add(n as u64);
        if let Some(cb) = byte_progress {
            cb(written, total);
        }
    }

    file.sync_all().map_err(TomeError::Io)?;
    Ok(hex::encode(hasher.finalize()))
}

fn verify_checksum(entry: &ModelEntry, observed_hex: &str) -> Result<(), TomeError> {
    if observed_hex.eq_ignore_ascii_case(entry.sha256) {
        Ok(())
    } else {
        Err(TomeError::ModelChecksumMismatch {
            model: entry.name.to_owned(),
            expected: entry.sha256.to_owned(),
            got: observed_hex.to_owned(),
        })
    }
}

fn write_manifest(entry: &ModelEntry, final_dir: &Path) -> Result<ModelManifest, TomeError> {
    let manifest = ModelManifest {
        name: entry.name.to_owned(),
        version: entry.version.to_owned(),
        kind: entry.kind,
        source_url: entry.source_url.to_owned(),
        sha256: entry.sha256.to_owned(),
        size_bytes: entry.size_bytes,
        licence: entry.licence.to_owned(),
        files: entry.files.iter().map(|s| (*s).to_owned()).collect(),
        installed_at: OffsetDateTime::now_utc(),
    };

    let body = serde_json::to_vec_pretty(&manifest).map_err(|e| {
        TomeError::ModelRegistrationParseError {
            file: final_dir.join("manifest.json"),
            message: format!("serialise: {e}"),
        }
    })?;

    let temp = NamedTempFile::new_in(final_dir).map_err(TomeError::Io)?;
    let mut handle = temp.as_file();
    handle.write_all(&body).map_err(TomeError::Io)?;
    handle.sync_all().map_err(TomeError::Io)?;
    let manifest_path: PathBuf = final_dir.join("manifest.json");
    temp.persist(&manifest_path)
        .map_err(|e| TomeError::Io(e.error))?;

    Ok(manifest)
}

// Bring `Read::read` into scope so the explicit byte loop above compiles
// against `reqwest::blocking::Response`'s `Read` impl.
use std::io::Read;

/// Streaming SHA-256 of `path`'s contents. Used by `tome models list --verify`
/// and tests that need to confirm an on-disk artefact's integrity. Reads in
/// fixed-size chunks so a several-hundred-MB model rehash stays bounded in
/// memory.
pub fn sha256_file(path: &Path) -> Result<String, TomeError> {
    let mut file = File::open(path).map_err(TomeError::Io)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; STREAM_CHUNK_SIZE];
    loop {
        if git::was_cancelled() {
            return Err(TomeError::Interrupted);
        }
        let n = file.read(&mut buf).map_err(TomeError::Io)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Wrap a diagnostic string through the credential scrubber so presigned
/// URL query strings, `Authorization: Bearer` headers, and the like are
/// redacted before the message lands in `TomeError`.
fn scrub_for_diag(text: &str) -> String {
    git::scrub_to_string(text.as_bytes())
}
