//! Lint rule registry + runner — the shared validation core for `lint` and
//! `convert` (which folds lint diagnostics into its report).
//!
//! The [`rules`] module holds the individual rules (each a [`Rule`] with an
//! `id`/`scope`/`autofixable` + scope-appropriate `check_*` methods). The
//! [`run`] walker here visits **all** nested IR (catalog → plugins → entries) —
//! it never stops at the first failure — aggregates every diagnostic (both the
//! IR-node-carried ones and the rule-produced ones), and computes one
//! [`Verdict`] (errors > strict-warnings > clean). `--autofix` (US3) applies
//! the `autofixable` fixes per-file via atomic replace with `first_error`
//! forward-progress; the `--json` shape is a single `{ findings[], summary }`
//! object.
//!
//! Framework lands in Phase 2 (Foundational); concrete rules + autofix in
//! Phase 5 (US3).

pub mod autofix;
pub mod parse;
pub mod rules;

use crate::authoring::ir::{Artifact, CatalogIr, Diagnostic, EntryIr, PluginIr, Severity};
use crate::error::TomeError;

/// Which IR level a rule applies to. A rule overrides only the `check_*` method
/// for its scope; the others default to "no findings".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Catalog,
    Plugin,
    Entry,
}

/// A lint rule. Implementors override exactly the `check_*` method matching
/// their [`Scope`]; the runner calls every rule against every node at its
/// scope. Default impls return no findings so a rule only writes the one it
/// cares about.
pub trait Rule {
    /// Stable rule identifier surfaced in findings.
    fn id(&self) -> &'static str;
    /// The IR level this rule inspects.
    fn scope(&self) -> Scope;
    /// Whether this rule can produce mechanically-applicable fixes.
    fn autofixable(&self) -> bool {
        false
    }

    fn check_catalog(&self, _catalog: &CatalogIr) -> Vec<Diagnostic> {
        Vec::new()
    }
    fn check_plugin(&self, _plugin: &PluginIr) -> Vec<Diagnostic> {
        Vec::new()
    }
    fn check_entry(&self, _entry: &EntryIr) -> Vec<Diagnostic> {
        Vec::new()
    }
}

/// Options for a lint run.
#[derive(Debug, Clone, Copy, Default)]
pub struct LintOptions {
    /// Warnings (with no errors) also fail the run.
    pub strict: bool,
}

/// The verdict a lint run resolves to (FR-021 / contracts/command-lint.md).
/// Precedence: `Errors`(85) > `StrictWarnings`(86) > `Clean`(0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Clean,
    StrictWarnings,
    Errors,
}

/// Aggregated lint output: every diagnostic plus per-severity tallies.
#[derive(Debug, Clone, Default)]
pub struct LintReport {
    pub diagnostics: Vec<Diagnostic>,
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
}

impl LintReport {
    pub(crate) fn from_diagnostics(diagnostics: Vec<Diagnostic>) -> Self {
        let mut report = LintReport {
            diagnostics,
            ..Default::default()
        };
        for d in &report.diagnostics {
            match d.severity {
                Severity::Error => report.errors += 1,
                Severity::Warning => report.warnings += 1,
                Severity::Info => report.infos += 1,
            }
        }
        report
    }

    /// Resolve the verdict given strictness. Errors always dominate; warnings
    /// only matter under `--strict`.
    pub fn verdict(&self, strict: bool) -> Verdict {
        if self.errors > 0 {
            Verdict::Errors
        } else if strict && self.warnings > 0 {
            Verdict::StrictWarnings
        } else {
            Verdict::Clean
        }
    }

    /// Map the verdict to the command-boundary `Result`: `Clean` → `Ok`,
    /// `Errors` → [`TomeError::ValidationFoundErrors`] (85), `StrictWarnings`
    /// → [`TomeError::ValidationStrictWarnings`] (86).
    pub fn into_result(&self, strict: bool) -> Result<(), TomeError> {
        match self.verdict(strict) {
            Verdict::Clean => Ok(()),
            Verdict::Errors => Err(TomeError::ValidationFoundErrors {
                errors: self.errors,
            }),
            Verdict::StrictWarnings => Err(TomeError::ValidationStrictWarnings {
                warnings: self.warnings,
            }),
        }
    }

    /// Diagnostics that carry a mechanically-applicable fix (for `--autofix`).
    pub fn autofixable(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(|d| d.autofix.is_some())
    }
}

/// Run every rule over every nested IR node, never exiting early, and return
/// the aggregated report. IR-node-carried diagnostics (from parsing/import)
/// are included alongside the rule-produced ones.
pub fn run(artifact: &Artifact, rules: &[Box<dyn Rule>]) -> LintReport {
    let mut out = Vec::new();
    match artifact {
        Artifact::Catalog(cat) => walk_catalog(cat, rules, &mut out),
        Artifact::Plugin(plugin) => walk_plugin(plugin, rules, &mut out),
        Artifact::Skill(entry) => walk_entry(entry, rules, &mut out),
    }
    LintReport::from_diagnostics(out)
}

fn walk_catalog(cat: &CatalogIr, rules: &[Box<dyn Rule>], out: &mut Vec<Diagnostic>) {
    out.extend(cat.diagnostics.iter().cloned());
    for r in rules {
        if r.scope() == Scope::Catalog {
            out.extend(r.check_catalog(cat));
        }
    }
    for plugin in &cat.plugins {
        walk_plugin(plugin, rules, out);
    }
}

fn walk_plugin(plugin: &PluginIr, rules: &[Box<dyn Rule>], out: &mut Vec<Diagnostic>) {
    out.extend(plugin.diagnostics.iter().cloned());
    for r in rules {
        if r.scope() == Scope::Plugin {
            out.extend(r.check_plugin(plugin));
        }
    }
    for entry in &plugin.entries {
        walk_entry(entry, rules, out);
    }
}

fn walk_entry(entry: &EntryIr, rules: &[Box<dyn Rule>], out: &mut Vec<Diagnostic>) {
    out.extend(entry.diagnostics.iter().cloned());
    for r in rules {
        if r.scope() == Scope::Entry {
            out.extend(r.check_entry(entry));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `EntryIr`/`PluginIr` already arrive via `super::*`; only import the rest.
    use crate::authoring::ir::{MappedFrontmatter, Provenance};
    use crate::plugin::identity::EntryKind;
    use std::path::PathBuf;

    /// A rule that flags every entry it sees, at a configurable severity.
    struct FlagEntries {
        severity: Severity,
    }
    impl Rule for FlagEntries {
        fn id(&self) -> &'static str {
            "test/flag-entries"
        }
        fn scope(&self) -> Scope {
            Scope::Entry
        }
        fn check_entry(&self, entry: &EntryIr) -> Vec<Diagnostic> {
            vec![Diagnostic::new(
                "test/flag-entries",
                self.severity,
                format!("flagged {}", entry.name),
            )]
        }
    }

    fn entry(name: &str) -> EntryIr {
        EntryIr {
            kind: EntryKind::Skill,
            name: name.into(),
            description: Some("d".into()),
            frontmatter: MappedFrontmatter::default(),
            body: String::new(),
            supporting_files: Vec::new(),
            source_path: PathBuf::from("s"),
            diagnostics: Vec::new(),
        }
    }

    fn plugin_with(entries: Vec<EntryIr>, node_diags: Vec<Diagnostic>) -> PluginIr {
        PluginIr {
            name: "p".into(),
            version: "1.0.0".into(),
            description: None,
            author: None,
            license: None,
            entries,
            mcp_servers: Vec::new(),
            hooks_files: Vec::new(),
            hooks_json: None,
            mcp_json: None,
            provenance: Provenance::local("t", PathBuf::from("s")),
            diagnostics: node_diags,
        }
    }

    #[test]
    fn runner_visits_all_entries_and_collects_node_diagnostics() {
        let plugin = plugin_with(
            vec![entry("a"), entry("b"), entry("c")],
            vec![Diagnostic::warning("node/x", "carried")],
        );
        let rules: Vec<Box<dyn Rule>> = vec![Box::new(FlagEntries {
            severity: Severity::Warning,
        })];
        let report = run(&Artifact::Plugin(plugin), &rules);
        // 3 entry findings + 1 node-carried diagnostic.
        assert_eq!(report.diagnostics.len(), 4);
        assert_eq!(report.warnings, 4);
    }

    #[test]
    fn verdict_precedence_errors_over_strict_warnings_over_clean() {
        let clean = LintReport::from_diagnostics(vec![]);
        assert_eq!(clean.verdict(false), Verdict::Clean);
        assert_eq!(clean.verdict(true), Verdict::Clean);

        let warn = LintReport::from_diagnostics(vec![Diagnostic::warning("w", "x")]);
        assert_eq!(
            warn.verdict(false),
            Verdict::Clean,
            "warnings don't fail without --strict"
        );
        assert_eq!(warn.verdict(true), Verdict::StrictWarnings);

        let err = LintReport::from_diagnostics(vec![
            Diagnostic::warning("w", "x"),
            Diagnostic::error("e", "y"),
        ]);
        assert_eq!(err.verdict(false), Verdict::Errors, "errors dominate");
        assert_eq!(err.verdict(true), Verdict::Errors);
    }

    #[test]
    fn into_result_maps_verdict_to_exit_codes() {
        let err = LintReport::from_diagnostics(vec![Diagnostic::error("e", "y")]);
        assert!(matches!(
            err.into_result(false),
            Err(TomeError::ValidationFoundErrors { errors: 1 })
        ));

        let warn = LintReport::from_diagnostics(vec![Diagnostic::warning("w", "x")]);
        assert!(
            warn.into_result(false).is_ok(),
            "warnings pass without --strict"
        );
        assert!(matches!(
            warn.into_result(true),
            Err(TomeError::ValidationStrictWarnings { warnings: 1 })
        ));

        let clean = LintReport::from_diagnostics(vec![Diagnostic::info("i", "z")]);
        assert!(clean.into_result(true).is_ok());
    }
}
