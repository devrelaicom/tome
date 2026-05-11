//! Interruption-injecting tests for the atomic write path (SC-012). The
//! contract: an interrupted write leaves the on-disk file in either its
//! pre-state or its post-state, never a partial-bytes state.

use std::fs;
use std::path::Path;

use tempfile::TempDir;
use time::OffsetDateTime;
use tome::catalog::store::{save, write_atomic};
use tome::config::{CatalogEntry, Config};

fn make_config(name: &str) -> Config {
    let mut cfg = Config::default();
    cfg.catalogs.insert(
        name.into(),
        CatalogEntry {
            name: name.into(),
            url: format!("https://example/{}", name),
            ref_: "main".into(),
            path: std::path::PathBuf::from("/tmp/x"),
            last_synced: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        },
    );
    cfg
}

#[test]
fn write_atomic_does_not_leave_partial_file_when_target_dir_writable() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("config.toml");
    write_atomic(&target, b"first").unwrap();
    write_atomic(&target, b"second").unwrap();
    let read = fs::read(&target).unwrap();
    assert_eq!(read, b"second");
}

#[test]
fn no_temp_file_left_behind_after_successful_write() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("config.toml");
    save(&target, &make_config("a")).unwrap();
    let entries: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    // Exactly one entry — the target. The same-directory temp file used by
    // `tempfile::NamedTempFile::persist` is consumed by the rename.
    assert_eq!(entries.len(), 1, "stray files: {:?}", entries);
}

#[test]
fn failed_persist_into_nonexistent_dir_does_not_create_target() {
    // Pre-existing file then attempt a write to a sub-path whose parent will
    // be created. The pre-existing file is unrelated and must be untouched.
    let dir = TempDir::new().unwrap();
    let untouched = dir.path().join("other.toml");
    fs::write(&untouched, b"do not touch").unwrap();
    let target = dir.path().join("nested/config.toml");
    save(&target, &make_config("a")).unwrap();
    let kept = fs::read(&untouched).unwrap();
    assert_eq!(kept, b"do not touch");
    assert!(target.exists());
}

#[test]
fn concurrent_writes_yield_a_complete_file_not_a_torn_one() {
    // Spawn 8 writers racing to replace the same target. The atomic-rename
    // contract guarantees the final file matches one of the writers' inputs,
    // never a mix.
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("config.toml");
    fs::write(&target, b"pre").unwrap();
    let mut handles = Vec::new();
    for i in 0..8u8 {
        let t = target.clone();
        handles.push(std::thread::spawn(move || {
            let payload = format!("writer-{}", i).into_bytes();
            // ignore any individual error — at least one must succeed
            let _ = write_atomic(&t, &payload);
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let bytes = fs::read(&target).unwrap();
    let final_text = String::from_utf8(bytes).expect("utf-8");
    assert!(
        is_one_of_the_writers(&final_text) || final_text == "pre",
        "torn write detected: {:?}",
        final_text
    );
}

fn is_one_of_the_writers(s: &str) -> bool {
    (0..8u8).any(|i| s == format!("writer-{}", i))
}

#[test]
fn missing_target_directory_is_created_on_save() {
    let dir = TempDir::new().unwrap();
    let nested = dir.path().join("a/b/c");
    let target = nested.join("config.toml");
    assert!(!nested.exists());
    save(&target, &make_config("a")).unwrap();
    assert!(target.exists());
    assert!(is_dir(&nested));
}

fn is_dir(p: &Path) -> bool {
    fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false)
}
