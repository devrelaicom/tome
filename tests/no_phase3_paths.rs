//! Structural guard: forbid Phase 3 path identifiers and the deleted
//! `workspaces.txt` opt-in registry file.
//!
//! FR-304 ("no Phase 3 fallback") requires that nothing in `src/`
//! references the XDG-separated path fields the F2a slice deleted from
//! [`tome::paths::Paths`]. The regex is anchored on `paths.` field
//! access so that legitimate variable names (e.g. the doctor JSON
//! `workspace_registry` envelope field, which carries its own
//! deprecation story) aren't flagged.
//!
//! The `workspaces.txt` literal is checked outside any `paths.` prefix
//! because the on-disk file is gone entirely.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

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
fn no_phase3_path_field_access() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs_files(&src, &mut files);

    // `paths.config_dir`, `paths.data_dir`, `paths.state_dir`,
    // `paths.cache_dir` (bare; `cache_dir_for` is the surviving
    // accessor method and ends with `_for`).
    let field_re =
        Regex::new(r"\bpaths\.(config_dir|data_dir|state_dir|cache_dir|workspace_registry)\b")
            .unwrap();

    // Phase 3 Scope-aware accessors that were deleted in F2a.
    let method_re = Regex::new(
        r"\bpaths\.(config_file_for|index_db_for|index_lock_for|workspace_marker_dir)\b",
    )
    .unwrap();

    // The opt-in registry file name — gone in Phase 4. No more reads,
    // no more writes.
    let literal_re = Regex::new(r#""workspaces\.txt""#).unwrap();

    let mut violations: Vec<String> = Vec::new();
    for file in &files {
        let contents = fs::read_to_string(file).expect("read .rs file");
        for cap in field_re.captures_iter(&contents) {
            violations.push(format!("  {}: paths.{}", file.display(), &cap[1]));
        }
        for cap in method_re.captures_iter(&contents) {
            violations.push(format!("  {}: paths.{}(...)", file.display(), &cap[1]));
        }
        if literal_re.is_match(&contents) {
            violations.push(format!(
                "  {}: contains \"workspaces.txt\" literal",
                file.display()
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "Phase 3 path identifiers are forbidden under src/:\n{}",
        violations.join("\n"),
    );
}
