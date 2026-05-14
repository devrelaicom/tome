//! Probe well-known agentic-coding harness install directories.
//!
//! Per research §R-7 and `contracts/doctor.md` §Behaviour step 6:
//! detection is **directory existence only** — no content reads, no
//! config parsing (FR-167). This keeps doctor cheap, side-effect-free,
//! and avoids any per-harness schema coupling.
//!
//! The harness list is fixed at compile time. Adding a new harness
//! means appending to [`KNOWN_HARNESSES`] and updating the JSON
//! contract's enum range. No discovery — we deliberately don't scan
//! `$HOME` for unknown dotfiles.

use std::path::{Path, PathBuf};

use crate::doctor::report::HarnessPresence;

/// `(machine_name, dot_dir_name)` for every known harness. The machine
/// name is the wire-shape identifier used in the JSON record; the
/// dot-dir name is what we probe under `$HOME/`.
///
/// Order matches the contract's listing.
pub const KNOWN_HARNESSES: &[(&str, &str)] = &[
    ("claude_code", ".claude"),
    ("codex", ".codex"),
    ("cursor", ".cursor"),
    ("gemini", ".gemini"),
    ("opencode", ".opencode"),
    ("continue", ".continue"),
];

/// Probe each entry in [`KNOWN_HARNESSES`] under `home`. Returns one
/// `HarnessPresence` per known harness in fixed order — present and
/// absent both reported, so the JSON consumer always sees the full
/// list.
pub fn probe(home: &Path) -> Vec<HarnessPresence> {
    KNOWN_HARNESSES
        .iter()
        .map(|(name, dir)| {
            let path: PathBuf = home.join(dir);
            HarnessPresence {
                name: (*name).to_owned(),
                path: path.clone(),
                present: path.is_dir(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn probe_empty_home_reports_all_absent() {
        let tmp = TempDir::new().unwrap();
        let result = probe(tmp.path());
        assert_eq!(result.len(), KNOWN_HARNESSES.len());
        for h in &result {
            assert!(!h.present, "expected {} absent, got present", h.name);
        }
    }

    #[test]
    fn probe_detects_existing_directories() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        std::fs::create_dir_all(tmp.path().join(".cursor")).unwrap();

        let result = probe(tmp.path());
        let claude = result.iter().find(|h| h.name == "claude_code").unwrap();
        assert!(claude.present);
        let cursor = result.iter().find(|h| h.name == "cursor").unwrap();
        assert!(cursor.present);
        let codex = result.iter().find(|h| h.name == "codex").unwrap();
        assert!(!codex.present);
    }

    #[test]
    fn probe_ignores_files_named_like_harness_dirs() {
        // A file at ~/.claude (regular file, not a directory) does NOT
        // count as a detected harness — harnesses install into a dir.
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join(".claude"), b"not a harness").unwrap();
        let result = probe(tmp.path());
        let claude = result.iter().find(|h| h.name == "claude_code").unwrap();
        assert!(!claude.present, "regular file should not count as harness");
    }
}
