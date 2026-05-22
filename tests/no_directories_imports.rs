//! Structural guard: forbid use of the `directories` crate.
//!
//! Tome resolves the home directory via raw `$HOME` env-var inspection
//! (research §R-1). The `directories` crate was never a real
//! dependency — the Phase 3 path-resolver code mirrored its XDG
//! conventions but resolved via `std::env`. Phase 4 / F2a drops the
//! XDG layout entirely; this test stays as a forward-looking guard so
//! a future patch doesn't accidentally pull `directories` in.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

const FORBIDDEN: &[&str] = &["directories::", "extern crate directories"];

fn collect_rs_files(root: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).expect("read_dir under src/") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        let ft = entry.file_type().expect("file_type");
        if ft.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension() == Some(OsStr::new("rs")) {
            out.push(path);
        }
    }
}

#[test]
fn no_directories_crate_imports() {
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
        "`directories` crate is forbidden under src/:\n{}",
        violations.join("\n"),
    );
}
