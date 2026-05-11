//! Exhaustive negative-case coverage for `plugins[].source` validation
//! (data-model.md §3 step 6). One row per rejection case; each row asserts
//! the precise `ManifestInvalid` variant and that the error message names
//! the manifest file path and the offending value (FR-023, SC-005).

use std::os::unix::fs::symlink;
use std::path::PathBuf;

use tempfile::TempDir;
use tome::catalog::manifest::validate_source;
use tome::error::ManifestInvalid;

fn catalog_with(plugin_subpath: &str) -> (TempDir, PathBuf, PathBuf) {
    // Build a catalog root containing the named subpath as an actual file/dir
    // so canonicalize() succeeds and the only failure path is the validator's
    // semantic checks (URL form, absolute, parent traversal, escape).
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    let manifest_file = root.join("tome-catalog.toml");
    std::fs::write(&manifest_file, b"placeholder").unwrap();
    if !plugin_subpath.is_empty() {
        let abs = root.join(plugin_subpath);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(&abs, b"plugin").ok();
    }
    (temp, root, manifest_file)
}

#[test]
fn https_url_is_rejected() {
    let (_t, root, manifest) = catalog_with("");
    let err = validate_source(&root, &manifest, "https://example.com/repo").unwrap_err();
    assert!(matches!(err, ManifestInvalid::SourceLooksLikeUrl { .. }));
    let display = format!("{}", err);
    assert!(display.contains("https://example.com/repo"));
    assert!(display.contains(manifest.to_str().unwrap()));
}

#[test]
fn file_url_is_rejected() {
    let (_t, root, manifest) = catalog_with("");
    let err = validate_source(&root, &manifest, "file:///abs/path").unwrap_err();
    assert!(matches!(err, ManifestInvalid::SourceLooksLikeUrl { .. }));
}

#[test]
fn ssh_url_is_rejected() {
    let (_t, root, manifest) = catalog_with("");
    let err = validate_source(&root, &manifest, "git@host:owner/repo").unwrap_err();
    assert!(matches!(err, ManifestInvalid::SourceLooksLikeUrl { .. }));
    let display = format!("{}", err);
    assert!(display.contains("git@host:owner/repo"));
}

#[test]
fn absolute_unix_path_is_rejected() {
    let (_t, root, manifest) = catalog_with("");
    let err = validate_source(&root, &manifest, "/etc/passwd").unwrap_err();
    assert!(matches!(err, ManifestInvalid::SourceAbsolute { .. }));
    let display = format!("{}", err);
    assert!(display.contains("/etc/passwd"));
}

#[test]
fn windows_drive_prefix_is_rejected() {
    let (_t, root, manifest) = catalog_with("");
    let err = validate_source(&root, &manifest, "C:\\plugins").unwrap_err();
    assert!(matches!(err, ManifestInvalid::SourceAbsolute { .. }));
}

#[test]
fn parent_traversal_is_rejected_syntactically() {
    let (_t, root, manifest) = catalog_with("");
    let err = validate_source(&root, &manifest, "../escape").unwrap_err();
    assert!(matches!(err, ManifestInvalid::SourceParentTraversal { .. }));
}

#[test]
fn parent_traversal_embedded_is_also_rejected() {
    let (_t, root, manifest) = catalog_with("");
    let err = validate_source(&root, &manifest, "./plugins/../escape").unwrap_err();
    assert!(matches!(err, ManifestInvalid::SourceParentTraversal { .. }));
}

#[test]
fn symlink_outside_catalog_is_rejected() {
    let outside = TempDir::new().unwrap();
    let outside_file = outside.path().join("target");
    std::fs::write(&outside_file, b"escape").unwrap();

    let (_t, root, manifest) = catalog_with("");
    let link_in_catalog = root.join("link");
    symlink(&outside_file, &link_in_catalog).unwrap();

    let err = validate_source(&root, &manifest, "link").unwrap_err();
    assert!(
        matches!(err, ManifestInvalid::SourceEscapesRoot { .. }),
        "got: {:?}",
        err
    );
    let display = format!("{}", err);
    assert!(display.contains("link"));
}

#[test]
fn nonexistent_relative_path_is_unresolvable() {
    let (_t, root, manifest) = catalog_with("");
    let err = validate_source(&root, &manifest, "plugins/missing").unwrap_err();
    assert!(matches!(err, ManifestInvalid::SourceUnresolvable { .. }));
}

#[test]
fn happy_relative_path_resolves_under_root() {
    let (_t, root, manifest) = catalog_with("plugins/foo");
    let resolved = validate_source(&root, &manifest, "plugins/foo").unwrap();
    assert!(resolved.starts_with(root.canonicalize().unwrap()));
}
