//! Tests for the `StandaloneFile` rules-file strategy.
//!
//! Exercises `rules_file::{write_standalone, remove_standalone}` against
//! the parent-dir creation + chmod + symlink-refusal + idempotence
//! contract.

use std::thread::sleep;
use std::time::Duration;

use tempfile::TempDir;
use tome::harness::rules_file::{remove_standalone, write_standalone};

const MTIME_TICK: Duration = Duration::from_millis(1500);

#[test]
fn creates_standalone_file() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("rules.mdc");
    write_standalone(&target, "hello world").unwrap();
    let contents = std::fs::read_to_string(&target).unwrap();
    assert_eq!(contents, "hello world");
}

#[test]
fn creates_parent_directory() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join(".cursor").join("rules").join("tome.mdc");
    write_standalone(&target, "x").unwrap();
    assert!(target.exists());
    assert!(target.parent().unwrap().is_dir());
}

#[cfg(unix)]
#[test]
fn parent_dir_is_0700_on_unix() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("freshdir").join("tome.mdc");
    write_standalone(&target, "x").unwrap();
    let mode = std::fs::metadata(target.parent().unwrap())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700, "fresh parent dir should be chmod 0700");
}

#[test]
fn overwrites_existing_file() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("rules.mdc");
    write_standalone(&target, "A").unwrap();
    write_standalone(&target, "B").unwrap();
    let contents = std::fs::read_to_string(&target).unwrap();
    assert_eq!(contents, "B");
}

#[test]
fn idempotent_rewrite_no_op() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("rules.mdc");
    write_standalone(&target, "stable").unwrap();
    let mtime_before = std::fs::metadata(&target).unwrap().modified().unwrap();
    sleep(MTIME_TICK);
    write_standalone(&target, "stable").unwrap();
    let mtime_after = std::fs::metadata(&target).unwrap().modified().unwrap();
    assert_eq!(
        mtime_before, mtime_after,
        "identical-contents write must not touch the file"
    );
}

#[test]
fn removes_standalone_file() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("subdir").join("rules.mdc");
    write_standalone(&target, "x").unwrap();
    assert!(target.exists());
    let parent = target.parent().unwrap().to_path_buf();
    remove_standalone(&target).unwrap();
    assert!(!target.exists());
    assert!(parent.exists(), "parent directory must be untouched");
}

#[test]
fn remove_on_missing_file_is_no_op() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("does-not-exist.mdc");
    remove_standalone(&target).unwrap();
}

#[cfg(unix)]
#[test]
fn refuses_write_through_symlink_on_unix() {
    use std::os::unix::fs::symlink;
    let tmp = TempDir::new().unwrap();
    let actual = tmp.path().join("actual.mdc");
    let link = tmp.path().join("link.mdc");
    std::fs::write(&actual, "").unwrap();
    symlink(&actual, &link).unwrap();
    let err = write_standalone(&link, "body").expect_err("symlink target must be refused");
    assert!(format!("{err}").contains("symlink"));
}

#[cfg(unix)]
#[test]
fn refuses_remove_through_symlink_on_unix() {
    use std::os::unix::fs::symlink;
    let tmp = TempDir::new().unwrap();
    let actual = tmp.path().join("actual.mdc");
    let link = tmp.path().join("link.mdc");
    std::fs::write(&actual, "stuff").unwrap();
    symlink(&actual, &link).unwrap();
    let err = remove_standalone(&link).expect_err("symlink target must be refused");
    assert!(format!("{err}").contains("symlink"));
}
