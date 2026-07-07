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
        embedding_dim: Some(384),
        files: &["model.onnx"],
        // Single-file fixture: no non-primary files, so no aux fetch.
        aux_urls: &[],
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

    let manifest_path = root.path().join("test-model").join("manifest.toml");
    assert!(manifest_path.exists(), "manifest.toml was not persisted");

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
fn mid_stream_connection_drop_retains_partial_for_resume() {
    // Phase 10 / T196 covered FR-020a / FR-053 partial-dir CLEANUP on a
    // mid-stream failure; #420 (resumable downloads) deliberately inverts
    // that for transport-class interruptions: the staged prefix is now
    // RETAINED so the next run can resume with `Range: bytes=<len>-`
    // instead of paying the full download again. SIGINT cancellation runs
    // through the same pipeline-error path (`was_cancelled()` →
    // `TomeError::Interrupted`), so a mid-stream connection drop remains a
    // faithful proxy for the SIGINT case without flipping the global
    // `CANCELLED` static (test discipline, P3 retro). Verification-class
    // failures (checksum mismatch, over-cap aux) still clean up — see
    // `checksum_mismatch_aborts_and_cleans_partial_dir`.
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
    // short body happens to pass the read loop before EOF (a mismatch
    // wipes the staging tree; an Io retains it — both are valid ends here,
    // the load-bearing assertion is that the final dir never appears).
    assert!(
        matches!(
            err,
            TomeError::Io(_) | TomeError::ModelChecksumMismatch { .. }
        ),
        "expected Io or ModelChecksumMismatch on mid-stream abort, got {err:?}",
    );

    if matches!(err, TomeError::Io(_)) {
        let partial_file = root.path().join("test-model.partial").join("model.onnx");
        assert!(
            partial_file.is_file()
                && std::fs::metadata(&partial_file)
                    .map(|m| m.len())
                    .unwrap_or(0)
                    > 0,
            "transport-class abort must retain a non-empty partial for resume: {}",
            partial_file.display(),
        );
    }
    let final_dir = root.path().join("test-model");
    assert!(
        !final_dir.exists(),
        "final dir created despite mid-stream abort: {}",
        final_dir.display(),
    );
}

// ---- Resumable downloads (#420) --------------------------------------------
//
// Each scenario drives the REAL two-run flow: run 1 is interrupted
// mid-stream by a trickle-then-drop response (leaving a retained partial),
// run 2 hits the SAME URL again and the scripted server decides whether to
// honour the `Range` header. The recorder captures every raw request head so
// the tests can assert exactly what the client sent.

/// One scripted response per accepted connection.
enum Script {
    /// Send a 200 header claiming `claimed_total` bytes, deliver only
    /// `prefix`, then drop the connection (the run-1 interruption).
    DropAfter {
        prefix: Vec<u8>,
        claimed_total: usize,
    },
    /// Honour `Range: bytes=N-` with a 206 slice of `full` (+
    /// `Content-Range`); reply 200 with the whole body when no range came.
    HonourRange { full: Vec<u8> },
    /// Ignore any `Range` header and reply 200 with the whole body.
    IgnoreRange { full: Vec<u8> },
    /// Reply 206 to a ranged request but with a `Content-Range` START that
    /// does NOT match the requested offset (a misbehaving server/proxy
    /// restarting from 0 while still claiming partial content), serving the
    /// FULL body as the "tail". A correct client must refuse to append.
    /// Replies 200 with the whole body when no range came.
    MismatchedContentRange { full: Vec<u8> },
    /// Reply 206 to a ranged request WITHOUT any `Content-Range` header
    /// (e.g. a proxy stripped it), serving the correct tail. The offset is
    /// unverifiable, so a correct client must refuse to append. Replies 200
    /// with the whole body when no range came.
    MissingContentRange { full: Vec<u8> },
}

/// Scripted sequential server: one `Script` per accepted connection, in
/// order. Returns the BASE URL (callers append the file path — the server
/// itself dispatches purely on accept order) plus a recorder of raw request
/// heads, so tests can assert on both the `Range` header and the request
/// path (`GET /model.onnx` vs `GET /tokenizer.json`).
fn spawn_scripted_server(
    scripts: Vec<Script>,
) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1:0");
    let addr = listener.local_addr().expect("local_addr");
    let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorder = std::sync::Arc::clone(&requests);
    thread::spawn(move || {
        for script in scripts {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            let mut sink = [0u8; 4096];
            let n = stream.read(&mut sink).unwrap_or(0);
            let head = String::from_utf8_lossy(&sink[..n]).into_owned();
            let range_offset = parse_range_offset(&head);
            recorder
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(head);
            match script {
                Script::DropAfter {
                    prefix,
                    claimed_total,
                } => {
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {claimed_total}\r\nConnection: close\r\n\r\n"
                    );
                    let _ = stream.write_all(header.as_bytes());
                    let _ = stream.write_all(&prefix);
                    let _ = stream.flush();
                    // Drop WITHOUT the rest → the client errors mid-stream.
                }
                Script::HonourRange { full } => {
                    let response = match range_offset {
                        Some(offset) if (offset as usize) < full.len() => {
                            let tail = &full[offset as usize..];
                            let mut out = format!(
                                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                                tail.len(),
                                offset,
                                full.len() - 1,
                                full.len(),
                            )
                            .into_bytes();
                            out.extend_from_slice(tail);
                            out
                        }
                        Some(_) => {
                            b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                                .to_vec()
                        }
                        None => http_200(&full),
                    };
                    let _ = stream.write_all(&response);
                    let _ = stream.flush();
                }
                Script::IgnoreRange { full } => {
                    let _ = stream.write_all(&http_200(&full));
                    let _ = stream.flush();
                }
                Script::MismatchedContentRange { full } => {
                    let response = match range_offset {
                        Some(_) => {
                            // Claim a start of 0 regardless of the requested
                            // offset, and serve the full body — the classic
                            // "proxy restarted from byte 0 but kept the 206".
                            let mut out = format!(
                                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes 0-{}/{}\r\nConnection: close\r\n\r\n",
                                full.len(),
                                full.len() - 1,
                                full.len(),
                            )
                            .into_bytes();
                            out.extend_from_slice(&full);
                            out
                        }
                        None => http_200(&full),
                    };
                    let _ = stream.write_all(&response);
                    let _ = stream.flush();
                }
                Script::MissingContentRange { full } => {
                    let response = match range_offset {
                        Some(offset) if (offset as usize) < full.len() => {
                            // The tail is CORRECT — only the header is gone.
                            // The client cannot verify the offset, so it must
                            // still refuse to append.
                            let tail = &full[offset as usize..];
                            let mut out = format!(
                                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                tail.len(),
                            )
                            .into_bytes();
                            out.extend_from_slice(tail);
                            out
                        }
                        _ => http_200(&full),
                    };
                    let _ = stream.write_all(&response);
                    let _ = stream.flush();
                }
            }
        }
    });
    (format!("http://{addr}"), requests)
}

/// Extract the offset of a `Range: bytes=N-` request header, if present.
fn parse_range_offset(request_head: &str) -> Option<u64> {
    request_head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if !name.trim().eq_ignore_ascii_case("range") {
            return None;
        }
        value
            .trim()
            .strip_prefix("bytes=")?
            .strip_suffix('-')?
            .parse()
            .ok()
    })
}

/// The deterministic full payload the resume scenarios reassemble.
fn resume_payload() -> Vec<u8> {
    (0u32..2048).flat_map(|i| i.to_le_bytes()).collect()
}

/// Killing a download mid-flight and re-running resumes from the partial
/// offset: run 2 sends `Range: bytes=<len>-`, the server's 206 tail is
/// appended, and the reassembled file passes the pinned SHA-256.
#[test]
fn interrupted_download_resumes_from_partial_offset_via_206() {
    let full = resume_payload();
    let (base, requests) = spawn_scripted_server(vec![
        Script::DropAfter {
            prefix: full[..1024].to_vec(),
            claimed_total: full.len(),
        },
        Script::HonourRange { full: full.clone() },
    ]);
    let entry = entry_for(
        format!("{base}/model.onnx"),
        sha256_hex(&full),
        full.len() as u64,
    );
    let root = TempDir::new().expect("tempdir");

    // Run 1: interrupted mid-stream; the partial is retained.
    download_model(entry, root.path(), None).expect_err("run 1 must abort mid-stream");
    let partial_file = root.path().join("test-model.partial").join("model.onnx");
    let staged = std::fs::metadata(&partial_file)
        .map(|m| m.len())
        .expect("run 1 must retain a partial");
    assert!(staged > 0, "retained partial must be non-empty");

    // Run 2: resumes and completes.
    let manifest = download_model(entry, root.path(), None).expect("run 2 must resume + succeed");
    assert_eq!(manifest.name, "test-model");

    let reqs = requests.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(reqs.len(), 2, "exactly two requests expected");
    assert_eq!(
        parse_range_offset(&reqs[1]),
        Some(staged),
        "run 2 must request `Range: bytes=<staged len>-`; request was:\n{}",
        reqs[1],
    );

    let landed = std::fs::read(root.path().join("test-model/model.onnx")).unwrap();
    assert_eq!(
        landed, full,
        "reassembled bytes must equal the full payload"
    );
    assert!(
        !root.path().join("test-model.partial").exists(),
        "partial dir must be gone after a successful resume",
    );
}

/// A server that ignores the range (plain 200 full body) makes run 2
/// truncate-and-restart — and still land a correct, verified file.
#[test]
fn server_ignoring_range_falls_back_to_clean_restart() {
    let full = resume_payload();
    let (base, requests) = spawn_scripted_server(vec![
        Script::DropAfter {
            prefix: full[..1024].to_vec(),
            claimed_total: full.len(),
        },
        Script::IgnoreRange { full: full.clone() },
    ]);
    let entry = entry_for(
        format!("{base}/model.onnx"),
        sha256_hex(&full),
        full.len() as u64,
    );
    let root = TempDir::new().expect("tempdir");

    download_model(entry, root.path(), None).expect_err("run 1 must abort mid-stream");
    download_model(entry, root.path(), None).expect("run 2 must succeed via truncate-and-restart");

    let reqs = requests.lock().unwrap_or_else(|e| e.into_inner());
    assert!(
        parse_range_offset(&reqs[1]).is_some(),
        "run 2 should have ATTEMPTED a ranged resume; request was:\n{}",
        reqs[1],
    );
    let landed = std::fs::read(root.path().join("test-model/model.onnx")).unwrap();
    assert_eq!(
        landed, full,
        "a 200 response must overwrite the partial, not append to it",
    );
}

/// A resumed file that fails its pinned SHA-256 (the staged prefix was bad)
/// falls back to exactly ONE clean full restart — request 3 carries no
/// `Range` — and succeeds when the fresh bytes verify.
#[test]
fn resumed_checksum_failure_falls_back_to_one_clean_restart() {
    let full = resume_payload();
    // Same length, different bytes: the 206 tail spliced onto the good
    // prefix will NOT hash to the pin.
    let corrupt: Vec<u8> = full.iter().map(|b| b ^ 0x5A).collect();
    let (base, requests) = spawn_scripted_server(vec![
        Script::DropAfter {
            prefix: full[..1024].to_vec(),
            claimed_total: full.len(),
        },
        Script::HonourRange { full: corrupt },
        Script::HonourRange { full: full.clone() },
    ]);
    let entry = entry_for(
        format!("{base}/model.onnx"),
        sha256_hex(&full),
        full.len() as u64,
    );
    let root = TempDir::new().expect("tempdir");

    download_model(entry, root.path(), None).expect_err("run 1 must abort mid-stream");
    download_model(entry, root.path(), None).expect("run 2 must recover via the one clean restart");

    let reqs = requests.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(reqs.len(), 3, "resume + failed verify + clean restart");
    assert!(
        parse_range_offset(&reqs[1]).is_some(),
        "request 2 must be the ranged resume",
    );
    assert_eq!(
        parse_range_offset(&reqs[2]),
        None,
        "the post-mismatch restart must be a FRESH request (no Range); got:\n{}",
        reqs[2],
    );
    let landed = std::fs::read(root.path().join("test-model/model.onnx")).unwrap();
    assert_eq!(landed, full);
}

/// When the clean restart ALSO fails verification, the error surfaces as
/// today's `ModelChecksumMismatch` and the staging tree is removed — the
/// retry is one-shot, never a loop.
#[test]
fn resumed_checksum_failure_then_fresh_failure_errors_and_cleans() {
    let full = resume_payload();
    let corrupt: Vec<u8> = full.iter().map(|b| b ^ 0x5A).collect();
    let (base, requests) = spawn_scripted_server(vec![
        Script::DropAfter {
            prefix: full[..1024].to_vec(),
            claimed_total: full.len(),
        },
        Script::HonourRange {
            full: corrupt.clone(),
        },
        // The clean restart is served corrupt bytes too.
        Script::IgnoreRange { full: corrupt },
    ]);
    let entry = entry_for(
        format!("{base}/model.onnx"),
        sha256_hex(&full),
        full.len() as u64,
    );
    let root = TempDir::new().expect("tempdir");

    download_model(entry, root.path(), None).expect_err("run 1 must abort mid-stream");
    let err = download_model(entry, root.path(), None)
        .expect_err("run 2 must fail after the one-shot retry");
    assert!(
        matches!(err, TomeError::ModelChecksumMismatch { .. }),
        "expected ModelChecksumMismatch after the failed retry, got {err:?}",
    );

    let reqs = requests.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(reqs.len(), 3, "resume + failed verify + ONE retry, no loop");
    assert!(
        !root.path().join("test-model.partial").exists(),
        "a verification-class failure must remove the staging tree",
    );
    assert!(
        !root.path().join("test-model").exists(),
        "no model dir may land on double checksum failure",
    );
}

/// Two-file (primary + one aux) entry whose URLs point at `base`. Same
/// `Box::leak` fixture discipline as [`entry_for`]; the aux-URL slice needs
/// its own leak to satisfy the `&'static [&'static str]` field.
fn two_file_entry_for(base: &str, primary_sha: String, primary_size: u64) -> &'static ModelEntry {
    let primary_url = format!("{base}/model.onnx");
    let aux_url = format!("{base}/tokenizer.json");
    let aux_urls: &'static [&'static str] =
        Box::leak(vec![&*Box::leak(aux_url.into_boxed_str())].into_boxed_slice());
    Box::leak(Box::new(ModelEntry {
        name: "test-model",
        version: "1",
        kind: ModelKind::Embedder,
        source_url: Box::leak(primary_url.into_boxed_str()),
        sha256: Box::leak(primary_sha.into_boxed_str()),
        size_bytes: primary_size,
        licence: "MIT",
        embedding_dim: Some(384),
        files: &["model.onnx", "tokenizer.json"],
        aux_urls,
    }))
}

/// A deterministic aux payload distinct from [`resume_payload`].
fn aux_payload() -> Vec<u8> {
    (0u32..1024)
        .flat_map(|i| (i ^ 0x00FF_00FF).to_le_bytes())
        .collect()
}

/// The #480 two-file resume scenario: run 1 lands the primary in full but is
/// interrupted mid-aux; run 2 must (a) NOT re-fetch the completed,
/// checksum-verified primary — no second `GET /model.onnx` — and (b) resume
/// the aux with `Range: bytes=<staged len>-`, appending the 206 tail.
#[test]
fn aux_interruption_resumes_aux_via_206_without_refetching_primary() {
    let primary = resume_payload();
    let aux = aux_payload();
    let (base, requests) = spawn_scripted_server(vec![
        // Run 1: primary completes (no partial exists yet → no Range → 200)…
        Script::HonourRange {
            full: primary.clone(),
        },
        // …then the aux drops mid-stream.
        Script::DropAfter {
            prefix: aux[..512].to_vec(),
            claimed_total: aux.len(),
        },
        // Run 2: the ONLY request is the ranged aux fetch.
        Script::HonourRange { full: aux.clone() },
    ]);
    let entry = two_file_entry_for(&base, sha256_hex(&primary), primary.len() as u64);
    let root = TempDir::new().expect("tempdir");

    download_model(entry, root.path(), None).expect_err("run 1 must abort during the aux fetch");
    let partial_dir = root.path().join("test-model.partial");
    let staged_aux = std::fs::metadata(partial_dir.join("tokenizer.json"))
        .map(|m| m.len())
        .expect("run 1 must retain the interrupted aux partial");
    assert!(staged_aux > 0, "retained aux partial must be non-empty");
    assert_eq!(
        std::fs::metadata(partial_dir.join("model.onnx"))
            .map(|m| m.len())
            .expect("run 1 must retain the completed primary"),
        primary.len() as u64,
        "the primary must be complete in the retained staging tree",
    );

    download_model(entry, root.path(), None).expect("run 2 must resume the aux and succeed");

    let reqs = requests.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(
        reqs.len(),
        3,
        "primary + aux (run 1), aux only (run 2); got:\n{reqs:#?}",
    );
    assert_eq!(
        reqs.iter().filter(|r| r.contains("/model.onnx")).count(),
        1,
        "the completed primary must NOT be re-fetched on run 2; requests:\n{reqs:#?}",
    );
    assert!(
        reqs[2].contains("/tokenizer.json"),
        "run 2's only request must target the aux; request was:\n{}",
        reqs[2],
    );
    assert_eq!(
        parse_range_offset(&reqs[2]),
        Some(staged_aux),
        "run 2 must request `Range: bytes=<staged aux len>-`; request was:\n{}",
        reqs[2],
    );

    let landed_primary = std::fs::read(root.path().join("test-model/model.onnx")).unwrap();
    assert_eq!(landed_primary, primary);
    let landed_aux = std::fs::read(root.path().join("test-model/tokenizer.json")).unwrap();
    assert_eq!(
        landed_aux, aux,
        "the resumed aux must reassemble to the full payload"
    );
    assert!(
        !partial_dir.exists(),
        "partial dir must be gone after a successful aux-only resume",
    );
}

/// A 206 whose `Content-Range` start disagrees with the requested offset is
/// never appended: the downloader restarts clean (one fresh un-ranged
/// request). Driven on the AUX file — the unchecksummed path where a
/// wrong-offset splice would previously have landed silently (#480): the
/// pre-fix appender would have written `prefix + full` (a corrupt sidecar)
/// with no later verification to catch it.
#[test]
fn aux_content_range_mismatch_falls_back_to_clean_restart() {
    let primary = resume_payload();
    let aux = aux_payload();
    let (base, requests) = spawn_scripted_server(vec![
        // Run 1: primary completes, aux drops mid-stream.
        Script::HonourRange {
            full: primary.clone(),
        },
        Script::DropAfter {
            prefix: aux[..512].to_vec(),
            claimed_total: aux.len(),
        },
        // Run 2: the ranged aux resume gets a 206 claiming the WRONG start…
        Script::MismatchedContentRange { full: aux.clone() },
        // …so the downloader must discard it and re-request fresh (no Range).
        Script::HonourRange { full: aux.clone() },
    ]);
    let entry = two_file_entry_for(&base, sha256_hex(&primary), primary.len() as u64);
    let root = TempDir::new().expect("tempdir");

    download_model(entry, root.path(), None).expect_err("run 1 must abort during the aux fetch");
    download_model(entry, root.path(), None).expect("run 2 must recover via a clean aux restart");

    let reqs = requests.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(
        reqs.len(),
        4,
        "primary + aux (run 1), refused 206 + fresh restart (run 2); got:\n{reqs:#?}",
    );
    assert!(
        parse_range_offset(&reqs[2]).is_some(),
        "request 3 must be the ranged aux resume attempt; request was:\n{}",
        reqs[2],
    );
    assert_eq!(
        parse_range_offset(&reqs[3]),
        None,
        "the post-mismatch restart must be a FRESH request (no Range); got:\n{}",
        reqs[3],
    );
    let landed_aux = std::fs::read(root.path().join("test-model/tokenizer.json")).unwrap();
    assert_eq!(
        landed_aux, aux,
        "a mismatched 206 must never be appended — the landed aux would be a \
         wrong-offset splice",
    );
}

/// A 206 with NO `Content-Range` header at all (a stripping proxy) is
/// equally untrustworthy: even though this server happens to send the
/// correct tail, the offset is unverifiable, so the downloader must refuse
/// the append and restart clean. The pre-#480 appender would have accepted
/// it in 2 requests; the fix costs exactly one extra fresh request.
#[test]
fn missing_content_range_206_falls_back_to_clean_restart() {
    let full = resume_payload();
    let (base, requests) = spawn_scripted_server(vec![
        Script::DropAfter {
            prefix: full[..1024].to_vec(),
            claimed_total: full.len(),
        },
        Script::MissingContentRange { full: full.clone() },
        Script::HonourRange { full: full.clone() },
    ]);
    let entry = entry_for(
        format!("{base}/model.onnx"),
        sha256_hex(&full),
        full.len() as u64,
    );
    let root = TempDir::new().expect("tempdir");

    download_model(entry, root.path(), None).expect_err("run 1 must abort mid-stream");
    download_model(entry, root.path(), None).expect("run 2 must recover via a clean restart");

    let reqs = requests.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(
        reqs.len(),
        3,
        "interrupted run + refused header-less 206 + fresh restart; got:\n{reqs:#?}",
    );
    assert!(
        parse_range_offset(&reqs[1]).is_some(),
        "request 2 must be the ranged resume attempt",
    );
    assert_eq!(
        parse_range_offset(&reqs[2]),
        None,
        "the post-refusal restart must be a FRESH request (no Range); got:\n{}",
        reqs[2],
    );
    let landed = std::fs::read(root.path().join("test-model/model.onnx")).unwrap();
    assert_eq!(landed, full);
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
