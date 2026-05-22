//! Integration tests for the atomic model downloader (T057).
//!
//! Each test spins up a single-shot `TcpListener` on `127.0.0.1:0`,
//! constructs a synthetic `ModelEntry` pointing at that server, and exercises
//! [`download_model`]. The local HTTP server is hand-rolled (~40 lines) to
//! avoid pulling in `httptest` / `wiremock` as dev-dependencies; for these
//! tests we only need to serve one request with a fixed payload or a
//! controlled error status.
//!
//! Scenarios covered:
//!
//! 1. **Success** — the payload SHA-256 matches the registry hash; the file
//!    lands in the final directory and the manifest is written.
//! 2. **Checksum mismatch** — the registry advertises a different hash than
//!    the bytes; download fails with `ModelChecksumMismatch` and the
//!    `.partial/` directory is removed.
//! 3. **HTTP 404** — the server returns a non-2xx status; download fails
//!    with `Io` and the `.partial/` directory is removed.
//!
//! Interrupt safety (FR-053) is exercised indirectly by the
//! `mid_stream_connection_drop_*` test added in Phase 10 / T196: the
//! cleanup path that fires on a `reqwest` connection-drop is the same
//! path that fires on a SIGINT-triggered `TomeError::Interrupted` —
//! both propagate out of `stream_to_partial` and trigger
//! `download_model`'s pipeline-error cleanup closure. SIGINT itself
//! flips the global `crate::catalog::git::CANCELLED` static; the test
//! discipline keeps that static unmanipulated (P3 retro), so we
//! validate the cleanup path via the equivalent failure mode.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use sha2::{Digest, Sha256};
use tempfile::TempDir;

use tome::embedding::download::download_model;
use tome::embedding::registry::{ModelEntry, ModelKind};
use tome::error::TomeError;

/// One-shot HTTP server: serves a single request, writes `response`, exits.
/// Returns the bound `127.0.0.1:<port>` URL the test should hand to
/// `download_model`.
fn spawn_one_shot_server(response: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1:0");
    let addr = listener.local_addr().expect("local_addr");
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            // Read the request headers until we see the blank line. We don't
            // parse them; just drain enough that the client's send buffer can
            // flush. 4 KB is plenty for a typical `GET` line plus headers.
            let mut sink = [0u8; 4096];
            let _ = stream.read(&mut sink);
            let _ = stream.write_all(&response);
            let _ = stream.flush();
        }
    });
    format!("http://{addr}/model.onnx")
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

fn http_404() -> Vec<u8> {
    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Build a `ModelEntry` pointing at `url`. Strings live for the duration of
/// the test process via `Box::leak` because `ModelEntry`'s fields are
/// `&'static str` — that's fine for a test fixture: the leaks are bounded
/// by the test binary's lifetime and `cargo test` runs each binary fresh.
fn entry_for(url: String, sha256: String, size: u64) -> &'static ModelEntry {
    Box::leak(Box::new(ModelEntry {
        name: "test-model",
        version: "1",
        kind: ModelKind::Embedder,
        source_url: Box::leak(url.into_boxed_str()),
        sha256: Box::leak(sha256.into_boxed_str()),
        size_bytes: size,
        licence: "MIT",
        files: &["model.onnx"],
    }))
}

#[test]
fn happy_path_writes_file_and_manifest() {
    let payload = b"ONNX-FAKE-PAYLOAD-12345";
    let url = spawn_one_shot_server(http_200(payload));
    let entry = entry_for(url, sha256_hex(payload), payload.len() as u64);
    let root = TempDir::new().expect("tempdir");

    let manifest = download_model(entry, root.path(), None).expect("download should succeed");

    assert_eq!(manifest.name, "test-model");
    assert_eq!(manifest.sha256, entry.sha256);

    let final_file = root.path().join("test-model").join("model.onnx");
    let written = std::fs::read(&final_file).expect("final file present");
    assert_eq!(written, payload);

    let manifest_path = root.path().join("test-model").join("manifest.json");
    assert!(manifest_path.exists(), "manifest.json was not persisted");

    // The .partial directory must be gone after a successful download.
    let partial = root.path().join("test-model.partial");
    assert!(
        !partial.exists(),
        "partial dir leaked: {}",
        partial.display()
    );
}

#[test]
fn checksum_mismatch_aborts_and_cleans_partial_dir() {
    let payload = b"served-bytes";
    let url = spawn_one_shot_server(http_200(payload));
    let wrong_hash = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_owned();
    let entry = entry_for(url, wrong_hash, payload.len() as u64);
    let root = TempDir::new().expect("tempdir");

    let err = download_model(entry, root.path(), None).expect_err("hash mismatch must abort");
    match err {
        TomeError::ModelChecksumMismatch { model, .. } => {
            assert_eq!(model, "test-model");
        }
        other => panic!("expected ModelChecksumMismatch, got {other:?}"),
    }

    let partial = root.path().join("test-model.partial");
    assert!(
        !partial.exists(),
        "partial dir leaked after checksum mismatch: {}",
        partial.display()
    );
    let final_dir = root.path().join("test-model");
    assert!(
        !final_dir.exists(),
        "final dir created despite checksum mismatch: {}",
        final_dir.display()
    );
}

#[test]
fn http_error_status_aborts_and_cleans_partial_dir() {
    let url = spawn_one_shot_server(http_404());
    // The hash doesn't matter — we should fail before reaching the verify
    // step — but it must be non-placeholder to clear the registry guard.
    let entry = entry_for(
        url,
        "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789".to_owned(),
        0,
    );
    let root = TempDir::new().expect("tempdir");

    let err = download_model(entry, root.path(), None).expect_err("404 must abort");
    assert!(matches!(err, TomeError::Io(_)), "expected Io, got {err:?}");

    let partial = root.path().join("test-model.partial");
    assert!(!partial.exists(), "partial dir leaked after HTTP 404");
}

/// Slow-trickle server: writes the HTTP 200 header + Content-Length larger
/// than the actual body, sends a single chunk, then closes the socket
/// without sending the rest. The client should fail mid-stream — either
/// with a `reqwest` read error or with the `Content-Length` mismatch
/// surfacing as an EOF. Either way the partial dir must be cleaned up.
fn spawn_trickle_then_drop_server(prefix: Vec<u8>, claimed_total: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1:0");
    let addr = listener.local_addr().expect("local_addr");
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut sink = [0u8; 4096];
            let _ = stream.read(&mut sink);
            // Header advertises a total length that we will not honour: the
            // client expects `claimed_total` bytes but we only deliver
            // `prefix.len()` before dropping the connection.
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {claimed_total}\r\nConnection: close\r\n\r\n"
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(&prefix);
            let _ = stream.flush();
            // Drop the stream WITHOUT writing the rest. The client will see
            // an EOF / connection reset partway through the body.
        }
    });
    format!("http://{addr}/model.onnx")
}

#[test]
fn mid_stream_connection_drop_aborts_and_cleans_partial_dir() {
    // Phase 10 / T196 — covers FR-020a / FR-053 partial-dir cleanup on a
    // mid-stream failure. SIGINT cancellation runs through the same
    // pipeline-error path (the `was_cancelled()` check at the top of
    // each read loop returns `TomeError::Interrupted`, which propagates
    // out of `stream_to_partial` and triggers the cleanup closure
    // exactly as a `reqwest` connection-drop error does). The two
    // cleanup paths are unified at `download_model`'s `pipeline`
    // closure, so a mid-stream connection drop is a faithful proxy for
    // the SIGINT case — and it doesn't need to flip the global
    // `CANCELLED` static (which the test discipline keeps unmanipulated;
    // see P3 retro).
    let prefix = vec![0xAB; 1024]; // 1 KB of bytes sent…
    let url = spawn_trickle_then_drop_server(prefix, 8 * 1024); // …of 8 KB advertised.
    let entry = entry_for(
        url,
        "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789".to_owned(),
        8 * 1024,
    );
    let root = TempDir::new().expect("tempdir");

    let err = download_model(entry, root.path(), None).expect_err("mid-stream drop must abort");
    // The most likely error is `Io` (reqwest's connection-reset translation)
    // but a `ModelChecksumMismatch` is also acceptable IF the server's
    // short body happens to pass the read loop before EOF: in either case
    // the cleanup invariant is the only load-bearing assertion.
    assert!(
        matches!(
            err,
            TomeError::Io(_) | TomeError::ModelChecksumMismatch { .. }
        ),
        "expected Io or ModelChecksumMismatch on mid-stream abort, got {err:?}",
    );

    let partial = root.path().join("test-model.partial");
    assert!(
        !partial.exists(),
        "partial dir leaked after mid-stream connection drop: {}",
        partial.display(),
    );
    let final_dir = root.path().join("test-model");
    assert!(
        !final_dir.exists(),
        "final dir created despite mid-stream abort: {}",
        final_dir.display(),
    );
}

#[test]
fn placeholder_checksum_is_refused() {
    // Synthetic entry with the all-zero placeholder sha256 — must error
    // before any HTTP request happens. The URL therefore can be bogus.
    let entry = entry_for(
        "http://127.0.0.1:1/never-reached".to_owned(),
        "0".repeat(64),
        0,
    );
    let root = TempDir::new().expect("tempdir");

    let err = download_model(entry, root.path(), None).expect_err("placeholder must refuse");
    match err {
        TomeError::ModelCorrupt { model, detail } => {
            assert_eq!(model, "test-model");
            assert!(
                detail.contains("placeholder"),
                "detail missing context: {detail}"
            );
        }
        other => panic!("expected ModelCorrupt, got {other:?}"),
    }
}
