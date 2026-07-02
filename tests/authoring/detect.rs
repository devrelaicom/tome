//! Integration coverage for the public `authoring::detect` + `untrusted`
//! surface (US2 foundation). Exercises the source-format detector and the
//! untrusted-read guard through the crate's public API exactly as the
//! `convert` importers will, proving the boundary refuses escapes/symlinks and
//! that detection maps structure → (harness, level).

use std::fs;
use std::path::Path;

use tome::authoring::detect::{ArtifactLevel, SourceHarness, detect};
use tome::authoring::untrusted::UntrustedRoot;

/// Build a canonicalised source root and run `setup` against it.
fn source(setup: impl FnOnce(&Path)) -> (tempfile::TempDir, UntrustedRoot) {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    setup(&base);
    let root = UntrustedRoot::open(&base).unwrap();
    (tmp, root)
}

#[test]
fn detect_maps_cc_marketplace_to_catalog() {
    let (_t, root) = source(|base| {
        fs::create_dir(base.join(".claude-plugin")).unwrap();
        fs::write(base.join(".claude-plugin/marketplace.json"), b"{}").unwrap();
    });
    let d = detect(&root, None, ArtifactLevel::Catalog).unwrap();
    assert_eq!(d.harness, SourceHarness::ClaudeCode);
    assert_eq!(d.level, ArtifactLevel::Catalog);
}

#[test]
fn detect_maps_native_skill_and_from_overrides_harness() {
    let (_t, root) = source(|base| {
        fs::write(base.join("SKILL.md"), b"---\nname: foo\n---\nbody\n").unwrap();
    });
    let generic = detect(&root, None, ArtifactLevel::Skill).unwrap();
    assert_eq!(generic.harness, SourceHarness::AgentSkills);

    let opencode = detect(&root, Some(SourceHarness::OpenCode), ArtifactLevel::Skill).unwrap();
    assert_eq!(opencode.harness, SourceHarness::OpenCode);
    assert_eq!(opencode.level, ArtifactLevel::Skill);
}

#[test]
fn detect_reports_level_mismatch_as_usage() {
    let (_t, root) = source(|base| {
        fs::write(base.join("SKILL.md"), b"body").unwrap();
    });
    // A skill source asked to convert as a plugin.
    let err = detect(&root, None, ArtifactLevel::Plugin).unwrap_err();
    assert_eq!(err.exit_code(), 2);
}

#[test]
fn detect_unrecognized_source_is_83() {
    let (_t, root) = source(|base| {
        fs::write(base.join("README.md"), b"hi").unwrap();
    });
    let err = detect(&root, None, ArtifactLevel::Plugin).unwrap_err();
    assert_eq!(err.exit_code(), 83);
}

#[cfg(unix)]
#[test]
fn untrusted_root_refuses_symlink_escape() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    fs::write(base.join("SKILL.md"), b"body").unwrap();
    // A symlink pointing outside the source root.
    let secret = tmp.path().parent().unwrap().join("secret.txt");
    let _ = fs::write(&secret, b"top secret");
    std::os::unix::fs::symlink(&secret, base.join("leak.md")).unwrap();

    let root = UntrustedRoot::open(&base).unwrap();
    // The clean body reads fine...
    assert_eq!(root.read_body(Path::new("SKILL.md")).unwrap(), "body");
    // ...the symlink escape is refused (exit 7), not followed.
    let err = root.read_body(Path::new("leak.md")).unwrap_err();
    assert_eq!(err.exit_code(), 7);
}

#[test]
fn untrusted_root_refuses_parent_dir_traversal() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    fs::create_dir(base.join("inner")).unwrap();
    let root = UntrustedRoot::open(&base.join("inner")).unwrap();
    let err = root
        .read_text(Path::new("../escape.txt"), 1024)
        .unwrap_err();
    assert_eq!(err.exit_code(), 7);
}
