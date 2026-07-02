//! Shared wrapper for `tome {catalog,plugin,skill} convert` — the single
//! arg→`authoring::convert`→report path, parameterized by the artifact level
//! each command surface fixes. Follows the silent-compute / emit split:
//! [`crate::authoring::convert::run`] does the I/O-bounded compute and returns a
//! structured outcome; this module renders it per output mode.
//!
//! `SOURCE` may be local or remote (`owner/repo` / git URL — shallow-cloned into
//! a temp dir, [`resolve_source`]). `--output` lands the copy at
//! `<output>/<name>/`; `--into` injects it into an existing Tome artifact
//! (a plugin into a catalog, registering it in `plugins[]`; a skill into a
//! plugin's `skills/`), auto-detected from the target's manifest
//! ([`into_target`]).

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::authoring::convert::{self, ConvertConfig, ConvertOutcome};
use crate::authoring::detect::ArtifactLevel;
use crate::catalog::git::{Git, scrub_to_string};
use crate::catalog::store::write_atomic;
use crate::cli::ConvertArgs;
use crate::error::TomeError;
use crate::output::{Mode, write_json};
use crate::util::{TOME_CONFIG_MAX, bounded_read_to_string};
use crate::workspace::ResolvedScope;

/// Run a conversion for the given artifact `level`.
///
/// `no_fetch` is passed explicitly by the caller: `catalog convert` threads its
/// `--no-fetch` flag; `plugin`/`skill convert` pass `false` (a single
/// plugin/skill conversion has no remote-plugin fan-out to skip). `fetch_remote`
/// is `!no_fetch`.
pub fn run(
    args: ConvertArgs,
    no_fetch: bool,
    _scope: &ResolvedScope,
    mode: Mode,
    level: ArtifactLevel,
) -> Result<(), TomeError> {
    // Resolve SOURCE to a local root: a local path is used in place; a remote
    // (`owner/repo`, git URL) is shallow-cloned into a temp dir whose `Drop`
    // guarantees cleanup on every exit path. `_clone` is held for the whole
    // conversion (NFR-003).
    let (source_root, _clone) = resolve_source(&args.source)?;

    // The new name is the positional `<NAME>` (or `None` → `<source-name>-tome`).
    let new_name = args.name.clone();

    // `--output` lands the copy at `<output>/<name>/`; `--into` injects it into
    // an existing Tome artifact (a plugin into a catalog, a skill into a
    // plugin), auto-detected from the target's manifest. The two are mutually
    // exclusive (clap-enforced). `register` names the catalog manifest a
    // plugin-into-catalog injection must register the new plugin in.
    let (output_dir, register) = match &args.into {
        Some(into) => into_target(into, level)?,
        None => (
            args.output.clone().unwrap_or_else(|| PathBuf::from(".")),
            None,
        ),
    };

    let cfg = ConvertConfig {
        level,
        from: args.from,
        new_name,
        strict: args.strict,
        allow: args.allow,
        force: args.force,
        dry_run: args.dry_run,
        fetch_remote: !no_fetch,
        output_dir,
    };

    // Run the convert compute into a Result; derive the telemetry outcome from
    // BOTH the verdict and the Result, emit ONE `tome.authoring_action` event,
    // then proceed with the original control flow unchanged (the emit is
    // infallible and side-effect-only).
    let convert_result = convert::run(&source_root, &cfg);
    emit_authoring_telemetry(level, &convert_result);

    let outcome = convert_result?;

    // Register the injected plugin in the target catalog's `plugins[]` (atomic,
    // comment-preserving, idempotent) — never under `--dry-run`.
    if let Some(catalog_manifest) = register
        && !outcome.dry_run
    {
        register_plugin_in_catalog(&catalog_manifest, &outcome.final_name)?;
    }

    emit_report(&outcome, mode)?;

    // Under `--dry-run --strict` the plan was reported above; now surface the
    // would-be non-zero verdict (a real strict run aborted inside `convert::run`
    // before any write, so this only fires for dry-run).
    if let Some(feature) = outcome.strict_blocked {
        return Err(TomeError::ConversionUnsupportedStrict { feature });
    }
    Ok(())
}

/// Map a detected [`SourceHarness`](crate::authoring::detect::SourceHarness) to
/// the closed telemetry [`SourceFormat`](crate::telemetry::event::SourceFormat).
/// The native-SKILL.md harnesses (Cursor / OpenCode / Cline / Agent-Skills) all
/// collapse to `NativeSkill`.
fn source_format_of(
    harness: crate::authoring::detect::SourceHarness,
) -> crate::telemetry::event::SourceFormat {
    use crate::authoring::detect::SourceHarness;
    use crate::telemetry::event::SourceFormat;
    match harness {
        SourceHarness::ClaudeCode => SourceFormat::ClaudeCode,
        SourceHarness::Codex => SourceFormat::Codex,
        SourceHarness::Cursor
        | SourceHarness::OpenCode
        | SourceHarness::Cline
        | SourceHarness::AgentSkills => SourceFormat::NativeSkill,
    }
}

/// Map an [`ArtifactLevel`] to the telemetry [`Artifact`](crate::telemetry::event::Artifact).
pub(crate) fn artifact_of(level: ArtifactLevel) -> crate::telemetry::event::Artifact {
    use crate::telemetry::event::Artifact;
    match level {
        ArtifactLevel::Catalog => Artifact::Catalog,
        ArtifactLevel::Plugin => Artifact::Plugin,
        ArtifactLevel::Skill => Artifact::Skill,
    }
}

/// Emit one `tome.authoring_action{verb=Convert}` event with the REAL outcome.
///
/// On `Ok`: a non-dry-run `strict_blocked` is impossible (a strict run aborts
/// before returning Ok), so the outcome is `Errors` when the report has errors,
/// `Warnings` when it has warnings, else `Ok`. On `Err`: a
/// `ConversionUnsupportedStrict` is `StrictRefused`; anything else is `Errors`.
/// `source_format` is the detected source on `Ok`, and `Unknown` when the
/// failure happened before/at detection (we have no outcome to read it from).
fn emit_authoring_telemetry(level: ArtifactLevel, result: &Result<ConvertOutcome, TomeError>) {
    use crate::telemetry::event::{
        AuthoringActionEvent, AuthoringOutcome, AuthoringVerb, SourceFormat,
    };
    let (source_format, outcome) = match result {
        Ok(o) => {
            let outcome = if o.strict_blocked.is_some() {
                AuthoringOutcome::StrictRefused
            } else if o.report.errors > 0 {
                AuthoringOutcome::Errors
            } else if o.report.warnings > 0 {
                AuthoringOutcome::Warnings
            } else {
                AuthoringOutcome::Ok
            };
            (source_format_of(o.harness), outcome)
        }
        Err(TomeError::ConversionUnsupportedStrict { .. }) => {
            (SourceFormat::Unknown, AuthoringOutcome::StrictRefused)
        }
        // best-effort: a failure before detection has no source format to read.
        Err(_) => (SourceFormat::Unknown, AuthoringOutcome::Errors),
    };
    crate::telemetry::emit(AuthoringActionEvent {
        verb: AuthoringVerb::Convert,
        artifact: artifact_of(level),
        source_format,
        outcome,
    });
}

/// Resolve an `--into <DIR>` target to `(output_dir, register_in_catalog)`:
/// auto-detect the target artifact from its manifest and pick where the
/// converted copy lands. A plugin injected into a catalog lands at
/// `<catalog>/<name>/` and registers in the catalog manifest; a skill injected
/// into a plugin lands at `<plugin>/skills/<name>/` (no manifest edit — skills
/// are discovered by directory).
///
/// Shared with `create` (the SSOT for `--into` target detection).
pub(crate) fn into_target(
    into: &Path,
    level: ArtifactLevel,
) -> Result<(PathBuf, Option<PathBuf>), TomeError> {
    let catalog_manifest = into.join("tome-catalog.toml");
    let plugin_manifest = into.join("tome-plugin.toml");

    if catalog_manifest.is_file() {
        match level {
            ArtifactLevel::Plugin => Ok((into.to_path_buf(), Some(catalog_manifest))),
            ArtifactLevel::Skill => Err(TomeError::Usage(
                "`skill convert --into` targets a plugin, but a catalog was found".to_owned(),
            )),
            ArtifactLevel::Catalog => Err(TomeError::Usage(
                "a catalog cannot be injected into another catalog".to_owned(),
            )),
        }
    } else if plugin_manifest.is_file() {
        match level {
            ArtifactLevel::Skill => Ok((into.join("skills"), None)),
            ArtifactLevel::Plugin => Err(TomeError::Usage(
                "`plugin convert --into` targets a catalog, but a plugin was found".to_owned(),
            )),
            ArtifactLevel::Catalog => Err(TomeError::Usage(
                "a catalog cannot be injected into a plugin".to_owned(),
            )),
        }
    } else {
        Err(TomeError::Usage(format!(
            "no Tome artifact (tome-catalog.toml / tome-plugin.toml) found at --into target `{}`",
            into.display()
        )))
    }
}

/// Register `plugin_name` in a catalog manifest's `plugins[]` array-of-tables
/// via a comment/format-preserving `toml_edit` edit, landed atomically
/// (`write_atomic`; NFR-011). Idempotent — a plugin already present is a no-op.
///
/// Shared with `create` (the SSOT for catalog `plugins[]` registration).
pub(crate) fn register_plugin_in_catalog(
    manifest_path: &Path,
    plugin_name: &str,
) -> Result<(), TomeError> {
    let body = bounded_read_to_string(manifest_path, TOME_CONFIG_MAX)?;
    let mut doc: toml_edit::DocumentMut = body.parse().map_err(|e| {
        TomeError::Usage(format!(
            "catalog manifest {} is not valid TOML: {e}",
            manifest_path.display()
        ))
    })?;

    if doc.get("plugins").is_none() {
        doc["plugins"] = toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
    }
    let plugins = doc["plugins"].as_array_of_tables_mut().ok_or_else(|| {
        TomeError::Usage(format!(
            "catalog manifest {} `plugins` is not an array of tables; register the plugin manually",
            manifest_path.display()
        ))
    })?;

    let already = plugins
        .iter()
        .any(|t| t.get("name").and_then(|v| v.as_str()) == Some(plugin_name));
    if already {
        return Ok(());
    }

    let mut entry = toml_edit::Table::new();
    entry["name"] = toml_edit::value(plugin_name);
    entry["source"] = toml_edit::value(plugin_name);
    plugins.push(entry);

    write_atomic(manifest_path, doc.to_string().as_bytes())
}

/// Resolve a `SOURCE` to a local directory root. **An existing local path wins**
/// (returned as-is, no temp clone); only a `SOURCE` that does not exist on disk
/// is treated as remote (`owner/repo`, git URL) and shallow-cloned into a
/// [`tempfile::TempDir`] whose `Drop` cleans up on success, conversion error,
/// `--strict` abort, and SIGINT-unwind (NFR-003). The local-wins precedence
/// means a CWD-relative directory named like an `owner/repo` shorthand shadows
/// the remote — pass an explicit URL to force a clone. Credentials are scrubbed
/// from the display source and every `git` error chain (`catalog::git`).
fn resolve_source(source: &str) -> Result<(PathBuf, Option<tempfile::TempDir>), TomeError> {
    let path = Path::new(source);
    if path.exists() {
        return Ok((path.to_path_buf(), None));
    }
    let url = crate::commands::catalog::source::resolve(source)?;
    let tempdir = tempfile::Builder::new()
        .prefix("tome-convert-")
        .tempdir()
        .map_err(TomeError::Io)?;
    let clone_dest = tempdir.path().join("repo");
    let git = Git::new(scrub_to_string(url.as_bytes()));
    git.clone_shallow(&url, &clone_dest, None)?;
    Ok((clone_dest, Some(tempdir)))
}

/// Render the conversion outcome: a human summary, or a JSONL action stream
/// (one diagnostic per line + a final `result` line) under `--json` (FR-015).
fn emit_report(outcome: &ConvertOutcome, mode: Mode) -> Result<(), TomeError> {
    match mode {
        Mode::Json => {
            for d in &outcome.report.diagnostics {
                write_json(&convert_diagnostic_json(d))?;
            }
            write_json(&convert_result_json(outcome))?;
        }
        Mode::Human => {
            let verb = if outcome.dry_run {
                "Would convert"
            } else {
                "Converted"
            };
            println!(
                "{verb} {} {} `{}` → `{}`",
                outcome.harness.as_str(),
                outcome.level.as_str(),
                outcome.source_name,
                outcome.final_name
            );
            for d in &outcome.report.diagnostics {
                println!("  [{}] {}: {}", d.severity.as_str(), d.rule_id, d.message);
            }
            println!(
                "{} {} file(s) to {}  ({} warning(s), {} info(s))",
                if outcome.dry_run { "Dry run:" } else { "Done:" },
                outcome.written.len(),
                outcome.target.display(),
                outcome.report.warnings,
                outcome.report.infos,
            );
            if let Some(feature) = &outcome.strict_blocked {
                println!("STRICT: would abort — {feature}");
            }
            // A "what next" bridge into the iteration loop after a real convert
            // that surfaced warnings — mirrors the `hint:` remediation the
            // `PluginNotConverted` error already models (FR #298). Human-only;
            // never in `--json`. Gated to SUCCESS (nothing under `--dry-run`, so
            // the lint target actually exists) WITH warnings (a clean convert
            // needs no lint nudge).
            if let Some(bridge) = next_bridge(outcome) {
                print!("{bridge}");
            }
        }
    }
    Ok(())
}

/// The post-convert "what next" bridge (human mode). Returns `Some(text)` only
/// when the convert (a) wrote files (`!dry_run`) and (b) surfaced at least one
/// warning; otherwise `None` (a `--dry-run` wrote nothing, a clean convert needs
/// no lint nudge). The text points into the iteration loop with real,
/// copy-pasteable commands: `tome <level> lint <target> --autofix` — where
/// `<level>` is the convert level and `<target>` is the path convert just wrote
/// — then `tome harness use <harness>` to activate it (`<harness>` is a
/// placeholder the user fills). Each printed line is newline-terminated.
fn next_bridge(outcome: &ConvertOutcome) -> Option<String> {
    if outcome.dry_run || outcome.report.warnings == 0 {
        return None;
    }
    Some(format!(
        "Next: run `tome {} lint {} --autofix`, then `tome harness use <harness>`\n",
        outcome.level.as_str(),
        outcome.target.display(),
    ))
}

/// One `--json` JSONL diagnostic line: the `type: "diagnostic"` discriminator
/// plus the shared [`crate::authoring::ir::finding_json`] finding fields
/// (`rule`/`severity`/`message`/`file`/`line`/`autofixable`) — byte-identical
/// to a `lint --json` `findings[]` entry, so a caller can parse either stream's
/// findings the same way (issue #299). The JSONL envelope is preserved: this
/// stays one line per diagnostic followed by the trailing `type: "result"`
/// line, never lint's single-object shape.
fn convert_diagnostic_json(d: &crate::authoring::ir::Diagnostic) -> serde_json::Value {
    let mut value = crate::authoring::ir::finding_json(d);
    // Prepend the JSONL discriminator. `finding_json` returns a JSON object, so
    // the `as_object_mut` unwrap is infallible; the map preserves insertion
    // order (`serde_json` `preserve_order`) with `type` appended last.
    if let Some(obj) = value.as_object_mut() {
        obj.insert("type".to_owned(), json!("diagnostic"));
    }
    value
}

/// The final `--json` JSONL `result` line. Shape pinned by
/// `convert_result_json_shape_is_pinned`.
fn convert_result_json(outcome: &ConvertOutcome) -> serde_json::Value {
    json!({
        "type": "result",
        "harness": outcome.harness.as_str(),
        "level": outcome.level.as_str(),
        "source_name": outcome.source_name,
        "final_name": outcome.final_name,
        "target": outcome.target.display().to_string(),
        "dry_run": outcome.dry_run,
        "written": outcome.written.len(),
        "errors": outcome.report.errors,
        "warnings": outcome.report.warnings,
        "infos": outcome.report.infos,
        "strict_blocked": outcome.strict_blocked,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    /// Build a minimal [`ConvertOutcome`] with the given `warnings` and `dry_run`
    /// for exercising the human-mode "what next" bridge gating.
    fn outcome_with(level: ArtifactLevel, warnings: usize, dry_run: bool) -> ConvertOutcome {
        let report = crate::authoring::lint::LintReport {
            warnings,
            ..Default::default()
        };
        ConvertOutcome {
            harness: crate::authoring::detect::SourceHarness::ClaudeCode,
            level,
            source_name: "demo".to_owned(),
            final_name: "demo-tome".to_owned(),
            target: PathBuf::from("/out/demo-tome"),
            report,
            written: vec![PathBuf::from("tome-plugin.toml")],
            dry_run,
            strict_blocked: None,
        }
    }

    #[test]
    fn next_bridge_names_real_commands_after_a_warning_convert() {
        // Success (not dry-run) + warnings ⇒ the full bridge, with the convert
        // level and the actual output target substituted into copy-pasteable
        // `lint --autofix` + `harness use` commands.
        let outcome = outcome_with(ArtifactLevel::Plugin, 2, false);
        let bridge = next_bridge(&outcome).expect("a warning convert emits the bridge");
        assert_eq!(
            bridge,
            "Next: run `tome plugin lint /out/demo-tome --autofix`, \
             then `tome harness use <harness>`\n"
        );
        // The level matches the convert level, not a hardcoded one.
        let cat = next_bridge(&outcome_with(ArtifactLevel::Catalog, 1, false)).unwrap();
        assert!(cat.starts_with("Next: run `tome catalog lint "), "{cat}");
        let skill = next_bridge(&outcome_with(ArtifactLevel::Skill, 1, false)).unwrap();
        assert!(skill.starts_with("Next: run `tome skill lint "), "{skill}");
    }

    #[test]
    fn next_bridge_is_suppressed_on_dry_run() {
        // A `--dry-run` writes nothing, so the lint target does not exist yet —
        // no bridge, even with warnings.
        assert!(next_bridge(&outcome_with(ArtifactLevel::Plugin, 3, true)).is_none());
    }

    #[test]
    fn next_bridge_is_suppressed_without_warnings() {
        // A clean convert (zero diagnostics) needs no lint nudge (issue #298:
        // warnings → the bridge).
        assert!(next_bridge(&outcome_with(ArtifactLevel::Plugin, 0, false)).is_none());
    }

    #[test]
    fn convert_result_json_shape_is_pinned() {
        // T-MAJOR-2 (phase-wide): pin the `--json` JSONL `result` line shape, the
        // richest of the three verbs' wire shapes (a `jq` consumer scripts it).
        let outcome = ConvertOutcome {
            harness: crate::authoring::detect::SourceHarness::ClaudeCode,
            level: ArtifactLevel::Plugin,
            source_name: "demo".to_owned(),
            final_name: "demo-tome".to_owned(),
            target: PathBuf::from("/out/demo-tome"),
            report: crate::authoring::lint::LintReport::default(),
            written: vec![PathBuf::from("tome-plugin.toml")],
            dry_run: false,
            strict_blocked: None,
        };
        let v = convert_result_json(&outcome);
        assert_eq!(v["type"], "result");
        assert_eq!(v["harness"], "claude-code");
        assert_eq!(v["level"], "plugin");
        assert_eq!(v["source_name"], "demo");
        assert_eq!(v["final_name"], "demo-tome");
        assert_eq!(v["target"], "/out/demo-tome");
        assert_eq!(v["dry_run"], false);
        assert_eq!(v["written"], 1);
        assert_eq!(v["errors"], 0);
        assert_eq!(v["warnings"], 0);
        assert_eq!(v["infos"], 0);
        assert!(v["strict_blocked"].is_null());

        // A diagnostic line carries its own `type` discriminator PLUS the shared
        // finding fields (`file`/`line`/`autofixable`) enriching it to parity
        // with `lint --json`'s finding shape (issue #299). Absent location ⇒
        // JSON null for `file`/`line`, mirroring lint exactly.
        let d = crate::authoring::ir::Diagnostic::warning("convert/x", "boom");
        let dj = convert_diagnostic_json(&d);
        assert_eq!(dj["type"], "diagnostic");
        assert_eq!(dj["severity"], "warning");
        assert_eq!(dj["rule"], "convert/x");
        assert_eq!(dj["message"], "boom");
        assert!(dj["file"].is_null(), "no location ⇒ file is null: {dj}");
        assert!(dj["line"].is_null(), "no location ⇒ line is null: {dj}");
        assert_eq!(dj["autofixable"], false);
    }

    #[test]
    fn convert_diagnostic_line_matches_lint_finding_fields() {
        // Cross-verb parity (issue #299): for the SAME diagnostic, convert's
        // JSONL diagnostic line and lint's `findings[]` entry carry byte-
        // identical `rule`/`severity`/`message`/`file`/`line`/`autofixable`.
        // convert adds ONLY the `type: "diagnostic"` discriminator on top.
        use crate::authoring::ir::{Diagnostic, Fix, Location, finding_json};

        // Exercise both the located+autofixable case AND the absent-location
        // case (JSON null derivation must agree between the two verbs).
        let located = Diagnostic::warning("convert/y", "needs a fix")
            .at(Location {
                file: std::path::PathBuf::from("skills/demo/SKILL.md"),
                line: Some(7),
            })
            .with_fix(Fix {
                path: std::path::PathBuf::from("skills/demo/SKILL.md"),
                replacement: "fixed".to_owned(),
            });
        let unlocated = Diagnostic::info("convert/z", "no location here");

        for d in [&located, &unlocated] {
            let lint_finding = finding_json(d);
            let convert_line = convert_diagnostic_json(d);
            // The convert line differs ONLY by the extra `type` key.
            assert_eq!(convert_line["type"], "diagnostic");
            for key in ["rule", "severity", "message", "file", "line", "autofixable"] {
                assert_eq!(
                    convert_line[key], lint_finding[key],
                    "field `{key}` must byte-match lint's finding for {d:?}"
                );
            }
        }
        // Spot-check the enriched values on the located diagnostic.
        let dj = convert_diagnostic_json(&located);
        assert_eq!(dj["file"], "skills/demo/SKILL.md");
        assert_eq!(dj["line"], 7);
        assert_eq!(dj["autofixable"], true);
    }

    /// Run a git subcommand in `dir`, asserting success (identity injected so CI
    /// never prompts).
    fn git(args: &[&str], dir: &Path) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "Tome Test")
            .env("GIT_AUTHOR_EMAIL", "tests@tome.invalid")
            .env("GIT_COMMITTER_NAME", "Tome Test")
            .env("GIT_COMMITTER_EMAIL", "tests@tome.invalid")
            .status()
            .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
        assert!(status.success(), "git {args:?} exited {status}");
    }

    #[test]
    fn resolve_source_returns_a_local_path_without_cloning() {
        let tmp = tempfile::tempdir().unwrap();
        let (root, guard) = resolve_source(tmp.path().to_str().unwrap()).unwrap();
        assert_eq!(root, tmp.path());
        assert!(guard.is_none(), "a local path is not cloned");
    }

    #[test]
    fn resolve_source_clones_a_remote_into_a_cleaned_up_tempdir() {
        // A real local git repo, addressed by a `file://` URL so the path does
        // not exist literally and the remote branch is taken.
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir(&repo).unwrap();
        fs::write(
            repo.join("SKILL.md"),
            "---\nname: s\ndescription: d\n---\nbody\n",
        )
        .unwrap();
        git(&["init", "-q", "-b", "main"], &repo);
        git(&["add", "-A"], &repo);
        git(&["commit", "-q", "-m", "init"], &repo);

        let url = format!("file://{}", repo.display());
        let cloned_path;
        {
            let (root, guard) = resolve_source(&url).unwrap();
            assert!(guard.is_some(), "a remote source clones into a tempdir");
            assert!(root.join("SKILL.md").exists(), "cloned content is present");
            cloned_path = guard.as_ref().unwrap().path().to_path_buf();
            // `guard` drops at the end of this scope.
        }
        assert!(
            !cloned_path.exists(),
            "the temp clone is cleaned up on drop"
        );
    }

    fn write_catalog_manifest(dir: &Path) {
        fs::write(
            dir.join("tome-catalog.toml"),
            "name = \"c\"\nversion = \"0.0.0\"\ndescription = \"d\"\n\n[owner]\nname = \"o\"\nemail = \"o@x.io\"\n",
        )
        .unwrap();
    }

    #[test]
    fn into_target_routes_plugin_to_catalog_and_skill_to_plugin() {
        let tmp = tempfile::tempdir().unwrap();
        let cat = tmp.path().join("cat");
        fs::create_dir(&cat).unwrap();
        write_catalog_manifest(&cat);
        let (out, register) = into_target(&cat, ArtifactLevel::Plugin).unwrap();
        assert_eq!(out, cat);
        assert_eq!(register, Some(cat.join("tome-catalog.toml")));

        let plug = tmp.path().join("plug");
        fs::create_dir(&plug).unwrap();
        fs::write(
            plug.join("tome-plugin.toml"),
            "name = \"p\"\nversion = \"0.0.0\"\n",
        )
        .unwrap();
        let (out, register) = into_target(&plug, ArtifactLevel::Skill).unwrap();
        assert_eq!(out, plug.join("skills"));
        assert!(register.is_none());
    }

    #[test]
    fn into_target_rejects_mismatch_and_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let cat = tmp.path().join("cat");
        fs::create_dir(&cat).unwrap();
        write_catalog_manifest(&cat);
        // A skill cannot be injected into a catalog.
        assert_eq!(
            into_target(&cat, ArtifactLevel::Skill)
                .unwrap_err()
                .exit_code(),
            2
        );
        // A directory with no Tome manifest is not a valid --into target.
        let empty = tmp.path().join("empty");
        fs::create_dir(&empty).unwrap();
        assert_eq!(
            into_target(&empty, ArtifactLevel::Plugin)
                .unwrap_err()
                .exit_code(),
            2
        );
    }

    #[test]
    fn register_plugin_in_catalog_appends_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest = tmp.path().join("tome-catalog.toml");
        write_catalog_manifest(tmp.path());

        register_plugin_in_catalog(&manifest, "alpha").unwrap();
        let body = fs::read_to_string(&manifest).unwrap();
        assert!(body.contains("[[plugins]]"), "{body}");
        assert!(body.contains("name = \"alpha\""), "{body}");
        assert!(body.contains("source = \"alpha\""), "{body}");

        // A second registration of the same plugin is a no-op (byte-identical).
        register_plugin_in_catalog(&manifest, "alpha").unwrap();
        assert_eq!(fs::read_to_string(&manifest).unwrap(), body);
    }

    #[test]
    fn into_a_catalog_lands_the_plugin_and_registers_it() {
        // The full `--into` composition (as `run` performs it): detect the
        // target, convert into it, register in the catalog manifest.
        let tmp = tempfile::tempdir().unwrap();
        let cat = tmp.path().join("cat");
        fs::create_dir(&cat).unwrap();
        write_catalog_manifest(&cat);

        let src = tmp.path().join("src");
        fs::create_dir_all(src.join(".claude-plugin")).unwrap();
        fs::write(
            src.join(".claude-plugin/plugin.json"),
            br#"{"name":"alpha","version":"1.0.0"}"#,
        )
        .unwrap();

        let (output_dir, register) = into_target(&cat, ArtifactLevel::Plugin).unwrap();
        let cfg = crate::authoring::convert::ConvertConfig {
            level: ArtifactLevel::Plugin,
            from: None,
            new_name: None,
            strict: false,
            allow: Vec::new(),
            force: false,
            dry_run: false,
            fetch_remote: true,
            output_dir,
        };
        let outcome = crate::authoring::convert::run(&src, &cfg).unwrap();
        if let Some(manifest) = register {
            register_plugin_in_catalog(&manifest, &outcome.final_name).unwrap();
        }

        assert!(
            cat.join("alpha-tome/tome-plugin.toml").exists(),
            "plugin landed under the catalog"
        );
        let manifest = fs::read_to_string(cat.join("tome-catalog.toml")).unwrap();
        assert!(manifest.contains("name = \"alpha-tome\""), "{manifest}");
    }
}
