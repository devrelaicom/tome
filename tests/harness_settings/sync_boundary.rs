//! Structural test enforcing the sync-only invariant outside `src/mcp/`.
//!
//! Phase 3 introduces `tokio` and `rmcp` strictly inside `src/mcp/`. Every
//! other module in `src/` must remain synchronous — no `async fn`, no
//! `.await`, no `tokio::` references, no `tokio_*` companion crates.
//!
//! The MCP server is the constitution's anticipated forcing function for
//! async; nothing else gets to import the async runtime. If you find
//! yourself reaching for `tokio::spawn` from a `commands/` module, stop and
//! revisit the design instead.
//!
//! Phase 4 extends the sync-only boundary to the new `src/summarise/`,
//! `src/harness/`, `src/settings/`, and `src/util/` modules: `llama-cpp-2`
//! is a sync API, `toml_edit` is sync, and the harness modules are pure
//! file-system code. No exemption is added.
//!
//! Release-binary baselines (macOS arm64, stripped, lto = "thin",
//! panic = abort, opt-level = 3):
//!
//! - End of Phase 3 (v0.3.0): 22.04 MiB.
//! - Start of Phase 4 (deps wired — llama-cpp-2 + toml_edit + serde_json
//!   `preserve_order` — no `src/summarise/` yet): 22.13 MiB (23,196,656
//!   bytes) measured 2026-05-22. Essentially unchanged from v0.3.0; LTO
//!   drops the un-referenced llama.cpp static lib and `toml_edit` object
//!   code until those modules import them.
//! - End of Phase 4: projected ~28.4 MiB on macOS arm64, ~34 MB on Linux
//!   x86_64 (research §R-4). Re-record here once F6 + F7 + US4 wire the
//!   summariser + harness modules into the binary.
//!
//! CI also gates this number via the existing 50 MB release-binary check
//! (constitution NFR-001).

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

const FORBIDDEN: &[&str] = &["async fn", ".await", "tokio::", "tokio_"];

fn is_exempt(path: &Path) -> bool {
    // `src/mcp/` is the one async island. Anything underneath it is exempt.
    path.components()
        .any(|c| c.as_os_str() == OsStr::new("mcp"))
}

fn collect_rs_files(root: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).expect("read_dir under src/") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        let ft = entry.file_type().expect("file_type");
        if ft.is_dir() {
            if is_exempt(&path) {
                continue;
            }
            collect_rs_files(&path, out);
        } else if path.extension() == Some(OsStr::new("rs")) {
            out.push(path);
        }
    }
}

#[test]
fn sync_boundary_outside_mcp() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs_files(&src, &mut files);

    let mut violations: Vec<String> = Vec::new();
    for file in &files {
        let contents = fs::read_to_string(file).expect("read .rs file");
        for needle in FORBIDDEN {
            if contents.contains(needle) {
                violations.push(format!("  {}: contains {needle:?}", file.display()));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "sync-only invariant violated outside src/mcp/:\n{}",
        violations.join("\n"),
    );
}

/// Phase 9 — the shared meta-skill compute (`authoring::meta`) is the install
/// path behind BOTH the CLI and the MCP tool. The MCP side calls it under
/// `spawn_blocking`, so the module itself MUST stay sync. The generic walker
/// above already covers it; this names it explicitly so the invariant is
/// self-documenting and a future async leak in this exact file is caught with a
/// pointed message.
#[test]
fn authoring_meta_is_sync() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/authoring/meta.rs");
    let contents = fs::read_to_string(&file).expect("read src/authoring/meta.rs");
    for needle in FORBIDDEN {
        assert!(
            !contents.contains(needle),
            "src/authoring/meta.rs must stay sync but contains {needle:?}"
        );
    }
}

/// Phase 10 — the entire `src/telemetry/**` tree MUST stay `tokio`-free: it is a
/// cross-cutting concern called from `commands/*`, the app boundary, AND the
/// MCP island, where the MCP timer `spawn_blocking`s into `telemetry::flush`.
/// The generic walker above already covers it (telemetry is not under `mcp/`),
/// but we assert it explicitly so a future async leak anywhere under
/// `src/telemetry/` is caught with a pointed message naming the offending file.
#[test]
fn telemetry_is_sync() {
    let telemetry = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/telemetry");
    let mut files = Vec::new();
    collect_rs_files(&telemetry, &mut files);
    assert!(!files.is_empty(), "expected .rs files under src/telemetry/");

    let mut violations: Vec<String> = Vec::new();
    for file in &files {
        let contents = fs::read_to_string(file).expect("read telemetry .rs file");
        for needle in FORBIDDEN {
            if contents.contains(needle) {
                violations.push(format!("  {}: contains {needle:?}", file.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "src/telemetry/ must stay tokio-free:\n{}",
        violations.join("\n"),
    );
}

/// Phase 12 — the BYOK/BYOM provider layer is the MOST LIKELY future async-leak
/// surface: it does network I/O (`reqwest::blocking`) and is reached from the
/// MCP island (which `spawn_blocking`s into it for the remote embedder/reranker).
/// The user explicitly chose a hand-rolled ALL-SYNC transport over a second async
/// island, so `src/provider/`, `src/embedding/remote.rs`, and
/// `src/summarise/remote.rs` MUST stay sync. The generic `sync_boundary_outside_mcp`
/// walker above already covers these structurally (none are under `mcp/`); this
/// names them explicitly so a future `tokio`/`async`/`.await` leak in this exact
/// surface is caught with a pointed, self-documenting message.
#[test]
fn provider_layer_is_sync() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    // The whole `src/provider/` tree (transport, config, per-kind shapes, …).
    collect_rs_files(&manifest.join("src/provider"), &mut files);
    // Plus the two remote-capability shims that live outside `provider/`.
    files.push(manifest.join("src/embedding/remote.rs"));
    files.push(manifest.join("src/summarise/remote.rs"));

    let mut violations: Vec<String> = Vec::new();
    for file in &files {
        let contents =
            fs::read_to_string(file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        for needle in FORBIDDEN {
            if contents.contains(needle) {
                violations.push(format!("  {}: contains {needle:?}", file.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "the provider layer must stay sync (hand-rolled reqwest::blocking, no second async island):\n{}",
        violations.join("\n"),
    );
}
