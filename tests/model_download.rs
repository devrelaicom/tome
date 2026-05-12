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
//! Interrupt safety (FR-053) is not exercised here: the cancellation flag is
//! global process state and the download body executes inside a single
//! `reqwest::blocking::get` call. The design is documented in
//! `src/embedding/download.rs`; a higher-level integration test once
//! `tome models download` is wired covers the end-to-end SIGINT flow.

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

    let manifest = download_model(entry, root.path()).expect("download should succeed");

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

    let err = download_model(entry, root.path()).expect_err("hash mismatch must abort");
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

    let err = download_model(entry, root.path()).expect_err("404 must abort");
    assert!(matches!(err, TomeError::Io(_)), "expected Io, got {err:?}");

    let partial = root.path().join("test-model.partial");
    assert!(!partial.exists(), "partial dir leaked after HTTP 404");
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

    let err = download_model(entry, root.path()).expect_err("placeholder must refuse");
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
