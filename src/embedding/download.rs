//! Atomic, SIGINT-aware, resumable model artefact downloader.
//!
//! Workflow (FR-020a, research §R5; resume added by #420):
//!
//! 1. Create (or adopt) a sibling `.partial/` directory next to the target
//!    model dir. A leftover `.partial/` from an interrupted run is kept ONLY
//!    when its Tome-owned [`RESUME_MARKER`] pins the same registry identity
//!    this run verifies against; otherwise the stale tree is wiped first.
//! 2. Stream the HTTP body chunk-by-chunk through a `Sha256` while writing
//!    to `<.partial>/<filename>`. When a non-empty partial file already
//!    exists, request `Range: bytes=<len>-` and APPEND on a 206 whose
//!    `Content-Range` start matches the requested offset — the
//!    already-staged prefix is folded into the streaming digest first, so
//!    the final hash covers the whole file exactly as a fresh download
//!    would. A 200 (or any server that ignores the range) truncates and
//!    restarts; a 206 with a missing/unparsable/mismatched `Content-Range`
//!    start is never appended (it would splice bytes at the wrong offset —
//!    undetectable for unchecksummed aux files) and restarts clean with one
//!    fresh un-ranged request (#480); an oversized/corrupt partial restarts
//!    clean. When the marker-verified staged PRIMARY is already complete
//!    (size equals the pin) and passes the pinned SHA-256, the primary
//!    fetch is skipped entirely — an aux-only interruption never re-pays
//!    the primary transfer (#480). After every chunk, peek the global
//!    cancellation flag (FR-053); on cancel, abort and RETAIN the
//!    `.partial/` tree so the next run resumes.
//! 3. On EOF, hex-compare the streaming digest against the registry's
//!    pinned hash. A mismatch on a RESUMED file gets exactly one clean full
//!    restart (the partial prefix was bad); a mismatch on a fresh download
//!    → `ModelChecksumMismatch` (exit 32) and remove `.partial/`, exactly
//!    as before resume existed.
//! 4. Stream every non-primary file (`entry.files[1..]`, fetched from
//!    `entry.aux_urls` positionally — e.g. `tokenizer.json` + the optional
//!    fastembed config files) into the same `.partial/` directory, with the
//!    same Range-resume behaviour. These are not checksum-verified (the
//!    registry only pins the primary's size + sha), consistent with
//!    `verify`'s design, so each aux stream is bounded by [`AUX_FILE_MAX`]
//!    (64 MiB) to deny an unbounded sidecar from a compromised pinned host
//!    (over-cap → [`TomeError::ModelCorrupt`]). Doing this BEFORE the
//!    rename keeps the all-or-nothing landing.
//! 5. `fsync` each file, remove the resume marker, then rename the
//!    `.partial/` directory to its final name. The rename is the atomicity
//!    boundary — readers either see the old directory (or none) or the new
//!    one, never a half-extracted state.
//! 6. Write `manifest.toml` atomically via `tempfile::NamedTempFile`.
//!
//! On failure, the `.partial/` tree is retained ONLY for transport-class
//! interruptions (`Io` / `Interrupted`) that left a non-empty primary
//! partial — the resumable cases. Verification-class failures (checksum
//! mismatch, aux over-cap, placeholder refusal) remove it, because those
//! bytes can never become a valid install.
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

/// Tome-owned marker written inside a `.partial/` staging directory,
/// recording the registry identity (`<version> <sha256>`) the staged bytes
/// were fetched against.
///
/// Resume safety hinges on this: aux files are NOT checksum-verified, so
/// appending to a partial staged under a DIFFERENT registry pin (e.g. after
/// a Tome upgrade re-pinned the model) would silently splice two artefact
/// versions into one corrupt sidecar. A missing or mismatched marker
/// therefore wipes the whole staging tree before anything streams. The
/// marker is removed just before the atomic rename so it never lands in the
/// final model directory.
const RESUME_MARKER: &str = ".tome-resume";

/// Ceiling for reading an existing [`RESUME_MARKER`] back. The file is
/// Tome-written and tiny; anything larger is not ours and reads as stale.
const RESUME_MARKER_MAX: u64 = 512;

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

    prepare_partial_dir(entry, &partial_dir)?;

    // Run the full pipeline inside a single closure so a failure at any
    // step (stream, verify, rename, manifest write) is followed by the
    // retain-or-cleanup decision below. Without this, a checksum mismatch —
    // which is detected *after* `stream_to_partial` returns Ok — would leak
    // the .partial dir.
    let pipeline = || -> Result<ModelManifest, TomeError> {
        let primary_dest = partial_dir.join(primary_filename);

        // #480: an aux-only interruption retains a COMPLETE primary in the
        // marker-verified staging tree. When the staged primary's size
        // already equals the pinned `size_bytes`, hash it from disk and run
        // the SAME pinned-SHA-256 gate a fresh stream would — on a match the
        // primary fetch is skipped entirely (an interrupted ~17 MB tokenizer
        // no longer costs a ~280 MB reranker re-download). Verification is
        // never skipped, only the network transfer. On a mismatch fall
        // through to the normal streaming path, which refuses a ranged
        // resume for an at-bound partial and restarts clean (the pre-#480
        // behaviour).
        let primary_already_verified = match std::fs::metadata(&primary_dest) {
            Ok(meta) if entry.size_bytes > 0 && meta.len() == entry.size_bytes => {
                verify_checksum(entry, &sha256_file(&primary_dest)?).is_ok()
            }
            _ => false,
        };
        if primary_already_verified {
            tracing::debug!(
                model = entry.name,
                "staged primary is complete and passes the pinned checksum; \
                 skipping the primary fetch",
            );
            // Report the complete primary once so a byte bar shows 100%
            // instead of sitting at 0 while the aux files stream.
            if let Some(cb) = byte_progress {
                cb(entry.size_bytes, entry.size_bytes);
            }
        } else {
            let (observed_hash, resumed) = stream_to_partial(entry, &primary_dest, byte_progress)?;
            if let Err(err) = verify_checksum(entry, &observed_hash) {
                if !resumed {
                    // Verification semantics are byte-identical to pre-resume
                    // for a fresh download: mismatch errors immediately (the
                    // outer arm removes the staging tree).
                    return Err(err);
                }
                // A resumed file failing its pinned SHA-256 means the staged
                // prefix was bad (torn write, disk corruption): fall back to
                // exactly ONE clean full restart, then error as a fresh
                // download would if it fails again (#420).
                tracing::warn!(
                    model = entry.name,
                    "resumed download failed checksum verification; restarting clean",
                );
                std::fs::remove_file(&primary_dest).map_err(TomeError::Io)?;
                let (fresh_hash, _) = stream_to_partial(entry, &primary_dest, byte_progress)?;
                verify_checksum(entry, &fresh_hash)?;
            }
        }

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
            // Resuming an aux partial is safe ONLY because
            // `prepare_partial_dir` wiped any tree staged under a different
            // registry pin. Scrubbing is preserved — `stream_url_to_partial`
            // runs the URL + reqwest error chain through the credential
            // scrubber exactly as the primary fetch (resumed requests
            // included; they share the one request/error path).
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

        // The resume marker is Tome bookkeeping for the staging dir only —
        // drop it before the rename so it never lands in the model dir.
        std::fs::remove_file(partial_dir.join(RESUME_MARKER)).map_err(TomeError::Io)?;

        if final_dir.exists() {
            std::fs::remove_dir_all(&final_dir).map_err(TomeError::Io)?;
        }
        std::fs::rename(&partial_dir, &final_dir).map_err(TomeError::Io)?;
        write_manifest(entry, &final_dir)
    };

    match pipeline() {
        Ok(manifest) => Ok(manifest),
        Err(err) => {
            if should_retain_partial(&err, &partial_dir.join(primary_filename)) {
                // Transport-class interruption (SIGINT / dropped connection)
                // with real bytes staged: keep the tree so the next run
                // resumes from the offset instead of paying the full
                // download again (#420).
            } else {
                // Best effort: the partial dir may already have been renamed
                // (e.g. if `write_manifest` failed) — in that case the remove
                // is a no-op and the user is left with a renamed dir + missing
                // manifest, which `tome status` flags as Corrupt on next open.
                let _ = std::fs::remove_dir_all(&partial_dir);
            }
            Err(err)
        }
    }
}

/// Prepare the `.partial/` staging directory for a (possibly resumed) run:
/// wipe a leftover tree whose [`RESUME_MARKER`] is absent or pins a different
/// registry identity, then (re)write the marker for this run. See the marker
/// docs for why a cross-pin resume must never happen.
fn prepare_partial_dir(entry: &ModelEntry, partial_dir: &Path) -> Result<(), TomeError> {
    let marker = partial_dir.join(RESUME_MARKER);
    if partial_dir.exists() {
        let stale = match std::fs::metadata(&marker) {
            Ok(meta) if meta.len() <= RESUME_MARKER_MAX => std::fs::read_to_string(&marker)
                .map(|contents| contents != resume_marker_contents(entry))
                .unwrap_or(true),
            // Oversized (not ours) or unreadable/absent marker → stale.
            _ => true,
        };
        if stale {
            std::fs::remove_dir_all(partial_dir).map_err(TomeError::Io)?;
        }
    }
    std::fs::create_dir_all(partial_dir).map_err(TomeError::Io)?;
    std::fs::write(&marker, resume_marker_contents(entry)).map_err(TomeError::Io)?;
    Ok(())
}

/// The registry identity a staged partial must match to be resumable.
fn resume_marker_contents(entry: &ModelEntry) -> String {
    format!("{} {}\n", entry.version, entry.sha256)
}

/// Whether a failed pipeline should RETAIN the `.partial/` staging tree for
/// a later resume. True only for transport-class interruptions (`Io` — a
/// dropped connection / HTTP error mid-run — or SIGINT's `Interrupted`)
/// that left a non-empty primary partial behind. Verification-class
/// failures (`ModelChecksumMismatch`, `ModelCorrupt`) mean the staged bytes
/// can never verify, so retaining them would only re-fail the next run.
fn should_retain_partial(err: &TomeError, primary_partial: &Path) -> bool {
    matches!(err, TomeError::Io(_) | TomeError::Interrupted)
        && std::fs::metadata(primary_partial)
            .map(|m| m.len() > 0)
            .unwrap_or(false)
}

/// Stream `entry.source_url` (the primary artefact) into `dest`, returning the
/// streaming SHA-256 for `verify_checksum` plus whether the transfer RESUMED
/// an existing partial (a 206 append). Thin wrapper over
/// [`stream_url_to_partial`] that supplies the primary's pinned size for the
/// progress bar.
fn stream_to_partial(
    entry: &ModelEntry,
    dest: &Path,
    byte_progress: Option<&dyn Fn(u64, u64)>,
) -> Result<(String, bool), TomeError> {
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
/// streaming digest plus whether the transfer RESUMED (appended to) an
/// existing partial via a 206. Used for both the primary artefact (size
/// known, progress driven, no `aux_cap`) and the non-primary aux files (size
/// unknown, `None` progress, byte-capped via `aux_cap`).
///
/// Resume protocol (#420): when `dest` already holds a non-empty byte prefix
/// that is still under its bound (the pinned size for the primary, the byte
/// cap for an aux file), the request carries `Range: bytes=<len>-`.
/// A 206 whose `Content-Range` start equals the requested offset appends to
/// the prefix — the prefix is hashed from disk first, so the returned digest
/// covers the whole file exactly as a fresh stream would. A 200 (or any
/// other success status — the server ignored or never honoured the range)
/// truncates and restarts. A 206 with a missing, unparsable, or mismatched
/// `Content-Range` start is REFUSED (#480 — appending a wrong-offset tail
/// would go undetected on an unchecksummed aux file) and restarts clean via
/// one fresh un-ranged request. A 416 (the offset is past the server's
/// actual resource end, i.e. the partial is oversized relative to reality)
/// also truncates and re-requests once without a range. An
/// oversized-on-disk partial never sends a range at all.
///
/// `total_for_progress` is the byte count reported to `byte_progress` as the
/// second argument; the network's `Content-Length` is intentionally NOT
/// consulted (it can disagree with the registry pin for redirected URLs and
/// the pin is authoritative). When `None`, `byte_progress` is not invoked.
/// On a resume the callback starts at the prefix offset, so a byte bar picks
/// up where the interrupted run left off.
///
/// `aux_cap`, when `Some`, bounds the streamed body: once cumulative bytes
/// written (prefix included) would exceed `aux_cap.max_bytes`, the stream
/// aborts with [`TomeError::ModelCorrupt`] (the partially written file is
/// left in the `.partial/` dir for `download_model`'s error arm to remove —
/// over-cap is a verification-class failure, never retained).
/// The primary path passes `None` because its `size_bytes` pin + SHA-256
/// verify already bound and authenticate the body; the `Content-Length`
/// remains untrusted on both paths.
///
/// CREDENTIAL SCRUBBING: `reqwest::Error::Display` and the status-line message
/// reproduce the failing URL verbatim, which can include presigned-URL query
/// parameters carrying credentials. Both are run through the credential
/// scrubber before reaching `TomeError` — this MUST hold for aux fetches AND
/// resumed (ranged) requests too; all of them route through the single
/// [`send_request`] / status-error path below.
fn stream_url_to_partial(
    url: &str,
    dest: &Path,
    total_for_progress: Option<u64>,
    byte_progress: Option<&dyn Fn(u64, u64)>,
    aux_cap: Option<AuxCap<'_>>,
) -> Result<(String, bool), TomeError> {
    // Resume offset: the staged prefix length, unless the file is absent,
    // empty, or already at/over its bound — an oversized partial cannot be
    // completed by appending, so it restarts clean exactly like a corrupted
    // one (the marker check in `prepare_partial_dir` already wiped
    // cross-pin leftovers).
    let mut resume_from = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
    let bound = match (&aux_cap, total_for_progress) {
        (Some(cap), _) => Some(cap.max_bytes),
        (None, Some(total)) => Some(total),
        (None, None) => None,
    };
    if let Some(bound) = bound
        && resume_from >= bound
    {
        resume_from = 0;
    }

    let mut response = send_request(url, resume_from)?;
    if resume_from > 0 && response.status().as_u16() == 416 {
        // Range-not-satisfiable: our offset is past the end of the actual
        // resource (the pin and reality disagree — verification will decide
        // later). Restart clean with one un-ranged request.
        resume_from = 0;
        response = send_request(url, 0)?;
    }

    // Cross-check a 206's `Content-Range` start against the offset we asked
    // for (#480). A missing, unparsable, or mismatched start means a
    // misbehaving server/proxy is sending a tail from the WRONG offset —
    // appending it would splice bytes at the wrong position. The primary's
    // pinned SHA-256 would catch that after the fact, but aux files are
    // unchecksummed, so the splice must be refused up front: discard the
    // response and restart clean with one fresh un-ranged request, exactly
    // like the server-ignores-Range 200 path. The fresh request routes back
    // through `send_request`, so credential scrubbing on its error path is
    // unchanged.
    if resume_from > 0
        && response.status().as_u16() == 206
        && content_range_start(&response) != Some(resume_from)
    {
        tracing::warn!(
            requested_offset = resume_from,
            "206 Content-Range start does not match the requested resume offset; \
             refusing to append and restarting clean",
        );
        resume_from = 0;
        response = send_request(url, 0)?;
    }

    let status = response.status();
    if !status.is_success() {
        return Err(TomeError::Io(std::io::Error::other(scrub_for_diag(
            &format!("HTTP {} fetching {}", status, url),
        ))));
    }
    // Only an explicit 206 means the server honoured the range and is
    // sending the tail; any other success (a plain 200, a proxy that
    // stripped the header) is the FULL body → truncate-and-restart.
    let resumed = resume_from > 0 && status.as_u16() == 206;

    let mut hasher: Sha256;
    let mut file: File;
    let mut written: u64;
    if resumed {
        // Fold the already-staged prefix into the streaming digest so the
        // final SHA-256 covers prefix + appended tail — verification
        // semantics stay byte-identical to a fresh download.
        hasher = hash_file_prefix(dest)?;
        file = std::fs::OpenOptions::new()
            .append(true)
            .open(dest)
            .map_err(TomeError::Io)?;
        written = resume_from;
        if let (Some(cb), Some(total)) = (byte_progress, total_for_progress) {
            cb(written, total);
        }
    } else {
        hasher = Sha256::new();
        file = File::create(dest).map_err(TomeError::Io)?;
        written = 0;
    }
    let mut buf = vec![0u8; STREAM_CHUNK_SIZE];

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
        // fixed-size `buf` regardless). `written` includes any resumed
        // prefix, so the cap bounds the TOTAL file, not just this stream.
        // The over-cap error names the model + file and reuses the existing
        // `ModelCorrupt` variant (exit 31) — no new error variant / exit code.
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
    Ok((hex::encode(hasher.finalize()), resumed))
}

/// Issue one GET for `url`, carrying `Range: bytes=<offset>-` when `offset`
/// is non-zero. The single request chokepoint for fresh, resumed, primary
/// and aux fetches alike — so the credential-scrubbed error mapping cannot
/// diverge between them.
fn send_request(url: &str, offset: u64) -> Result<reqwest::blocking::Response, TomeError> {
    let client = reqwest::blocking::Client::builder().build().map_err(|e| {
        TomeError::Io(std::io::Error::other(scrub_for_diag(&format!(
            "HTTP client init failed: {e}"
        ))))
    })?;
    let mut request = client.get(url);
    if offset > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={offset}-"));
    }
    request.send().map_err(|e| {
        TomeError::Io(std::io::Error::other(scrub_for_diag(&format!(
            "HTTP get failed: {e}"
        ))))
    })
}

/// The start offset a 206's `Content-Range: bytes <start>-<end>/<total>`
/// header advertises, when present and parseable. `None` (absent header,
/// non-ASCII value, or a shape [`parse_content_range_start`] rejects) reads
/// as "the offset cannot be trusted" at the resume check above.
fn content_range_start(response: &reqwest::blocking::Response) -> Option<u64> {
    let value = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)?
        .to_str()
        .ok()?;
    parse_content_range_start(value)
}

/// Parse the `<start>` from a `bytes <start>-<end>/<total>` header value
/// (RFC 9110 §14.4). Anything malformed — including the `bytes */<total>`
/// unsatisfied-range form, which carries no start — returns `None`.
fn parse_content_range_start(value: &str) -> Option<u64> {
    let rest = value.trim().strip_prefix("bytes")?.trim_start();
    let (start, _) = rest.split_once('-')?;
    start.trim().parse().ok()
}

/// Prime a `Sha256` with the existing on-disk bytes of `dest` (the staged
/// prefix a 206 resume appends after). Chunked so a several-hundred-MB
/// partial stays bounded in memory; observes SIGINT like the stream loop.
fn hash_file_prefix(path: &Path) -> Result<Sha256, TomeError> {
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
    Ok(hasher)
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
    /// request). Returns the bound base URL plus a recorder capturing each
    /// raw request head, so tests can assert on the presence/absence of the
    /// `Range` header. Mirrors the hand-rolled server in
    /// `tests/model_download.rs` but serves more than one request so we can
    /// satisfy the primary fetch + the aux fetch.
    fn spawn_sequential_server(
        responses: Vec<Vec<u8>>,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1:0");
        let addr = listener.local_addr().expect("local_addr");
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorder = std::sync::Arc::clone(&requests);
        thread::spawn(move || {
            for response in responses {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut sink = [0u8; 4096];
                        let n = stream.read(&mut sink).unwrap_or(0);
                        recorder
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push(String::from_utf8_lossy(&sink[..n]).into_owned());
                        let _ = stream.write_all(&response);
                        let _ = stream.flush();
                    }
                    Err(_) => break,
                }
            }
        });
        (format!("http://{addr}"), requests)
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
        let (base, _requests) =
            spawn_sequential_server(vec![http_200(primary), http_200(&oversized_aux)]);
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
        let (base, _requests) =
            spawn_sequential_server(vec![http_200(primary), http_200(&small_aux)]);
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
        // The staging bookkeeping must not leak into the landed model dir.
        assert!(
            !root
                .path()
                .join("test-aux-cap-model")
                .join(RESUME_MARKER)
                .exists(),
            "resume marker must be removed before the atomic rename",
        );
    }

    /// Single-file entry helper for the resume unit tests below (aux-less so
    /// one response per run suffices).
    fn one_file_entry(base: &str, sha256: String, size: u64) -> &'static ModelEntry {
        let url = format!("{base}/model.onnx");
        Box::leak(Box::new(ModelEntry {
            name: "test-resume-model",
            version: "1",
            kind: crate::embedding::registry::ModelKind::Embedder,
            source_url: Box::leak(url.into_boxed_str()),
            sha256: Box::leak(sha256.into_boxed_str()),
            size_bytes: size,
            licence: "MIT",
            embedding_dim: Some(384),
            files: &["model.onnx"],
            aux_urls: &[],
        }))
    }

    /// An oversized partial (at/over the pinned size) cannot be completed by
    /// appending: the downloader must restart clean — sending NO `Range`
    /// header — and land the verified fresh bytes.
    #[test]
    fn oversized_partial_restarts_clean_without_range() {
        let payload = b"FRESH-CORRECT-PAYLOAD";
        let (base, requests) = spawn_sequential_server(vec![http_200(payload)]);
        let entry = one_file_entry(&base, sha256_hex(payload), payload.len() as u64);
        let root = TempDir::new().expect("tempdir");

        // Craft a valid-marker partial whose primary is OVERSIZED (>= the
        // pinned size). Only reachable through crafted state, hence a unit
        // test with private access to the marker format.
        let partial_dir = root.path().join("test-resume-model.partial");
        std::fs::create_dir_all(&partial_dir).unwrap();
        std::fs::write(
            partial_dir.join(RESUME_MARKER),
            resume_marker_contents(entry),
        )
        .unwrap();
        std::fs::write(
            partial_dir.join("model.onnx"),
            vec![0xAA; payload.len() + 100],
        )
        .unwrap();

        download_model(entry, root.path(), None).expect("oversized partial must restart clean");
        let reqs = requests.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(reqs.len(), 1);
        assert!(
            !reqs[0].contains("range:") && !reqs[0].contains("Range:"),
            "oversized partial must not attempt a ranged resume; request was:\n{}",
            reqs[0],
        );
        let landed = std::fs::read(root.path().join("test-resume-model/model.onnx")).unwrap();
        assert_eq!(landed, payload);
    }

    /// A leftover partial with NO resume marker (or a marker from a different
    /// registry pin) is untrusted state: the whole staging tree is wiped and
    /// the download restarts from byte 0.
    #[test]
    fn markerless_partial_is_wiped_and_restarted() {
        let payload = b"FRESH-CORRECT-PAYLOAD";
        let (base, requests) = spawn_sequential_server(vec![http_200(payload)]);
        let entry = one_file_entry(&base, sha256_hex(payload), payload.len() as u64);
        let root = TempDir::new().expect("tempdir");

        // A plausible-length prefix, but no marker recording which pin it
        // was staged under — must NOT be resumed (an aux append under a
        // different pin would splice two artefact versions).
        let partial_dir = root.path().join("test-resume-model.partial");
        std::fs::create_dir_all(&partial_dir).unwrap();
        std::fs::write(partial_dir.join("model.onnx"), &payload[..4]).unwrap();

        download_model(entry, root.path(), None).expect("markerless partial must restart clean");
        let reqs = requests.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(reqs.len(), 1);
        assert!(
            !reqs[0].contains("range:") && !reqs[0].contains("Range:"),
            "markerless partial must not attempt a ranged resume; request was:\n{}",
            reqs[0],
        );
        let landed = std::fs::read(root.path().join("test-resume-model/model.onnx")).unwrap();
        assert_eq!(landed, payload);
    }

    /// A marker pinned to a DIFFERENT registry identity (version/sha changed
    /// between runs, e.g. a Tome upgrade re-pinned the model) also wipes the
    /// staging tree rather than resuming.
    #[test]
    fn cross_pin_partial_is_wiped_and_restarted() {
        let payload = b"FRESH-CORRECT-PAYLOAD";
        let (base, requests) = spawn_sequential_server(vec![http_200(payload)]);
        let entry = one_file_entry(&base, sha256_hex(payload), payload.len() as u64);
        let root = TempDir::new().expect("tempdir");

        let partial_dir = root.path().join("test-resume-model.partial");
        std::fs::create_dir_all(&partial_dir).unwrap();
        std::fs::write(
            partial_dir.join(RESUME_MARKER),
            "0 someotherpinnedsha256value\n",
        )
        .unwrap();
        std::fs::write(partial_dir.join("model.onnx"), &payload[..4]).unwrap();

        download_model(entry, root.path(), None).expect("cross-pin partial must restart clean");
        let reqs = requests.lock().unwrap_or_else(|e| e.into_inner());
        assert!(
            !reqs[0].contains("range:") && !reqs[0].contains("Range:"),
            "cross-pin partial must not attempt a ranged resume; request was:\n{}",
            reqs[0],
        );
        let landed = std::fs::read(root.path().join("test-resume-model/model.onnx")).unwrap();
        assert_eq!(landed, payload);
    }

    /// `Content-Range` start parsing (#480): valid `bytes <start>-<end>/<total>`
    /// values yield the start; anything malformed — including the
    /// `bytes */<total>` unsatisfied-range form, which carries no start —
    /// yields `None`, which the resume check treats as "cannot trust the
    /// offset" (clean restart).
    #[test]
    fn content_range_start_parses_valid_and_rejects_malformed() {
        assert_eq!(
            parse_content_range_start("bytes 1024-2047/4096"),
            Some(1024)
        );
        assert_eq!(parse_content_range_start(" bytes  0-99/100 "), Some(0));
        assert_eq!(parse_content_range_start("bytes 512-1023/*"), Some(512));
        // No start offset to trust in any of these:
        assert_eq!(parse_content_range_start("bytes */4096"), None);
        assert_eq!(parse_content_range_start("bytes garbage"), None);
        assert_eq!(parse_content_range_start(""), None);
        assert_eq!(parse_content_range_start("items 1024-2047/4096"), None);
        assert_eq!(parse_content_range_start("bytes -5-99/100"), None);
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
