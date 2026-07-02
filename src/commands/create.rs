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
    /// `--description`; `None` → the scaffold's name-derived default.
    pub description: Option<String>,
    /// `--author`; `None` → the placeholder catalog owner / no plugin author.
    pub author: Option<String>,
    /// `--dry-run`: compute + report the plan without writing to disk.
    pub dry_run: bool,
}

/// Scaffold a new artifact at the request's level.
pub fn run(req: CreateRequest, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let level = req.level;
    let result = run_inner(req, scope, mode);

    // One `tome.authoring_action{verb=Create}` emit with the REAL outcome.
    // `create` is lint-clean by construction (no report), so the outcome is
    // `Ok` on success and `Errors` on any failure. source_format is `Unknown`:
    // a scaffold has no foreign source.
    use crate::telemetry::event::{
        AuthoringActionEvent, AuthoringOutcome, AuthoringVerb, SourceFormat,
    };
    let outcome = if result.is_ok() {
        AuthoringOutcome::Ok
    } else {
        AuthoringOutcome::Errors
    };
    crate::telemetry::emit(AuthoringActionEvent {
        verb: AuthoringVerb::Create,
        artifact: crate::commands::convert::artifact_of(level),
        source_format: SourceFormat::Unknown,
        outcome,
    });

    result
}

fn run_inner(req: CreateRequest, _scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
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

    // `--description`/`--author` flow into the scaffold; omitted → the scaffold
    // falls back to its name-derived description + placeholder catalog owner
    // (byte-identical to the pre-flag behaviour).
    let params = CreateParams {
        name: req.name.clone(),
        plugin_name: req.plugin_name.clone(),
        description: req.description.clone(),
        author_name: req.author.clone(),
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
            dry_run: req.dry_run,
        },
    )?;

    // Register the injected plugin in the target catalog's `plugins[]` (atomic,
    // comment-preserving, idempotent). Skipped under `--dry-run`: registration
    // is a filesystem write, and dry-run must not touch disk.
    if let Some(catalog_manifest) = register
        && !req.dry_run
    {
        register_plugin_in_catalog(&catalog_manifest, &final_name)?;
    }

    emit_report(req.level, &final_name, &outcome, req.dry_run, mode)
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
/// Under `dry_run` the record reports the files that *would* be written and the
/// human summary says so.
fn emit_report(
    level: ArtifactLevel,
    final_name: &str,
    outcome: &EmitOutcome,
    dry_run: bool,
    mode: Mode,
) -> Result<(), TomeError> {
    match mode {
        Mode::Json => write_json(&create_json(level, final_name, outcome, dry_run))?,
        Mode::Human => {
            if dry_run {
                println!(
                    "Would create {} `{}` at {}",
                    level.as_str(),
                    final_name,
                    outcome.root.display()
                );
            } else {
                println!(
                    "Created {} `{}` at {}",
                    level.as_str(),
                    final_name,
                    outcome.root.display()
                );
            }
            for p in &outcome.written {
                println!("  {}", p.display());
            }
        }
    }
    Ok(())
}

/// The `--json` created-files record (a single object; key order/shape pinned
/// by `json_record_shape` below). Under `--dry-run` the `written` list carries
/// the files that *would* be written and `dry_run` is `true`.
fn create_json(
    level: ArtifactLevel,
    final_name: &str,
    outcome: &EmitOutcome,
    dry_run: bool,
) -> serde_json::Value {
    json!({
        "level": level.as_str(),
        "name": final_name,
        "root": outcome.root.display().to_string(),
        "written": outcome
            .written
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>(),
        "dry_run": dry_run,
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
            description: None,
            author: None,
            dry_run: false,
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
        let v = create_json(ArtifactLevel::Plugin, "toolkit", &outcome, false);
        assert_eq!(v["level"], "plugin");
        assert_eq!(v["name"], "toolkit");
        assert_eq!(v["root"], "/tmp/out/toolkit");
        assert_eq!(v["written"][0], "tome-plugin.toml");
        assert_eq!(v["written"][1], "skills/toolkit/SKILL.md");
        assert_eq!(v["dry_run"], false);
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

    #[test]
    fn description_and_author_land_in_the_emitted_plugin() {
        // #325 (a): --description + --author reach the emitted artifact — the
        // manifest carries the author, the SKILL.md body carries the
        // description, and neither shows the name-derived / placeholder default.
        let tmp = tempfile::tempdir().unwrap();
        let mut r = req(ArtifactLevel::Plugin, "toolkit");
        r.output = Some(tmp.path().to_path_buf());
        r.description = Some("QA helpers".to_owned());
        r.author = Some("Acme".to_owned());
        run(r, &scope(), Mode::Human).unwrap();

        let manifest =
            std::fs::read_to_string(tmp.path().join("toolkit/tome-plugin.toml")).unwrap();
        assert!(
            manifest.contains("description = \"QA helpers\""),
            "manifest must carry --description: {manifest}"
        );
        assert!(
            manifest.contains("name = \"Acme\""),
            "manifest must carry --author in [author]: {manifest}"
        );
        // The name-derived description default must NOT appear.
        assert!(!manifest.contains("The toolkit scaffold."), "{manifest}");

        let skill =
            std::fs::read_to_string(tmp.path().join("toolkit/skills/toolkit/SKILL.md")).unwrap();
        assert!(
            skill.contains("QA helpers"),
            "skill body must carry --description: {skill}"
        );
    }

    #[test]
    fn author_sets_the_catalog_owner_replacing_the_placeholder() {
        // #325: --author replaces the `Your Name` catalog-owner placeholder.
        let tmp = tempfile::tempdir().unwrap();
        let mut r = req(ArtifactLevel::Catalog, "my-catalog");
        r.output = Some(tmp.path().to_path_buf());
        r.author = Some("Acme".to_owned());
        run(r, &scope(), Mode::Human).unwrap();

        let manifest =
            std::fs::read_to_string(tmp.path().join("my-catalog/tome-catalog.toml")).unwrap();
        assert!(
            manifest.contains("name = \"Acme\""),
            "catalog owner must be the --author value: {manifest}"
        );
        assert!(
            !manifest.contains("Your Name"),
            "placeholder must be replaced: {manifest}"
        );
    }

    #[test]
    fn dry_run_writes_nothing_and_reports_the_plan() {
        // #325 (b): --dry-run must not create the target on disk, yet still
        // reports the planned files (JSON `written` populated, `dry_run: true`).
        let tmp = tempfile::tempdir().unwrap();
        let mut r = req(ArtifactLevel::Plugin, "toolkit");
        r.output = Some(tmp.path().to_path_buf());
        r.dry_run = true;
        run(r, &scope(), Mode::Human).unwrap();
        assert!(
            !tmp.path().join("toolkit").exists(),
            "--dry-run must not write the artifact to disk"
        );
    }

    #[test]
    fn dry_run_into_catalog_does_not_register_or_write() {
        // #325 (b): --dry-run with --into must neither land the plugin nor edit
        // the catalog manifest (both are filesystem writes).
        let tmp = tempfile::tempdir().unwrap();
        let cat = tmp.path().join("cat");
        write_catalog_with_comment(&cat);
        let manifest_before = std::fs::read_to_string(cat.join("tome-catalog.toml")).unwrap();

        let mut r = req(ArtifactLevel::Plugin, "toolkit");
        r.into = Some(cat.clone());
        r.dry_run = true;
        run(r, &scope(), Mode::Human).unwrap();

        assert!(
            !cat.join("toolkit").exists(),
            "--dry-run must not land the plugin"
        );
        assert_eq!(
            std::fs::read_to_string(cat.join("tome-catalog.toml")).unwrap(),
            manifest_before,
            "--dry-run must not register the plugin in the catalog manifest"
        );
    }

    #[test]
    fn omitting_the_flags_reproduces_the_placeholder_and_default() {
        // #325 (c): back-compat — no --description/--author/--dry-run reproduces
        // the name-derived description default + the `Your Name` catalog
        // placeholder, and writes to disk as before.
        let tmp = tempfile::tempdir().unwrap();
        let mut r = req(ArtifactLevel::Catalog, "my-catalog");
        r.output = Some(tmp.path().to_path_buf());
        run(r, &scope(), Mode::Human).unwrap();

        let manifest =
            std::fs::read_to_string(tmp.path().join("my-catalog/tome-catalog.toml")).unwrap();
        assert!(
            manifest.contains("name = \"Your Name\""),
            "omitted --author must keep the placeholder owner: {manifest}"
        );
        assert!(
            manifest.contains("The my-catalog scaffold."),
            "omitted --description must keep the name-derived default: {manifest}"
        );
    }

    #[test]
    fn dry_run_json_records_dry_run_true() {
        // #325 (b): the --json record flags dry_run.
        use crate::authoring::emit::EmitOutcome;
        let outcome = EmitOutcome {
            root: std::path::PathBuf::from("/tmp/out/toolkit"),
            written: vec![std::path::PathBuf::from("tome-plugin.toml")],
        };
        let v = create_json(ArtifactLevel::Plugin, "toolkit", &outcome, true);
        assert_eq!(v["dry_run"], true);
        assert_eq!(v["written"][0], "tome-plugin.toml");
    }

    #[test]
    fn clap_parses_the_new_flags_on_all_three_verbs() {
        // #325: --description / --author / --dry-run parse on catalog, plugin,
        // and skill create.
        use crate::cli::{CatalogCommand, Cli, Command, PluginCommand, SkillCommand};
        use clap::Parser;

        let cli = Cli::try_parse_from([
            "tome",
            "catalog",
            "create",
            "cat",
            "--description",
            "d",
            "--author",
            "a",
            "--dry-run",
        ])
        .unwrap();
        match cli.command {
            Command::Catalog(CatalogCommand::Create(args)) => {
                assert_eq!(args.description.as_deref(), Some("d"));
                assert_eq!(args.author.as_deref(), Some("a"));
                assert!(args.dry_run);
            }
            other => panic!("expected catalog create, got {other:?}"),
        }

        let cli = Cli::try_parse_from([
            "tome",
            "plugin",
            "create",
            "qa",
            "--description",
            "QA helpers",
            "--author",
            "Acme",
            "--dry-run",
        ])
        .unwrap();
        match cli.command {
            Command::Plugin(pa) => match pa.command {
                Some(PluginCommand::Create(args)) => {
                    assert_eq!(args.description.as_deref(), Some("QA helpers"));
                    assert_eq!(args.author.as_deref(), Some("Acme"));
                    assert!(args.dry_run);
                }
                other => panic!("expected plugin create, got {other:?}"),
            },
            other => panic!("expected plugin, got {other:?}"),
        }

        let cli = Cli::try_parse_from([
            "tome",
            "skill",
            "create",
            "review",
            "--description",
            "d",
            "--author",
            "a",
            "--dry-run",
        ])
        .unwrap();
        match cli.command {
            Command::Skill(SkillCommand::Create(args)) => {
                assert_eq!(args.description.as_deref(), Some("d"));
                assert_eq!(args.author.as_deref(), Some("a"));
                assert!(args.dry_run);
            }
            other => panic!("expected skill create, got {other:?}"),
        }
    }
}
