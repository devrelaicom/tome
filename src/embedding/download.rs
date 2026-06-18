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
//! 4. Stream every non-primary file (`entry.files[1..]`, fetched from
//!    `entry.aux_urls` positionally — e.g. `tokenizer.json` + the optional
//!    fastembed config files) into the same `.partial/` directory. These are
//!    not checksum-verified (the registry only pins the primary's size +
//!    sha), consistent with `verify`'s design, so each aux stream is bounded
//!    by [`AUX_FILE_MAX`] (64 MiB) to deny an unbounded sidecar from a
//!    compromised pinned host (over-cap → [`TomeError::ModelCorrupt`]).
//!    Doing this BEFORE the rename keeps the all-or-nothing landing: a
//!    failed (or over-cap) aux fetch leaves the `.partial/` tree, which the
//!    error arm removes.
//! 5. `fsync` each file, then rename the `.partial/` directory to its final
//!    name. The rename is the atomicity boundary — readers either see the
//!    old directory (or none) or the new one, never a half-extracted state.
//! 6. Write `manifest.toml` atomically via `tempfile::NamedTempFile`.
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

/// Byte cap for an auxiliary model file (tokenizer/config sidecar).
///
/// Aux files (`entry.files[1..]`, fetched from `entry.aux_urls`) carry no
/// pinned size and are NOT SHA-256-verified, unlike the primary artefact —
/// so without a cap a compromised or MITM'd pinned host could stream an
/// unbounded sidecar and exhaust memory/disk (P9 MAJOR-2). 64 MiB is
/// deliberately generous: the largest real aux file is the bge-reranker
/// `tokenizer.json` at ~17 MiB, so this leaves comfortable headroom while
/// still bounding the worst case. Over-cap surfaces as
/// [`TomeError::ModelCorrupt`] (exit 31, reusing the existing variant) and
/// the staged `.partial/` directory is removed by `download_model`'s
/// error-cleanup closure.
///
/// The primary artefact needs no analogous cap here: its `entry.size_bytes`
/// pin + post-stream SHA-256 verify already make an over-long or tampered
/// body fail closed.
const AUX_FILE_MAX: u64 = 64 * 1024 * 1024;

#[cfg(test)]
thread_local! {
    /// Test-only override for [`AUX_FILE_MAX`]. When `Some`, the aux-file
    /// streamer enforces this value instead of the 64 MiB production cap,
    /// so a deterministic unit test can prove the over-cap abort + partial
    /// cleanup against a tiny served body rather than 64 MiB of bytes.
    ///
    /// Mirrors the `thread_local! RefCell<Option<_>>` + RAII-guard injection
    /// pattern used elsewhere (`summarise::trigger::SUMMARISER_OVERRIDE`).
    /// Gated behind `cfg(test)`: this fix's test lives in the same crate
    /// (`#[cfg(test)] mod tests` below), so no published-API exposure is
    /// needed and the override compiles out of release builds entirely.
    static AUX_FILE_MAX_OVERRIDE: std::cell::RefCell<Option<u64>> =
        const { std::cell::RefCell::new(None) };
}

/// The effective aux-file cap: the test override when installed, else the
/// production [`AUX_FILE_MAX`]. In release builds this is a constant.
#[inline]
fn aux_file_max() -> u64 {
    #[cfg(test)]
    {
        if let Some(v) = AUX_FILE_MAX_OVERRIDE.with(|slot| *slot.borrow()) {
            return v;
        }
    }
    AUX_FILE_MAX
}

/// RAII guard installing a tiny [`AUX_FILE_MAX_OVERRIDE`] for one test, then
/// clearing it on drop (including on panic). Lets a deterministic test prove
/// the over-cap abort against a small served body instead of 64 MiB.
#[cfg(test)]
struct AuxCapOverrideGuard;

#[cfg(test)]
impl AuxCapOverrideGuard {
    fn install(max_bytes: u64) -> Self {
        AUX_FILE_MAX_OVERRIDE.with(|slot| *slot.borrow_mut() = Some(max_bytes));
        Self
    }
}

#[cfg(test)]
impl Drop for AuxCapOverrideGuard {
    fn drop(&mut self) {
        AUX_FILE_MAX_OVERRIDE.with(|slot| *slot.borrow_mut() = None);
    }
}

/// Download `entry` into `model_root`. The final installed location is
/// `model_root/<entry.name>/...` and the manifest path is
/// `model_root/<entry.name>/manifest.toml`.
///
/// The HTTP `entry.source_url` MUST point at the **primary** artefact (the
/// ONNX model file). Every non-primary file in `entry.files[1..]` is fetched
/// from `entry.aux_urls` (positionally) into the same staging directory
/// before the atomic rename, so a successful call leaves a COMPLETE, loadable
/// model directory — `FastembedEmbedder::load` needs `tokenizer.json`, which
/// is a non-primary file. Single-file models (the summariser) have an empty
/// `aux_urls` and this loop is a no-op.
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

        // Fetch every non-primary file (tokenizer.json + optional fastembed
        // config files) BEFORE the rename, so the landed directory is
        // complete-or-absent. `entry.aux_urls` pairs positionally with
        // `entry.files[1..]`; the invariant `files.len() == 1 + aux_urls.len()`
        // is checked by the `model_registry_invariant` test. The
        // `debug_assert!` catches a future edit that breaks the pairing for an
        // entry that actually reaches this path (stub entries never do).
        debug_assert!(
            entry.files.len() == 1 + entry.aux_urls.len(),
            "model `{}`: files ({}) must be 1 + aux_urls ({}) — positional zip drift",
            entry.name,
            entry.files.len(),
            entry.aux_urls.len(),
        );
        for (local_name, url) in entry.files.iter().skip(1).zip(entry.aux_urls.iter()) {
            // Aux files are not checksum-verified (the registry pins only the
            // primary's size + sha); `None` progress because there is no
            // pinned size to drive a bar against. Because they are unverified
            // and unsized, the stream is byte-capped (`AUX_FILE_MAX`) so a
            // compromised pinned host cannot serve an unbounded sidecar.
            // Scrubbing is preserved — `stream_url_to_partial` runs the URL +
            // reqwest error chain through the credential scrubber exactly as
            // the primary fetch.
            stream_url_to_partial(
                url,
                &partial_dir.join(local_name),
                None,
                None,
                Some(AuxCap {
                    max_bytes: aux_file_max(),
                    model: entry.name,
                    file: local_name,
                }),
            )?;
        }

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

/// Stream `entry.source_url` (the primary artefact) into `dest`, returning the
/// streaming SHA-256 for `verify_checksum`. Thin wrapper over
/// [`stream_url_to_partial`] that supplies the primary's pinned size for the
/// progress bar.
fn stream_to_partial(
    entry: &ModelEntry,
    dest: &Path,
    byte_progress: Option<&dyn Fn(u64, u64)>,
) -> Result<String, TomeError> {
    // No `AuxCap` for the primary: its pinned `size_bytes` + post-stream
    // SHA-256 verify already bound and authenticate the body.
    stream_url_to_partial(
        entry.source_url,
        dest,
        Some(entry.size_bytes),
        byte_progress,
        None,
    )
}

/// Byte-cap descriptor for an unverified, unsized auxiliary file fetch.
/// Carries the model + file names so an over-cap abort can surface a
/// precise [`TomeError::ModelCorrupt`] message.
struct AuxCap<'a> {
    max_bytes: u64,
    model: &'a str,
    file: &'a str,
}

/// Stream an arbitrary `(url, dest)` pair through a `Sha256`, returning the
/// streaming digest. Used for both the primary artefact (size known, progress
/// driven, no `aux_cap`) and the non-primary aux files (size unknown, `None`
/// progress, byte-capped via `aux_cap`).
///
/// `total_for_progress` is the byte count reported to `byte_progress` as the
/// second argument; the network's `Content-Length` is intentionally NOT
/// consulted (it can disagree with the registry pin for redirected URLs and
/// the pin is authoritative). When `None`, `byte_progress` is not invoked.
///
/// `aux_cap`, when `Some`, bounds the streamed body: once cumulative bytes
/// written would exceed `aux_cap.max_bytes`, the stream aborts with
/// [`TomeError::ModelCorrupt`] (the partially written file is left in the
/// `.partial/` dir for `download_model`'s error-cleanup closure to remove).
/// The primary path passes `None` because its `size_bytes` pin + SHA-256
/// verify already bound and authenticate the body; the `Content-Length`
/// remains untrusted on both paths.
///
/// CREDENTIAL SCRUBBING: `reqwest::Error::Display` and the status-line message
/// reproduce the failing URL verbatim, which can include presigned-URL query
/// parameters carrying credentials. Both are run through the credential
/// scrubber before reaching `TomeError` — this MUST hold for aux fetches too.
fn stream_url_to_partial(
    url: &str,
    dest: &Path,
    total_for_progress: Option<u64>,
    byte_progress: Option<&dyn Fn(u64, u64)>,
    aux_cap: Option<AuxCap<'_>>,
) -> Result<String, TomeError> {
    let mut response = reqwest::blocking::get(url).map_err(|e| {
        TomeError::Io(std::io::Error::other(scrub_for_diag(&format!(
            "HTTP get failed: {e}"
        ))))
    })?;

    if !response.status().is_success() {
        return Err(TomeError::Io(std::io::Error::other(scrub_for_diag(
            &format!("HTTP {} fetching {}", response.status(), url),
        ))));
    }

    let mut file = File::create(dest).map_err(TomeError::Io)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; STREAM_CHUNK_SIZE];
    let mut written: u64 = 0;

    loop {
        if git::was_cancelled() {
            return Err(TomeError::Interrupted);
        }
        let n = response.read(&mut buf).map_err(TomeError::Io)?;
        if n == 0 {
            break;
        }
        written = written.saturating_add(n as u64);
        // Enforce the aux byte cap BEFORE writing this chunk, so an
        // unbounded sidecar from a compromised pinned host can never grow
        // the on-disk partial past the cap (memory stays bounded by the
        // fixed-size `buf` regardless). The over-cap error names the model
        // + file and reuses the existing `ModelCorrupt` variant (exit 31) —
        // no new error variant / exit code.
        if let Some(cap) = &aux_cap
            && written > cap.max_bytes
        {
            let mib = cap.max_bytes / (1024 * 1024);
            return Err(TomeError::ModelCorrupt {
                model: cap.model.to_owned(),
                detail: format!("{} exceeded the {} MiB aux-file cap", cap.file, mib),
            });
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(TomeError::Io)?;
        if let (Some(cb), Some(total)) = (byte_progress, total_for_progress) {
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

    let manifest_path: PathBuf = final_dir.join("manifest.toml");
    let body = manifest.to_toml(&manifest_path)?;

    let temp = NamedTempFile::new_in(final_dir).map_err(TomeError::Io)?;
    let mut handle = temp.as_file();
    handle.write_all(body.as_bytes()).map_err(TomeError::Io)?;
    handle.sync_all().map_err(TomeError::Io)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;
    use tempfile::TempDir;

    /// Sequential one-shot HTTP server: serves `responses` in order, one per
    /// accepted connection (the downloader opens a fresh connection per
    /// `reqwest::blocking::get`). Returns the bound base URL; callers append
    /// the per-file path. Mirrors the hand-rolled server in
    /// `tests/model_download.rs` but serves more than one request so we can
    /// satisfy the primary fetch + the aux fetch.
    fn spawn_sequential_server(responses: Vec<Vec<u8>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1:0");
        let addr = listener.local_addr().expect("local_addr");
        thread::spawn(move || {
            for response in responses {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut sink = [0u8; 4096];
                        let _ = stream.read(&mut sink);
                        let _ = stream.write_all(&response);
                        let _ = stream.flush();
                    }
                    Err(_) => break,
                }
            }
        });
        format!("http://{addr}")
    }

    fn http_200(body: &[u8]) -> Vec<u8> {
        let mut out = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        out.extend_from_slice(body);
        out
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }

    /// Leak a two-file (primary + one aux) entry whose URLs point at `base`.
    /// `ModelEntry`'s `&'static str` fields force the leak; bounded by the
    /// test binary's lifetime, which is fine for a fixture.
    fn two_file_entry(base: &str, primary_sha: String, primary_size: u64) -> &'static ModelEntry {
        let primary_url = format!("{base}/model.onnx");
        let aux_url = format!("{base}/tokenizer.json");
        // Leak the aux-URL *slice* (not just the strings) so it satisfies the
        // `&'static [&'static str]` field; a bare `&[Box::leak(..)]` would be
        // a temporary freed at the end of the statement.
        let aux_urls: &'static [&'static str] =
            Box::leak(vec![&*Box::leak(aux_url.into_boxed_str())].into_boxed_slice());
        Box::leak(Box::new(ModelEntry {
            name: "test-aux-cap-model",
            version: "1",
            kind: crate::embedding::registry::ModelKind::Embedder,
            source_url: Box::leak(primary_url.into_boxed_str()),
            sha256: Box::leak(primary_sha.into_boxed_str()),
            size_bytes: primary_size,
            licence: "MIT",
            embedding_dim: Some(384),
            files: &["model.onnx", "tokenizer.json"],
            aux_urls,
        }))
    }

    /// An aux body that exceeds a tiny injected cap must abort the download
    /// with `ModelCorrupt` (naming the file + the MiB cap) and remove the
    /// staged `.partial/` dir — the unverified, unsized sidecar is bounded.
    #[test]
    fn oversized_aux_file_aborts_with_model_corrupt_and_cleans_partial() {
        // 1 MiB cap; the served aux body is 3 MiB → over-cap.
        let _guard = AuxCapOverrideGuard::install(1024 * 1024);

        let primary = b"PRIMARY-ONNX-BYTES";
        let oversized_aux = vec![0xCDu8; 3 * 1024 * 1024];
        // The downloader fetches the primary first, then the aux: serve in
        // that order.
        let base = spawn_sequential_server(vec![http_200(primary), http_200(&oversized_aux)]);
        let entry = two_file_entry(&base, sha256_hex(primary), primary.len() as u64);
        let root = TempDir::new().expect("tempdir");

        let err = download_model(entry, root.path(), None)
            .expect_err("over-cap aux fetch must abort the download");
        match err {
            TomeError::ModelCorrupt { model, detail } => {
                assert_eq!(model, "test-aux-cap-model");
                assert!(
                    detail.contains("tokenizer.json") && detail.contains("aux-file cap"),
                    "over-cap detail should name the file + the cap, got: {detail}"
                );
                assert!(
                    detail.contains("1 MiB"),
                    "over-cap detail should report the (injected) cap in MiB, got: {detail}"
                );
            }
            other => panic!("expected ModelCorrupt on over-cap aux, got {other:?}"),
        }

        // The over-cap abort must leave nothing behind: the staged dir is
        // removed by `download_model`'s error-cleanup closure, and the model
        // never lands in its final directory.
        let partial = root.path().join("test-aux-cap-model.partial");
        assert!(
            !partial.exists(),
            "partial dir leaked after over-cap aux abort: {}",
            partial.display()
        );
        let final_dir = root.path().join("test-aux-cap-model");
        assert!(
            !final_dir.exists(),
            "final dir created despite over-cap aux abort: {}",
            final_dir.display()
        );
    }

    /// An aux body within the cap completes normally — the cap is a ceiling,
    /// not a floor, and the happy path is unaffected.
    #[test]
    fn within_cap_aux_file_completes() {
        let _guard = AuxCapOverrideGuard::install(1024 * 1024);

        let primary = b"PRIMARY-ONNX-BYTES";
        let small_aux = vec![0xEFu8; 4096];
        let base = spawn_sequential_server(vec![http_200(primary), http_200(&small_aux)]);
        let entry = two_file_entry(&base, sha256_hex(primary), primary.len() as u64);
        let root = TempDir::new().expect("tempdir");

        let manifest =
            download_model(entry, root.path(), None).expect("within-cap download should succeed");
        assert_eq!(manifest.name, "test-aux-cap-model");

        let aux_path = root
            .path()
            .join("test-aux-cap-model")
            .join("tokenizer.json");
        let written = std::fs::read(&aux_path).expect("aux file present after success");
        assert_eq!(written, small_aux);
    }

    /// The production cap is the documented, generous-but-bounded 64 MiB —
    /// guards against an accidental edit shrinking it below the largest real
    /// aux file (the bge-reranker tokenizer at ~17 MiB) or removing the bound.
    #[test]
    fn production_aux_cap_is_64_mib() {
        assert_eq!(AUX_FILE_MAX, 64 * 1024 * 1024);
        // Sanity: comfortably above the largest real aux file (~17 MiB).
        // A `const` assertion so the headroom guarantee is checked at
        // compile time (and clippy doesn't flag a const-folded `assert!`).
        const _: () = assert!(AUX_FILE_MAX > 17 * 1024 * 1024);
    }
}
