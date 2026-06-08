//! Native `SKILL.md` importers (Tier 1): Cursor, OpenCode, Cline, and generic
//! Agent Skills. A native skill source is a single directory with `SKILL.md` at
//! its root (plus optional supporting subdirectories).
//!
//! These formats share Tome's `SKILL.md` shape, so the importer reuses the
//! shared skill parser ([`claude_code::import_skill`]) at the source root and
//! then applies only the per-harness supporting-path differences. Today that is
//! Cline, which names its supporting directories `docs/` and `templates/` where
//! Tome uses `references/` and `assets/`.

use std::path::Path;

use crate::authoring::detect::SourceHarness;
use crate::authoring::import::claude_code::import_skill;
use crate::authoring::ir::EntryIr;
use crate::authoring::untrusted::UntrustedRoot;
use crate::error::TomeError;

/// Import a bare native skill (a directory with `SKILL.md` at its root) into an
/// [`EntryIr`], applying any harness-specific supporting-path remap.
pub fn import(
    root: &UntrustedRoot,
    harness: SourceHarness,
    source_path: &Path,
) -> Result<EntryIr, TomeError> {
    let dir_name = source_path
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("skill");
    let mut entry = import_skill(root, Path::new(""), dir_name)?;
    if harness == SourceHarness::Cline {
        remap_cline(&mut entry);
    }
    Ok(entry)
}

/// Cline stores supporting material under `docs/` and `templates/`; remap those
/// to Tome's `references/` and `assets/` conventions, leaving everything else.
fn remap_cline(entry: &mut EntryIr) {
    for sf in &mut entry.supporting_files {
        let remapped = if let Ok(rest) = sf.relative.strip_prefix("docs") {
            Some(Path::new("references").join(rest))
        } else if let Ok(rest) = sf.relative.strip_prefix("templates") {
            Some(Path::new("assets").join(rest))
        } else {
            None
        };
        if let Some(p) = remapped {
            sf.relative = p;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::identity::EntryKind;
    use std::fs;
    use std::path::PathBuf;

    fn skill_dir(setup: impl FnOnce(&Path)) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        let src = base.join("my-skill");
        fs::create_dir(&src).unwrap();
        setup(&src);
        (tmp, src)
    }

    #[test]
    fn imports_a_generic_native_skill() {
        let (_t, src) = skill_dir(|src| {
            fs::write(
                src.join("SKILL.md"),
                "---\nname: my-skill\ndescription: d\n---\nbody ${CLAUDE_PROJECT_DIR}\n",
            )
            .unwrap();
            fs::create_dir(src.join("references")).unwrap();
            fs::write(src.join("references/r.md"), b"ref").unwrap();
        });
        let root = UntrustedRoot::open(&src).unwrap();
        let e = import(&root, SourceHarness::AgentSkills, &src).unwrap();
        assert_eq!(e.kind, EntryKind::Skill);
        assert_eq!(e.name, "my-skill");
        assert!(e.body.contains("${TOME_PROJECT_DIR}"));
        assert_eq!(e.supporting_files.len(), 1);
        assert_eq!(
            e.supporting_files[0].relative,
            PathBuf::from("references/r.md")
        );
    }

    #[test]
    fn cline_remaps_docs_and_templates() {
        let (_t, src) = skill_dir(|src| {
            fs::write(
                src.join("SKILL.md"),
                "---\nname: my-skill\ndescription: d\n---\nbody\n",
            )
            .unwrap();
            fs::create_dir(src.join("docs")).unwrap();
            fs::write(src.join("docs/guide.md"), b"g").unwrap();
            fs::create_dir(src.join("templates")).unwrap();
            fs::write(src.join("templates/t.txt"), b"t").unwrap();
        });
        let root = UntrustedRoot::open(&src).unwrap();
        let e = import(&root, SourceHarness::Cline, &src).unwrap();
        let rels: Vec<PathBuf> = e
            .supporting_files
            .iter()
            .map(|s| s.relative.clone())
            .collect();
        assert!(
            rels.contains(&PathBuf::from("references/guide.md")),
            "{rels:?}"
        );
        assert!(rels.contains(&PathBuf::from("assets/t.txt")), "{rels:?}");
    }

    #[test]
    fn skips_git_metadata_in_a_bare_skill_root() {
        let (_t, src) = skill_dir(|src| {
            fs::write(
                src.join("SKILL.md"),
                "---\nname: my-skill\ndescription: d\n---\nbody\n",
            )
            .unwrap();
            fs::create_dir_all(src.join(".git/objects")).unwrap();
            fs::write(src.join(".git/config"), b"[core]").unwrap();
        });
        let root = UntrustedRoot::open(&src).unwrap();
        let e = import(&root, SourceHarness::AgentSkills, &src).unwrap();
        assert!(
            e.supporting_files.is_empty(),
            "`.git` must not be copied: {:?}",
            e.supporting_files
        );
    }
}
