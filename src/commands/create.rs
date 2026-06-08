//! Shared wrapper for `tome {catalog,plugin,skill} create` — the single
//! arg→`authoring::scaffold`→emit path, parameterized by the artifact level
//! each command surface fixes. The per-level shims assemble a [`CreateRequest`]
//! and hand it here.
//!
//! This slice ships the **built-in** templates (an IR scaffold, lint-clean by
//! construction). A remote/custom `--template` renders a fetched directory tree
//! — a separate path that lands in a fast-follow; a non-built-in `--template`
//! is reported as [`TomeError::TemplateInvalid`] here.
//!
//! `--output` lands the scaffold at `<output>/<NAME>/`; `--into` injects it into
//! an existing Tome artifact, reusing the `convert` wrapper's target detection
//! and catalog registration ([`crate::commands::convert::into_target`] /
//! [`register_plugin_in_catalog`] — the SSOT for `--into`). A skill injected
//! into a plugin is emitted as a *naked* skill under the plugin's `skills/`.

use std::path::PathBuf;

use serde_json::json;

use crate::authoring::detect::ArtifactLevel;
use crate::authoring::emit::{EmitOptions, EmitOutcome, emit};
use crate::authoring::scaffold::{CreateParams, create_artifact};
use crate::commands::convert::{into_target, register_plugin_in_catalog};
use crate::error::TomeError;
use crate::output::{Mode, write_json};
use crate::workspace::ResolvedScope;

/// The two reserved built-in template names this slice ships.
const BUILTIN_DEFAULT: &str = "default";
const BUILTIN_BARE_SKILL: &str = "bare-skill";

/// Level-agnostic create request assembled by the per-level shims.
#[derive(Debug)]
pub struct CreateRequest {
    pub level: ArtifactLevel,
    pub name: String,
    pub template: Option<String>,
    pub output: Option<PathBuf>,
    pub into: Option<PathBuf>,
    pub force: bool,
    /// (skill) emit a naked skill instead of a plugin-wrapped one.
    pub bare: bool,
    /// (skill) the wrapping plugin name; `None` → `name`.
    pub plugin_name: Option<String>,
}

/// Scaffold a new artifact at the request's level.
pub fn run(req: CreateRequest, _scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    // `--template bare-skill` is an alias for `--bare`; any other non-built-in
    // value is a remote/custom template, deferred to a fast-follow.
    if let Some(t) = &req.template
        && !is_builtin_template(t)
    {
        return Err(TomeError::TemplateInvalid {
            template: t.clone(),
            reason:
                "remote/custom templates are not yet available; omit --template to use the built-in"
                    .to_owned(),
        });
    }
    let bare = req.bare || req.template.as_deref() == Some(BUILTIN_BARE_SKILL);

    // Resolve where the scaffold lands. `--into` reuses the convert wrapper's
    // target detection: a plugin → a catalog lands at `<catalog>/<name>/` and
    // registers; a skill → a plugin lands at `<plugin>/skills/<name>/` (no
    // manifest edit). The skill-into-plugin case forces *naked* skill emission
    // (it joins an existing plugin, so it must not re-wrap one).
    let (output_dir, register) = match &req.into {
        Some(into) => into_target(into, req.level)?,
        None => (
            req.output.clone().unwrap_or_else(|| PathBuf::from(".")),
            None,
        ),
    };
    let into_existing_plugin = req.into.is_some() && register.is_none();

    // `description`/`author_name` are reserved for the `--description`/`--author`
    // flags (a fast-follow); there are no such flags yet, so they are `None`
    // here and the scaffold falls back to its name-derived description +
    // placeholder owner. NOT a wiring bug.
    let params = CreateParams {
        name: req.name.clone(),
        plugin_name: req.plugin_name.clone(),
        description: None,
        author_name: None,
        date: today(),
        bare: bare || into_existing_plugin,
    };
    let (artifact, final_name) = create_artifact(req.level, &params)?;

    let target = output_dir.join(&final_name);
    let outcome = emit(
        &artifact,
        &target,
        EmitOptions {
            force: req.force,
            dry_run: false,
        },
    )?;

    // Register the injected plugin in the target catalog's `plugins[]` (atomic,
    // comment-preserving, idempotent).
    if let Some(catalog_manifest) = register {
        register_plugin_in_catalog(&catalog_manifest, &final_name)?;
    }

    emit_report(req.level, &final_name, &outcome, mode)
}

/// The reserved built-in template names (`default`, `bare-skill`). Anything else
/// is treated as a remote/custom source.
fn is_builtin_template(name: &str) -> bool {
    matches!(name, BUILTIN_DEFAULT | BUILTIN_BARE_SKILL)
}

/// Today's UTC date as `YYYY-MM-DD` (matching the substitution `${TOME_DATE}`
/// rendering, so scaffolded bodies read consistently).
fn today() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}",
        now.year(),
        u8::from(now.month()),
        now.day()
    )
}

/// Render the create outcome: a human summary or a `--json` created-files
/// record (a single object, like `lint`; distinct from `convert`'s JSONL).
fn emit_report(
    level: ArtifactLevel,
    final_name: &str,
    outcome: &EmitOutcome,
    mode: Mode,
) -> Result<(), TomeError> {
    match mode {
        Mode::Json => write_json(&create_json(level, final_name, outcome))?,
        Mode::Human => {
            println!(
                "Created {} `{}` at {}",
                level.as_str(),
                final_name,
                outcome.root.display()
            );
            for p in &outcome.written {
                println!("  {}", p.display());
            }
        }
    }
    Ok(())
}

/// The `--json` created-files record (a single object; key order/shape pinned
/// by `json_record_shape` below).
fn create_json(level: ArtifactLevel, final_name: &str, outcome: &EmitOutcome) -> serde_json::Value {
    json!({
        "level": level.as_str(),
        "name": final_name,
        "root": outcome.root.display().to_string(),
        "written": outcome
            .written
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(level: ArtifactLevel, name: &str) -> CreateRequest {
        CreateRequest {
            level,
            name: name.to_owned(),
            template: None,
            output: None,
            into: None,
            force: false,
            bare: false,
            plugin_name: None,
        }
    }

    fn scope() -> ResolvedScope {
        ResolvedScope::global_fallback()
    }

    #[test]
    fn is_builtin_template_recognises_the_reserved_names() {
        assert!(is_builtin_template("default"));
        assert!(is_builtin_template("bare-skill"));
        assert!(!is_builtin_template("acme/tome-template"));
        assert!(!is_builtin_template("https://example.com/t.git"));
    }

    #[test]
    fn a_non_builtin_template_is_template_invalid_82() {
        let tmp = tempfile::tempdir().unwrap();
        let mut r = req(ArtifactLevel::Plugin, "toolkit");
        r.output = Some(tmp.path().to_path_buf());
        r.template = Some("acme/tome-template".to_owned());
        let err = run(r, &scope(), Mode::Human).unwrap_err();
        assert_eq!(err.exit_code(), 82);
    }

    #[test]
    fn scaffolds_a_plugin_wrapped_skill_that_lands_on_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let mut r = req(ArtifactLevel::Skill, "review");
        r.output = Some(tmp.path().to_path_buf());
        run(r, &scope(), Mode::Human).unwrap();
        assert!(tmp.path().join("review/tome-plugin.toml").is_file());
        assert!(tmp.path().join("review/skills/review/SKILL.md").is_file());
    }

    #[test]
    fn bare_skill_emits_a_naked_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let mut r = req(ArtifactLevel::Skill, "review");
        r.output = Some(tmp.path().to_path_buf());
        r.bare = true;
        run(r, &scope(), Mode::Human).unwrap();
        assert!(tmp.path().join("review/SKILL.md").is_file());
        assert!(!tmp.path().join("review/tome-plugin.toml").exists());
    }

    #[test]
    fn template_bare_skill_is_an_alias_for_bare() {
        let tmp = tempfile::tempdir().unwrap();
        let mut r = req(ArtifactLevel::Skill, "review");
        r.output = Some(tmp.path().to_path_buf());
        r.template = Some("bare-skill".to_owned());
        run(r, &scope(), Mode::Human).unwrap();
        assert!(tmp.path().join("review/SKILL.md").is_file());
        assert!(!tmp.path().join("review/tome-plugin.toml").exists());
    }

    #[test]
    fn collision_without_force_is_output_exists_81() {
        let tmp = tempfile::tempdir().unwrap();
        let mut r = req(ArtifactLevel::Plugin, "toolkit");
        r.output = Some(tmp.path().to_path_buf());
        run(r, &scope(), Mode::Human).unwrap();
        // Re-create into the same dir without --force.
        let mut r2 = req(ArtifactLevel::Plugin, "toolkit");
        r2.output = Some(tmp.path().to_path_buf());
        let err = run(r2, &scope(), Mode::Human).unwrap_err();
        assert_eq!(err.exit_code(), 81);
    }

    #[test]
    fn skill_into_a_plugin_drops_a_naked_skill_under_skills() {
        let tmp = tempfile::tempdir().unwrap();
        // An existing plugin to inject into.
        let plug = tmp.path().join("toolkit");
        std::fs::create_dir_all(&plug).unwrap();
        std::fs::write(
            plug.join("tome-plugin.toml"),
            "name = \"toolkit\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();

        let manifest_before = "name = \"toolkit\"\nversion = \"1.0.0\"\n";
        let mut r = req(ArtifactLevel::Skill, "review");
        r.into = Some(plug.clone());
        run(r, &scope(), Mode::Human).unwrap();

        assert!(plug.join("skills/review/SKILL.md").is_file());
        // No nested plugin manifest was emitted for the injected skill.
        assert!(!plug.join("skills/review/tome-plugin.toml").exists());
        // The target plugin's manifest must NOT be edited — a skill is
        // discovered by directory, never registered (T-MINOR-6).
        assert_eq!(
            std::fs::read_to_string(plug.join("tome-plugin.toml")).unwrap(),
            manifest_before,
            "skill-into-plugin must not touch the plugin manifest"
        );
    }

    /// Build a catalog manifest with a leading hand-written comment.
    fn write_catalog_with_comment(cat: &std::path::Path) {
        std::fs::create_dir_all(cat).unwrap();
        std::fs::write(
            cat.join("tome-catalog.toml"),
            "# hand-written comment\nname = \"c\"\nversion = \"1.0.0\"\ndescription = \"d\"\n\n[owner]\nname = \"o\"\nemail = \"o@x.io\"\n",
        )
        .unwrap();
    }

    #[test]
    fn plugin_into_a_catalog_lands_and_registers() {
        let tmp = tempfile::tempdir().unwrap();
        let cat = tmp.path().join("cat");
        write_catalog_with_comment(&cat);

        let mut r = req(ArtifactLevel::Plugin, "toolkit");
        r.into = Some(cat.clone());
        run(r, &scope(), Mode::Human).unwrap();

        assert!(cat.join("toolkit/tome-plugin.toml").is_file());
        let manifest = std::fs::read_to_string(cat.join("tome-catalog.toml")).unwrap();
        assert!(manifest.contains("name = \"toolkit\""), "{manifest}");
    }

    #[test]
    fn plugin_into_catalog_preserves_comments_and_is_idempotent() {
        // T-MAJOR-2: the catalog edit preserves the hand-written comment, and a
        // second registration (force re-run) is a byte-identical no-op with a
        // single plugin entry.
        let tmp = tempfile::tempdir().unwrap();
        let cat = tmp.path().join("cat");
        write_catalog_with_comment(&cat);

        let mut r = req(ArtifactLevel::Plugin, "toolkit");
        r.into = Some(cat.clone());
        run(r, &scope(), Mode::Human).unwrap();

        let after_first = std::fs::read_to_string(cat.join("tome-catalog.toml")).unwrap();
        assert!(
            after_first.contains("# hand-written comment"),
            "comment must survive toml_edit: {after_first}"
        );

        // Re-run with --force (the plugin dir already exists) → the catalog edit
        // must be idempotent (no duplicate entry, byte-identical manifest).
        let mut r2 = req(ArtifactLevel::Plugin, "toolkit");
        r2.into = Some(cat.clone());
        r2.force = true;
        run(r2, &scope(), Mode::Human).unwrap();

        let after_second = std::fs::read_to_string(cat.join("tome-catalog.toml")).unwrap();
        assert_eq!(
            after_first, after_second,
            "second registration must be a byte-identical no-op"
        );
        assert_eq!(
            after_second.matches("name = \"toolkit\"").count(),
            1,
            "exactly one plugin entry"
        );
    }

    #[test]
    fn into_a_target_without_a_manifest_is_a_usage_error_2() {
        // T-MAJOR-4 / C-1: a bad --into target (no Tome manifest) is exit 2
        // (the contract was corrected from the earlier 3/27).
        let tmp = tempfile::tempdir().unwrap();
        let empty = tmp.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let mut r = req(ArtifactLevel::Plugin, "toolkit");
        r.into = Some(empty);
        let err = run(r, &scope(), Mode::Human).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn force_re_create_preserves_unrelated_user_files() {
        // T-MAJOR-3: --force overwrites only colliding files; a user file the
        // artifact does not contribute survives, through the command path.
        let tmp = tempfile::tempdir().unwrap();
        let mut r = req(ArtifactLevel::Plugin, "toolkit");
        r.output = Some(tmp.path().to_path_buf());
        run(r, &scope(), Mode::Human).unwrap();
        std::fs::write(tmp.path().join("toolkit/NOTES.md"), b"keep me").unwrap();

        let mut r2 = req(ArtifactLevel::Plugin, "toolkit");
        r2.output = Some(tmp.path().to_path_buf());
        r2.force = true;
        run(r2, &scope(), Mode::Human).unwrap();
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("toolkit/NOTES.md")).unwrap(),
            "keep me",
        );
    }

    #[test]
    fn template_default_selects_the_builtin() {
        // T-MINOR-5: `--template default` is the built-in selector, not a remote
        // fetch — it must succeed, not TemplateInvalid.
        let tmp = tempfile::tempdir().unwrap();
        let mut r = req(ArtifactLevel::Plugin, "toolkit");
        r.output = Some(tmp.path().to_path_buf());
        r.template = Some("default".to_owned());
        run(r, &scope(), Mode::Human).unwrap();
        assert!(tmp.path().join("toolkit/tome-plugin.toml").is_file());
    }

    #[test]
    fn json_record_has_level_name_root_and_written() {
        // T-MAJOR-1: pin the --json created-files record shape.
        use crate::authoring::emit::EmitOutcome;
        let outcome = EmitOutcome {
            root: std::path::PathBuf::from("/tmp/out/toolkit"),
            written: vec![
                std::path::PathBuf::from("tome-plugin.toml"),
                std::path::PathBuf::from("skills/toolkit/SKILL.md"),
            ],
        };
        let v = create_json(ArtifactLevel::Plugin, "toolkit", &outcome);
        assert_eq!(v["level"], "plugin");
        assert_eq!(v["name"], "toolkit");
        assert_eq!(v["root"], "/tmp/out/toolkit");
        assert_eq!(v["written"][0], "tome-plugin.toml");
        assert_eq!(v["written"][1], "skills/toolkit/SKILL.md");
    }

    #[test]
    fn clap_rejects_conflicting_flags_with_exit_2() {
        // C-2: clap `conflicts_with` for --template+--bare, --output+--into,
        // and --plugin-name+--bare → ArgumentConflict (exit 2).
        use clap::Parser;
        for argv in [
            vec!["tome", "skill", "create", "x", "--template", "t", "--bare"],
            vec![
                "tome", "skill", "create", "x", "--output", "o", "--into", "i",
            ],
            vec![
                "tome",
                "skill",
                "create",
                "x",
                "--plugin-name",
                "p",
                "--bare",
            ],
            vec![
                "tome",
                "skill",
                "create",
                "x",
                "--plugin-name",
                "p",
                "--into",
                "i",
            ],
        ] {
            let err = crate::cli::Cli::try_parse_from(argv.clone()).unwrap_err();
            assert_eq!(
                err.kind(),
                clap::error::ErrorKind::ArgumentConflict,
                "expected a conflict for {argv:?}"
            );
        }
    }
}
