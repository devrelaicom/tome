//! Shared wrapper for `tome {catalog,plugin,skill} convert` — the single
//! arg→`authoring::convert`→report path, parameterized by the artifact level
//! each command surface fixes. Follows the silent-compute / emit split:
//! [`crate::authoring::convert::run`] does the I/O-bounded compute and returns a
//! structured outcome; this module renders it per output mode.
//!
//! Slice scope (US2): **local** sources. Remote `SOURCE` fetching and `--into`
//! injection land in later slices and currently return a clear usage error
//! rather than a silent partial behaviour.

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::authoring::convert::{self, ConvertConfig, ConvertOutcome};
use crate::authoring::detect::ArtifactLevel;
use crate::catalog::git::{Git, scrub_to_string};
use crate::cli::ConvertArgs;
use crate::error::TomeError;
use crate::output::{Mode, write_json};
use crate::workspace::ResolvedScope;

/// Run a conversion for the given artifact `level`.
pub fn run(
    args: ConvertArgs,
    _scope: &ResolvedScope,
    mode: Mode,
    level: ArtifactLevel,
) -> Result<(), TomeError> {
    if args.into.is_some() {
        return Err(TomeError::Usage(
            "`--into` injection lands in a later slice; use `--output` for now".to_owned(),
        ));
    }

    // Resolve SOURCE to a local root: a local path is used in place; a remote
    // (`owner/repo`, git URL) is shallow-cloned into a temp dir whose `Drop`
    // guarantees cleanup on every exit path. `_clone` is held for the whole
    // conversion (NFR-003).
    let (source_root, _clone) = resolve_source(&args.source)?;

    let new_name =
        convert::resolve_requested_name(args.name.as_deref(), args.name_flag.as_deref())?;
    let output_dir = args.output.unwrap_or_else(|| PathBuf::from("."));

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
    emit_report(&outcome, mode)
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
}
