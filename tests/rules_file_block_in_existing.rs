//! Tests for the `BlockInExistingFile` rules-file strategy.
//!
//! Exercises `rules_file::{write_block, remove_block}` against the
//! atomic-write + symlink-refusal + idempotence contract.

use std::thread::sleep;
use std::time::Duration;

use tempfile::TempDir;
use tome::harness::BlockBodyStyle;
use tome::harness::rules_file::{remove_block, write_block};

/// Sleep long enough for second-resolution mtime to advance on macOS
/// APFS. 1s is sometimes flaky; 1.5s is safer.
const MTIME_TICK: Duration = Duration::from_millis(1500);

#[test]
fn creates_block_when_file_empty() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("RULES.md");
    std::fs::write(&target, "").unwrap();
    write_block(&target, "hello", BlockBodyStyle::Inline).unwrap();
    let contents = std::fs::read_to_string(&target).unwrap();
    assert_eq!(contents, "<!-- tome:begin -->\nhello\n<!-- tome:end -->\n");
}

#[test]
fn creates_block_when_file_missing() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("RULES.md");
    write_block(&target, "hello", BlockBodyStyle::Inline).unwrap();
    let contents = std::fs::read_to_string(&target).unwrap();
    assert_eq!(contents, "<!-- tome:begin -->\nhello\n<!-- tome:end -->\n");
}

#[test]
fn appends_block_to_existing_content() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("RULES.md");
    std::fs::write(&target, "existing\n").unwrap();
    write_block(&target, "hi", BlockBodyStyle::Inline).unwrap();
    let contents = std::fs::read_to_string(&target).unwrap();
    assert_eq!(
        contents,
        "existing\n\n<!-- tome:begin -->\nhi\n<!-- tome:end -->\n"
    );
}

#[test]
fn updates_existing_block_in_place() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("RULES.md");
    let initial = "top\n<!-- tome:begin -->\nold body\n<!-- tome:end -->\nbottom\n";
    std::fs::write(&target, initial).unwrap();
    write_block(&target, "new body", BlockBodyStyle::Inline).unwrap();
    let contents = std::fs::read_to_string(&target).unwrap();
    assert_eq!(
        contents,
        "top\n<!-- tome:begin -->\nnew body\n<!-- tome:end -->\nbottom\n"
    );
}

#[test]
fn idempotent_rewrite_no_op() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("RULES.md");
    write_block(&target, "body", BlockBodyStyle::Inline).unwrap();
    let mtime_before = std::fs::metadata(&target).unwrap().modified().unwrap();
    sleep(MTIME_TICK);
    write_block(&target, "body", BlockBodyStyle::Inline).unwrap();
    let mtime_after = std::fs::metadata(&target).unwrap().modified().unwrap();
    assert_eq!(
        mtime_before, mtime_after,
        "second write with identical body must not touch the file"
    );
}

#[test]
fn removes_block_preserving_surrounding_content() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("RULES.md");
    std::fs::write(
        &target,
        "top\n\n<!-- tome:begin -->\nbody\n<!-- tome:end -->\nbottom\n",
    )
    .unwrap();
    remove_block(&target).unwrap();
    let contents = std::fs::read_to_string(&target).unwrap();
    assert_eq!(contents, "top\nbottom\n");
}

#[test]
fn remove_block_on_clean_file_is_no_op() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("RULES.md");
    std::fs::write(&target, "no block here\n").unwrap();
    let mtime_before = std::fs::metadata(&target).unwrap().modified().unwrap();
    sleep(MTIME_TICK);
    remove_block(&target).unwrap();
    let mtime_after = std::fs::metadata(&target).unwrap().modified().unwrap();
    assert_eq!(
        mtime_before, mtime_after,
        "remove on a block-less file must not touch the file"
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "no block here\n");
}

#[cfg(unix)]
#[test]
fn refuses_write_through_symlink_on_unix() {
    use std::os::unix::fs::symlink;
    let tmp = TempDir::new().unwrap();
    let actual = tmp.path().join("actual.md");
    let link = tmp.path().join("link.md");
    std::fs::write(&actual, "").unwrap();
    symlink(&actual, &link).unwrap();
    let err = write_block(&link, "body", BlockBodyStyle::Inline)
        .expect_err("symlink target must be refused");
    let msg = format!("{err}");
    assert!(msg.contains("symlink"), "error message: {msg}");
}

#[cfg(unix)]
#[test]
fn refuses_remove_through_symlink_on_unix() {
    use std::os::unix::fs::symlink;
    let tmp = TempDir::new().unwrap();
    let actual = tmp.path().join("actual.md");
    let link = tmp.path().join("link.md");
    std::fs::write(&actual, "<!-- tome:begin -->\nx\n<!-- tome:end -->\n").unwrap();
    symlink(&actual, &link).unwrap();
    let err = remove_block(&link).expect_err("symlink target must be refused");
    assert!(format!("{err}").contains("symlink"));
}

#[test]
fn multiple_block_collapse() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("RULES.md");
    let initial = "<!-- tome:begin -->\nfirst\n<!-- tome:end -->\n<!-- tome:begin -->\nsecond\n<!-- tome:end -->\n";
    std::fs::write(&target, initial).unwrap();
    write_block(&target, "canonical", BlockBodyStyle::Inline).unwrap();
    let contents = std::fs::read_to_string(&target).unwrap();
    assert_eq!(
        contents,
        "<!-- tome:begin -->\ncanonical\n<!-- tome:end -->\n"
    );
}
