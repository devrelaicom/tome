//! Tests for the `StandaloneFile` rules-file strategy.
//!
//! Exercises `rules_file::{write_standalone, remove_standalone}` against
//! the parent-dir creation + chmod + symlink-refusal + idempotence
//! contract.

use std::thread::sleep;
use std::time::Duration;

use tempfile::TempDir;
use tome::harness::RulesFrontmatter;
use tome::harness::rules_file::{
    remove_standalone, write_standalone, write_standalone_with_frontmatter,
};

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

// ---------------------------------------------------------------------------
// G3 (FR-026): a namespaced standalone file with a Tome-owned YAML
// front-matter header. The header bytes are pinned distinct from the verbatim
// directive body.
// ---------------------------------------------------------------------------

/// Kiro's `.kiro/steering/tome.md` shape: `inclusion: always` front-matter
/// above the verbatim directive, byte-stable, into a namespaced dir.
#[test]
fn frontmatter_kiro_namespaced_file_byte_stable() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join(".kiro").join("steering").join("tome.md");
    let fm = RulesFrontmatter {
        fields: &[("inclusion", "always")],
    };
    write_standalone_with_frontmatter(&target, &fm, "the directive body\n").unwrap();
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "---\ninclusion: always\n---\nthe directive body\n",
    );
}

/// JetBrains AI Assistant's `.aiassistant/rules/tome.md` shape: the Always
/// apply-mode marker as front-matter. (Modeled as `apply: always` pending the
/// US1 live-probe of the exact AI-Assistant key.)
#[test]
fn frontmatter_jetbrains_namespaced_file_byte_stable() {
    let tmp = TempDir::new().unwrap();
    let target = tmp
        .path()
        .join(".aiassistant")
        .join("rules")
        .join("tome.md");
    let fm = RulesFrontmatter {
        fields: &[("apply", "always")],
    };
    write_standalone_with_frontmatter(&target, &fm, "body\n").unwrap();
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "---\napply: always\n---\nbody\n",
    );
}

/// The front-matter writer inherits the symlink refusal from `write_standalone`.
#[cfg(unix)]
#[test]
fn frontmatter_refuses_write_through_symlink_on_unix() {
    use std::os::unix::fs::symlink;
    let tmp = TempDir::new().unwrap();
    let actual = tmp.path().join("actual.md");
    let link = tmp.path().join("link.md");
    std::fs::write(&actual, "").unwrap();
    symlink(&actual, &link).unwrap();
    let fm = RulesFrontmatter {
        fields: &[("inclusion", "always")],
    };
    let err = write_standalone_with_frontmatter(&link, &fm, "body")
        .expect_err("symlink target must be refused");
    assert!(format!("{err}").contains("symlink"));
}
