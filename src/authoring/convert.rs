//! The `convert` pipeline: `detect → import → rewrite → lint → emit`.
//!
//! Turns a foreign artifact (a *local* source root) into a native Tome artifact
//! — a copy; the source is never mutated. This pipeline operates on an
//! already-local root; remote `SOURCE` fetching (into a cleaned-up temp clone)
//! and `--into` injection are the command wrapper's concern
//! (`commands::convert`).
//!
//! ```text
//! run(source_root, cfg)
//!   = UntrustedRoot::open            // the read boundary (#184)
//!   → detect(harness, level)         // structural detection
//!   → import_*                       // source → IR (harness-isms rewritten in-import)
//!   → rename                         // FR-007 new name (manifest/dir/frontmatter)
//!   → lint::run                      // fold diagnostics into the report
//!   → [--strict abort]               // anything unrepresentable → 84, before any write
//!   → emit                           // deterministic, atomic landing
//! ```
//!
//! The `--strict` gate runs **before** `emit`, so a real strict abort leaves
//! nothing on disk; under `--dry-run` the would-be violation is carried in the
//! outcome ([`ConvertOutcome::strict_blocked`]) so the plan is still reported.

use std::path::{Path, PathBuf};

use crate::authoring::detect::{ArtifactLevel, SourceHarness, detect};
use crate::authoring::emit::{EmitOptions, emit};
use crate::authoring::import::{claude_code, codex, native_skill};
use crate::authoring::ir::{Artifact, Diagnostic};
use crate::authoring::lint::{self, LintReport};
use crate::authoring::rewrite::is_unsupported_harness_ism;
use crate::authoring::untrusted::UntrustedRoot;
use crate::error::TomeError;
use crate::plugin::identity::{SegmentRejection, validate_segment};

/// Inputs to a single conversion.
#[derive(Debug, Clone)]
pub struct ConvertConfig {
    /// The artifact level the invoking command expects.
    pub level: ArtifactLevel,
    /// `--from` harness override.
    pub from: Option<String>,
    /// Explicit new name (`<NAME>`/`--name`); `None` derives `<current>-tome`.
    pub new_name: Option<String>,
    /// `--strict`: abort (writing nothing) on anything Tome cannot represent.
    pub strict: bool,
    /// `--force`: overwrite colliding files only.
    pub force: bool,
    /// `--dry-run`: compute the plan; write nothing.
    pub dry_run: bool,
    /// Parent directory the converted copy lands under (`<output_dir>/<name>/`).
    pub output_dir: PathBuf,
}

/// The result of a conversion (or a dry-run plan).
#[derive(Debug, Clone)]
pub struct ConvertOutcome {
    pub harness: SourceHarness,
    pub level: ArtifactLevel,
    /// The artifact's original name (pre-rename).
    pub source_name: String,
    /// The final (possibly renamed) artifact name.
    pub final_name: String,
    /// Where the artifact landed (or would land under `--dry-run`).
    pub target: PathBuf,
    /// Aggregated lint/import diagnostics.
    pub report: LintReport,
    /// Files written, relative to `target` (or planned, under `--dry-run`).
    pub written: Vec<PathBuf>,
    pub dry_run: bool,
    /// Under `--dry-run --strict`, the message of the first unrepresentable
    /// feature that WOULD abort a real run — so the plan is still reported and
    /// the caller can surface the non-zero verdict (`Some` only when both
    /// `--dry-run` and `--strict` are set and a blocking diagnostic was found).
    pub strict_blocked: Option<String>,
}

/// Resolve the requested new name from the positional `<NAME>` and `--name`
/// flag: both present and disagreeing is a usage error (FR-007).
pub fn resolve_requested_name(
    positional: Option<&str>,
    flag: Option<&str>,
) -> Result<Option<String>, TomeError> {
    match (positional, flag) {
        (Some(a), Some(b)) if a != b => Err(TomeError::Usage(format!(
            "conflicting new names: positional `{a}` vs `--name {b}` — supply one"
        ))),
        (Some(a), _) => Ok(Some(a.to_owned())),
        (None, Some(b)) => Ok(Some(b.to_owned())),
        (None, None) => Ok(None),
    }
}

/// Run a conversion end-to-end. `source_root` must be a local directory.
pub fn run(source_root: &Path, cfg: &ConvertConfig) -> Result<ConvertOutcome, TomeError> {
    let root = UntrustedRoot::open(source_root)?;
    let detected = detect(&root, cfg.from.as_deref(), cfg.level)?;

    let mut artifact = import(&root, source_root, detected.harness, detected.level)?;
    let source_name = artifact_name(&artifact).to_owned();
    let final_name = match &cfg.new_name {
        Some(n) => n.clone(),
        None => format!("{source_name}-tome"),
    };
    validate_new_name(&final_name)?;
    set_artifact_name(&mut artifact, &final_name);

    let report = lint::run(&artifact, &lint::rules::all());

    // The `--strict` verdict: the first unrepresentable feature, if any.
    let strict_blocked = if cfg.strict {
        first_strict_blocking(&report).map(|d| d.message.clone())
    } else {
        None
    };
    // A REAL (non-dry-run) strict violation aborts BEFORE any write (84,
    // nothing on disk). Under `--dry-run` the violation is instead carried in
    // the outcome so the plan is still reported, and the caller surfaces the
    // non-zero verdict afterwards (contract: dry-run "still reports it").
    if let Some(feature) = &strict_blocked
        && !cfg.dry_run
    {
        return Err(TomeError::ConversionUnsupportedStrict {
            feature: feature.clone(),
        });
    }

    let target = cfg.output_dir.join(&final_name);
    let outcome = emit(
        &artifact,
        &target,
        EmitOptions {
            force: cfg.force,
            dry_run: cfg.dry_run,
        },
    )?;

    Ok(ConvertOutcome {
        harness: detected.harness,
        level: detected.level,
        source_name,
        final_name,
        target,
        report,
        written: outcome.written,
        dry_run: cfg.dry_run,
        strict_blocked,
    })
}

/// Dispatch to the right importer for the detected harness + level.
fn import(
    root: &UntrustedRoot,
    source_root: &Path,
    harness: SourceHarness,
    level: ArtifactLevel,
) -> Result<Artifact, TomeError> {
    match (level, harness) {
        (ArtifactLevel::Plugin, SourceHarness::ClaudeCode) => {
            let default_name = source_basename(source_root);
            Ok(Artifact::Plugin(claude_code::import_plugin(
                root,
                &default_name,
                source_root,
            )?))
        }
        (ArtifactLevel::Plugin, SourceHarness::Codex) => {
            Ok(Artifact::Plugin(codex::import_project(root, source_root)?))
        }
        (ArtifactLevel::Skill, harness) => Ok(Artifact::Skill(native_skill::import(
            root,
            harness,
            source_root,
        )?)),
        (ArtifactLevel::Catalog, SourceHarness::ClaudeCode) => Ok(Artifact::Catalog(
            claude_code::import_marketplace(root, source_root)?,
        )),
        (ArtifactLevel::Catalog, other) => Err(TomeError::Usage(format!(
            "catalog conversion from `{}` is not supported (only Claude Code marketplaces)",
            other.as_str()
        ))),
        (ArtifactLevel::Plugin, other) => Err(TomeError::Usage(format!(
            "plugin conversion from `{}` is not supported",
            other.as_str()
        ))),
    }
}

/// The source dir's base name, used as the fallback plugin name.
fn source_basename(p: &Path) -> String {
    p.file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("plugin")
        .to_owned()
}

/// Validate that a user-supplied/derived new name is a safe path segment (it
/// becomes the artifact directory).
fn validate_new_name(name: &str) -> Result<(), TomeError> {
    validate_segment(name).map_err(|kind: SegmentRejection| {
        TomeError::Usage(format!("`{name}` is not a valid artifact name: {kind}"))
    })
}

fn artifact_name(artifact: &Artifact) -> &str {
    match artifact {
        Artifact::Catalog(c) => &c.name,
        Artifact::Plugin(p) => &p.name,
        Artifact::Skill(s) => &s.name,
    }
}

/// Apply the new name to the **root** artifact only. For a catalog, the
/// vendored child plugins intentionally keep their own names (the rename is the
/// catalog's `<name>-tome`; a child plugin's directory + `plugins[]` entry use
/// its own manifest name). Each child name is independently safe-segment
/// validated by `import_marketplace`.
fn set_artifact_name(artifact: &mut Artifact, name: &str) {
    match artifact {
        Artifact::Catalog(c) => c.name = name.to_owned(),
        Artifact::Plugin(p) => p.name = name.to_owned(),
        Artifact::Skill(s) => s.name = name.to_owned(),
    }
}

/// The first diagnostic that represents content Tome cannot faithfully carry —
/// what `--strict` aborts on. The benign drops (`Info`-level dropped fields,
/// the defaulted-version warning) are intentionally NOT in this set: they
/// produce a valid conversion. The genuinely-lossy ones are: unsupported
/// components/manifest fields, lossy agent fields, dropped tool restrictions,
/// skipped entries, malformed MCP servers, and unmappable harness-isms.
fn first_strict_blocking(report: &LintReport) -> Option<&Diagnostic> {
    report
        .diagnostics
        .iter()
        .find(|d| is_strict_blocking(d.rule_id))
}

fn is_strict_blocking(rule_id: &str) -> bool {
    use crate::authoring::import::rule as cc;
    matches!(
        rule_id,
        cc::UNSUPPORTED_COMPONENT
            | cc::UNSUPPORTED_MANIFEST_FIELD
            | cc::AGENT_LOSSY
            | cc::TOOL_RESTRICTION_DROPPED
            | cc::SKIPPED_ENTRY
            | cc::MALFORMED_MCP
            | cc::CODEX_UNSUPPORTED
            | cc::REMOTE_PLUGIN_SKIPPED
    ) || is_unsupported_harness_ism(rule_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_requested_name_handles_positional_flag_and_conflict() {
        assert_eq!(resolve_requested_name(None, None).unwrap(), None);
        assert_eq!(
            resolve_requested_name(Some("a"), None).unwrap(),
            Some("a".to_owned())
        );
        assert_eq!(
            resolve_requested_name(None, Some("b")).unwrap(),
            Some("b".to_owned())
        );
        assert_eq!(
            resolve_requested_name(Some("a"), Some("a")).unwrap(),
            Some("a".to_owned())
        );
        let err = resolve_requested_name(Some("a"), Some("b")).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn validate_new_name_rejects_unsafe() {
        validate_new_name("my-plugin-tome").unwrap();
        for bad in ["..", "a/b", "", ".hidden"] {
            assert!(validate_new_name(bad).is_err(), "expected `{bad}` rejected");
        }
    }

    #[test]
    fn strict_blocking_classifies_rule_ids() {
        use crate::authoring::import::rule as cc;
        assert!(is_strict_blocking(cc::UNSUPPORTED_COMPONENT));
        assert!(is_strict_blocking(cc::TOOL_RESTRICTION_DROPPED));
        assert!(is_strict_blocking(
            crate::authoring::rewrite::rule::SHELL_EXEC
        ));
        // Benign drops do NOT block a strict conversion.
        assert!(!is_strict_blocking(cc::MISSING_VERSION));
        assert!(!is_strict_blocking(cc::DROPPED_MANIFEST_FIELD));
        assert!(!is_strict_blocking(cc::DROPPED_FRONTMATTER));
        assert!(!is_strict_blocking(
            crate::authoring::rewrite::rule::PLUGIN_ROOT
        ));
    }

    #[test]
    fn source_basename_falls_back() {
        assert_eq!(source_basename(Path::new("/a/b/my-plugin")), "my-plugin");
        assert_eq!(source_basename(Path::new("/")), "plugin");
    }
}
