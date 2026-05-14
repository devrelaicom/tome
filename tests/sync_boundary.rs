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
//! Phase 3 release-binary baseline (macOS arm64, stripped, lto = "thin",
//! panic = abort, opt-level = 3):
//!
//! - Start of Phase 3 (deps wired, no `src/mcp/` yet): 20.91 MiB
//!   (21,922,336 bytes) measured on 2026-05-14. Essentially equal to the
//!   end-of-Phase-2 number (20.91 MiB) — LTO drops the un-referenced
//!   rmcp + tokio object code until the MCP module imports them.
//! - End of Phase 3 (post-US1): expected to land closer to the research
//!   §R-2 projection of ~22.8 MiB on macOS / ~31.5 MiB on Linux. Re-record
//!   here when the MCP module is feature-complete.
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
