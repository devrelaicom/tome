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
    /// `--from` harness override (a parsed [`SourceHarness`]; clap closes the
    /// surface at parse time).
    pub from: Option<SourceHarness>,
    /// Explicit new name (`<NAME>`); `None` derives `<current>-tome`.
    pub new_name: Option<String>,
    /// `--strict`: abort (writing nothing) on anything Tome cannot represent.
    pub strict: bool,
    /// `--allow <rule-id>` (repeatable): rule ids demoted out of the strict
    /// blocking set. An allowed rule still emits its normal warning/diagnostic;
    /// it just no longer aborts `--strict`. An id that isn't strict-blocking (or
    /// doesn't exist) is a harmless no-op. Only consulted when `strict` is set.
    pub allow: Vec<String>,
    /// `--force`: overwrite colliding files only.
    pub force: bool,
    /// `--dry-run`: compute the plan; write nothing.
    pub dry_run: bool,
    /// Fetch remote-source marketplace plugins (github/git/url) into temp clones
    /// and vendor them (`catalog convert`; default). `false` under `--no-fetch`.
    pub fetch_remote: bool,
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
    /// Under `--dry-run --strict`, an aggregate message naming the COUNT and the
    /// distinct blocking rule-ids that WOULD abort a real run — so the plan is
    /// still reported and the caller can surface the non-zero verdict (`Some`
    /// only when both `--dry-run` and `--strict` are set and, after `--allow`,
    /// at least one blocking diagnostic remained).
    pub strict_blocked: Option<String>,
}

/// Run a conversion end-to-end. `source_root` must be a local directory.
pub fn run(source_root: &Path, cfg: &ConvertConfig) -> Result<ConvertOutcome, TomeError> {
    let root = UntrustedRoot::open(source_root)?;
    let detected = detect(&root, cfg.from, cfg.level)?;

    // The fetch context owns the temp clones for remote-source marketplace
    // plugins. The clones MUST outlive `emit` — planned `Copy` files are read
    // from the clone at landing time — so the context lives to end of scope
    // and is dropped (cleaning up every clone) only after the emit completes.
    let mut fetch = crate::authoring::import::FetchContext::new(cfg.fetch_remote);

    let mut artifact = import(
        &root,
        source_root,
        detected.harness,
        detected.level,
        &mut fetch,
    )?;
    let source_name = artifact_name(&artifact).to_owned();
    let final_name = match &cfg.new_name {
        Some(n) => n.clone(),
        None => format!("{source_name}-tome"),
    };
    validate_new_name(&final_name)?;
    set_artifact_name(&mut artifact, &final_name);

    // `for_convert` omits the filesystem-structural rules: the IR's
    // `source_path` still points at the foreign source tree here, so
    // `UnsupportedComponents`/`EntryName` would re-flag what the importer already
    // reported (under different rule ids) by reading the SOURCE, not the output.
    let report = lint::run(&artifact, &lint::rules::for_convert());

    // The `--strict` verdict: ALL remaining blocking diagnostics (after
    // applying `--allow`), summarized into one message naming the count and the
    // distinct blocking rule-ids — so the user knows exactly what to `--allow`.
    let strict_blocked = if cfg.strict {
        let blocking = strict_blocking(&report, &cfg.allow);
        (!blocking.is_empty()).then(|| strict_blocked_message(&blocking))
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
    fetch: &mut crate::authoring::import::FetchContext,
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
            claude_code::import_marketplace(root, source_root, fetch)?,
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

/// Every diagnostic that represents content Tome cannot faithfully carry AND is
/// not demoted by `--allow` — what `--strict` aborts on. The benign drops
/// (`Info`-level dropped fields, the defaulted-version warning) are intentionally
/// NOT in the fixed blocking set: they produce a valid conversion. The
/// genuinely-lossy ones are: unsupported components/manifest fields, lossy agent
/// fields, dropped tool restrictions, skipped entries, malformed MCP servers,
/// and unmappable harness-isms.
///
/// A diagnostic is strict-blocking IFF its rule id is in the fixed set AND that
/// id is not present in `allow`. An `allow` entry that names a non-blocking (or
/// unknown) rule id is a harmless no-op — it simply matches nothing here.
fn strict_blocking<'a>(report: &'a LintReport, allow: &[String]) -> Vec<&'a Diagnostic> {
    report
        .diagnostics
        .iter()
        .filter(|d| is_strict_blocking(d.rule_id) && !allow.iter().any(|a| a == d.rule_id))
        .collect()
}

/// Summarize the strict-blocking diagnostics into one message: the total count
/// plus the distinct blocking rule-ids (so the user knows exactly what to
/// `--allow`). Rule-ids are listed in first-seen order, de-duplicated. The
/// caller has already checked the slice is non-empty.
fn strict_blocked_message(blocking: &[&Diagnostic]) -> String {
    let mut distinct: Vec<&str> = Vec::new();
    for d in blocking {
        if !distinct.contains(&d.rule_id) {
            distinct.push(d.rule_id);
        }
    }
    let ids = distinct.join(", ");
    let noun = if blocking.len() == 1 {
        "feature"
    } else {
        "features"
    };
    format!(
        "{} unrepresentable {} across {} rule(s): {} — pass `--allow <rule-id>` per rule to tolerate an intentional drop",
        blocking.len(),
        noun,
        distinct.len(),
        ids,
    )
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
            | cc::REMOTE_PLUGIN_FETCH_FAILED
            | cc::HOOKS_UNREADABLE
    )
        // A valid-UTF-8/invalid-JSON hooks.json is strict-blocking at convert time
        // because it would hard-fail `harness sync` at exit 43 later. The rule is
        // provenance-safe (reads IR only, not the source tree), so it fires on the
        // convert path without risk of double-flagging.
        || rule_id == crate::authoring::lint::rules::rule::HOOKS_SPEC
        || is_unsupported_harness_ism(rule_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_new_name_rejects_unsafe() {
        validate_new_name("my-plugin-tome").unwrap();
        // Incl. the highest-risk `join` inputs the SEC-1 lesson flags: absolute
        // and embedded-traversal names.
        for bad in ["..", "a/b", "", ".hidden", "/etc", "a/../b", "\\abs"] {
            assert!(validate_new_name(bad).is_err(), "expected `{bad}` rejected");
        }
    }

    #[test]
    fn strict_blocking_classifies_rule_ids() {
        use crate::authoring::import::rule as cc;
        assert!(is_strict_blocking(cc::UNSUPPORTED_COMPONENT));
        assert!(is_strict_blocking(cc::TOOL_RESTRICTION_DROPPED));
        assert!(is_strict_blocking(cc::HOOKS_UNREADABLE));
        // Symmetry: valid-UTF-8/invalid-JSON hooks.json is also strict-blocking
        // (would hard-fail harness sync at exit 43).
        assert!(is_strict_blocking(
            crate::authoring::lint::rules::rule::HOOKS_SPEC
        ));
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

    use crate::authoring::ir::{Diagnostic, Severity};
    use crate::authoring::lint::LintReport;

    fn report_with(diags: Vec<Diagnostic>) -> LintReport {
        let mut r = LintReport::default();
        for d in diags {
            match d.severity {
                Severity::Error => r.errors += 1,
                Severity::Warning => r.warnings += 1,
                Severity::Info => r.infos += 1,
            }
            r.diagnostics.push(d);
        }
        r
    }

    #[test]
    fn strict_blocking_collects_all_and_respects_allow() {
        use crate::authoring::import::rule as cc;
        let report = report_with(vec![
            Diagnostic::warning(cc::UNSUPPORTED_COMPONENT, "themes/ dropped"),
            Diagnostic::warning(cc::TOOL_RESTRICTION_DROPPED, "allowed-tools dropped"),
            // A benign drop — never blocks.
            Diagnostic::info(cc::DROPPED_MANIFEST_FIELD, "displayName dropped"),
        ]);

        // No allow: both genuinely-lossy findings block (the benign one does not).
        let all = strict_blocking(&report, &[]);
        assert_eq!(all.len(), 2, "counts ALL blocking findings, not just first");

        // Allow the component rule: only the tool-restriction one remains.
        let allowed = strict_blocking(&report, &[cc::UNSUPPORTED_COMPONENT.to_owned()]);
        assert_eq!(allowed.len(), 1);
        assert_eq!(allowed[0].rule_id, cc::TOOL_RESTRICTION_DROPPED);

        // Allow both blocking rule ids: nothing left to block.
        let none = strict_blocking(
            &report,
            &[
                cc::UNSUPPORTED_COMPONENT.to_owned(),
                cc::TOOL_RESTRICTION_DROPPED.to_owned(),
            ],
        );
        assert!(none.is_empty(), "all blocking rules demoted → no abort");
    }

    #[test]
    fn allow_of_non_blocking_or_unknown_id_is_a_no_op() {
        use crate::authoring::import::rule as cc;
        let report = report_with(vec![Diagnostic::warning(
            cc::UNSUPPORTED_COMPONENT,
            "themes/ dropped",
        )]);
        // Allowing a benign rule id and a made-up one changes nothing.
        let blocking = strict_blocking(
            &report,
            &[
                cc::DROPPED_MANIFEST_FIELD.to_owned(),
                "convert/does-not-exist".to_owned(),
            ],
        );
        assert_eq!(blocking.len(), 1);
        assert_eq!(blocking[0].rule_id, cc::UNSUPPORTED_COMPONENT);
    }

    #[test]
    fn strict_blocked_message_reports_count_and_distinct_rule_ids() {
        use crate::authoring::import::rule as cc;
        let d1 = Diagnostic::warning(cc::UNSUPPORTED_COMPONENT, "themes/ dropped");
        let d2 = Diagnostic::warning(cc::UNSUPPORTED_COMPONENT, "monitors/ dropped");
        let d3 = Diagnostic::warning(cc::TOOL_RESTRICTION_DROPPED, "allowed-tools dropped");
        let blocking = vec![&d1, &d2, &d3];
        let msg = strict_blocked_message(&blocking);
        // Count of findings (3) not distinct-rule count.
        assert!(msg.contains('3'), "reports the finding count: {msg}");
        // Both distinct rule-ids named, once each.
        assert!(msg.contains(cc::UNSUPPORTED_COMPONENT), "{msg}");
        assert!(msg.contains(cc::TOOL_RESTRICTION_DROPPED), "{msg}");
        assert_eq!(
            msg.matches(cc::UNSUPPORTED_COMPONENT).count(),
            1,
            "rule-id de-duplicated: {msg}"
        );
        // Actionable hint present.
        assert!(msg.contains("--allow"), "{msg}");
    }

    #[test]
    fn strict_blocked_message_singular_for_one_finding() {
        use crate::authoring::import::rule as cc;
        let d1 = Diagnostic::warning(cc::TOOL_RESTRICTION_DROPPED, "allowed-tools dropped");
        let msg = strict_blocked_message(&[&d1]);
        assert!(msg.contains("1 unrepresentable feature"), "{msg}");
        assert!(!msg.contains("features"), "singular noun: {msg}");
    }
}
