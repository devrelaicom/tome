//! Structural grep guard (R-7). Every struct that derives `Deserialize` in
//! `src/catalog/manifest.rs` and `src/config.rs` must carry
//! `#[serde(deny_unknown_fields)]`. Adding a deserialisable struct without
//! the attribute is a regression caught here. The exhaustive bad-manifest
//! corpus lands in Phase 4 (US2).

use std::fs;
use std::path::PathBuf;

fn project_file(rel: &str) -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir).join(rel)
}

fn assert_every_deserialize_has_deny_unknown(path: &str) {
    let contents = fs::read_to_string(project_file(path))
        .unwrap_or_else(|e| panic!("could not read {}: {}", path, e));
    let lines: Vec<&str> = contents.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        let l = line.trim();
        // A line that derives `serde::Deserialize`, or `Deserialize` imported
        // from `serde`. Tolerate any number of preceding/following derives.
        let derives_deserialize =
            l.starts_with("#[derive(") && l.contains("Deserialize") && !l.contains("// not-strict");
        if !derives_deserialize {
            continue;
        }
        // The next non-attribute, non-blank line that begins a `pub struct`
        // or `struct` must be preceded by a `#[serde(deny_unknown_fields)]`
        // attribute somewhere in the attribute cluster after `#[derive(...)]`.
        let mut saw_deny_unknown = false;
        for follow in &lines[idx + 1..] {
            let f = follow.trim();
            if f.is_empty() {
                continue;
            }
            if f.starts_with("#[serde(") && f.contains("deny_unknown_fields") {
                saw_deny_unknown = true;
                continue;
            }
            if f.starts_with("#[") {
                continue;
            }
            // First non-attribute line — must be a struct definition.
            assert!(
                f.starts_with("pub struct") || f.starts_with("struct"),
                "in {}, expected a struct after a #[derive(Deserialize)] cluster but found: {}",
                path,
                f
            );
            assert!(
                saw_deny_unknown,
                "in {}, struct line `{}` derives Deserialize without #[serde(deny_unknown_fields)]",
                path, f
            );
            break;
        }
    }
}

#[test]
fn manifest_module_structs_all_carry_deny_unknown_fields() {
    assert_every_deserialize_has_deny_unknown("src/catalog/manifest.rs");
}

#[test]
fn config_module_structs_all_carry_deny_unknown_fields() {
    assert_every_deserialize_has_deny_unknown("src/config.rs");
}
