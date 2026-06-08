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
pub fn run(
    args: ConvertArgs,
    _scope: &ResolvedScope,
    mode: Mode,
    level: ArtifactLevel,
) -> Result<(), TomeError> {
    // Resolve SOURCE to a local root: a local path is used in place; a remote
    // (`owner/repo`, git URL) is shallow-cloned into a temp dir whose `Drop`
    // guarantees cleanup on every exit path. `_clone` is held for the whole
    // conversion (NFR-003).
    let (source_root, _clone) = resolve_source(&args.source)?;

    let new_name =
        convert::resolve_requested_name(args.name.as_deref(), args.name_flag.as_deref())?;

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
        force: args.force,
        dry_run: args.dry_run,
        output_dir,
    };

    let outcome = convert::run(&source_root, &cfg)?;

    // Register the injected plugin in the target catalog's `plugins[]` (atomic,
    // comment-preserving, idempotent) — never under `--dry-run`.
    if let Some(catalog_manifest) = register
        && !outcome.dry_run
    {
        register_plugin_in_catalog(&catalog_manifest, &outcome.final_name)?;
    }

    emit_report(&outcome, mode)
}

/// Resolve an `--into <DIR>` target to `(output_dir, register_in_catalog)`:
/// auto-detect the target artifact from its manifest and pick where the
/// converted copy lands. A plugin injected into a catalog lands at
/// `<catalog>/<name>/` and registers in the catalog manifest; a skill injected
/// into a plugin lands at `<plugin>/skills/<name>/` (no manifest edit — skills
/// are discovered by directory).
fn into_target(into: &Path, level: ArtifactLevel) -> Result<(PathBuf, Option<PathBuf>), TomeError> {
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
fn register_plugin_in_catalog(manifest_path: &Path, plugin_name: &str) -> Result<(), TomeError> {
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

/// Resolve a `SOURCE` to a local directory root. A local path is returned as-is
/// (no temp clone); a remote `SOURCE` (`owner/repo`, git URL) is shallow-cloned
/// into a [`tempfile::TempDir`] whose `Drop` cleans up on success, conversion
/// error, `--strict` abort, and SIGINT-unwind (NFR-003). Credentials are
/// scrubbed from the display source and every `git` error chain (`catalog::git`).
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
                write_json(&json!({
                    "type": "diagnostic",
                    "severity": d.severity.as_str(),
                    "rule": d.rule_id,
                    "message": d.message,
                }))?;
            }
            write_json(&json!({
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
            }))?;
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
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

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
}
