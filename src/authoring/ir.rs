//! The normalized artifact IR — the in-memory model every authoring command
//! produces and consumes (catalog → plugins → entries → diagnostics).
//!
//! Not serialized to disk: [`super::emit`] writes the on-disk Tome format
//! (`tome-catalog.toml` / `tome-plugin.toml` / `SKILL.md`), and diagnostics
//! flow to the command report. With native-`SKILL.md`-only conversion the IR
//! is near-identical to the emitted format, so the per-harness importer code
//! stays a thin source→IR parser. See `data-model.md §4`.
//!
//! Existing types are reused as the single source of truth rather than
//! duplicated: [`EntryKind`] (the entry-kind discriminator), [`TomeAuthor`]
//! (the `[author]` shape), the catalog [`Owner`], and [`SkillFrontmatter`]
//! (the Tome-modelled frontmatter set, `data-model.md §6`).

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::catalog::manifest::Owner;
use crate::plugin::frontmatter::SkillFrontmatter;
use crate::plugin::identity::EntryKind;
use crate::plugin::manifest::TomeAuthor;

/// One authoring artifact at its top level — what `convert`/`create` produce
/// and `emit`/`lint` consume. The three levels nest: a catalog holds plugins,
/// a plugin holds entries.
#[derive(Debug, Clone)]
pub enum Artifact {
    Catalog(CatalogIr),
    Plugin(PluginIr),
    Skill(EntryIr),
}

/// A catalog (`tome-catalog.toml` + its vendored plugins).
#[derive(Debug, Clone)]
pub struct CatalogIr {
    pub name: String,
    pub version: String,
    pub description: String,
    pub owner: Owner,
    pub plugins: Vec<PluginIr>,
    /// Where this IR came from (source harness + path), for the report.
    pub provenance: Provenance,
    /// Diagnostics scoped to the catalog manifest itself.
    pub diagnostics: Vec<Diagnostic>,
}

/// A plugin (`tome-plugin.toml` + `skills/`/`commands/`/`agents/` + optional
/// `.mcp.json`).
#[derive(Debug, Clone)]
pub struct PluginIr {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub author: Option<TomeAuthor>,
    pub license: Option<String>,
    pub entries: Vec<EntryIr>,
    pub mcp_servers: Vec<McpServerIr>,
    /// Verbatim pass-through files — currently the `hooks/` subtree. Copied
    /// byte-identical to the emitted plugin, never harness-ism-rewritten:
    /// sync-time `harness::hooks::read_rewritten_entries` owns the
    /// `${CLAUDE_PLUGIN_ROOT}`/`${CLAUDE_PLUGIN_DATA}` rewrite and expects the
    /// tokens intact. Rel-paths are plugin-root-relative, importer-validated.
    pub hooks_files: Vec<SupportingFile>,
    /// Raw `hooks/hooks.json` body when present and readable — carried so the
    /// lint hooks-spec rule reads the IR, not the source tree. `None` means the
    /// file was absent, the `hooks/` directory was absent, or the file was
    /// present but unreadable (that last case also emits `convert/hooks-unreadable`).
    pub hooks_json: Option<String>,
    pub provenance: Provenance,
    pub diagnostics: Vec<Diagnostic>,
}

/// One entry: a skill (`skills/<name>/SKILL.md`), command (`commands/<n>.md`),
/// or agent (`agents/<n>.md`).
#[derive(Debug, Clone)]
pub struct EntryIr {
    pub kind: EntryKind,
    /// Entry name. For skills this MUST equal the directory name.
    pub name: String,
    pub description: Option<String>,
    /// Only the Tome-modelled frontmatter fields (`data-model.md §6`).
    pub frontmatter: MappedFrontmatter,
    /// Entry body (Markdown after the frontmatter). Valid UTF-8; harness-isms
    /// already rewritten on the `convert` path.
    pub body: String,
    /// Supporting files (`scripts/`, `references/`, `assets/`) to copy
    /// alongside the entry; each source path validated safe by the importer.
    pub supporting_files: Vec<SupportingFile>,
    /// Where this entry was read from (for the report + supporting-file copies).
    pub source_path: PathBuf,
    pub diagnostics: Vec<Diagnostic>,
}

/// A supporting file to copy alongside an entry. `relative` is the path under
/// the entry's directory the file lands at (preserving `scripts/`/`references/`
/// substructure); `source` is the absolute on-disk path to copy from (already
/// validated safe by the importer).
#[derive(Debug, Clone)]
pub struct SupportingFile {
    pub relative: PathBuf,
    pub source: PathBuf,
}

/// The Tome-modelled frontmatter set — an alias for the existing
/// [`SkillFrontmatter`] (the lenient parser already models exactly the §6
/// fields and ignores everything else). The emitter writes these fields back
/// in a fixed order for byte-stable output (FR-027).
pub type MappedFrontmatter = SkillFrontmatter;

/// An MCP server entry, synthesized into a plugin's `.mcp.json`.
#[derive(Debug, Clone)]
pub struct McpServerIr {
    pub name: String,
    pub transport: McpTransport,
}

/// MCP transport. Inferred from the source (`command` ⇒ stdio, `url` ⇒ http).
#[derive(Debug, Clone)]
pub enum McpTransport {
    Stdio {
        command: String,
        args: Vec<String>,
        /// Sorted for deterministic emission (FR-027).
        env: BTreeMap<String, String>,
    },
    Http {
        url: String,
    },
}

/// Provenance of an IR node — the source harness label and the path it was
/// read from (template/source for `create`/`convert`, the artifact itself for
/// `lint`). Surfaced in the human report.
#[derive(Debug, Clone)]
pub struct Provenance {
    pub source_harness: String,
    pub source_path: PathBuf,
}

impl Provenance {
    /// Provenance for an artifact authored in-place (e.g. `create` from a
    /// built-in template, or `lint` reading a Tome artifact).
    pub fn local(label: &str, path: PathBuf) -> Self {
        Self {
            source_harness: label.to_owned(),
            source_path: path,
        }
    }
}

/// Severity of a [`Diagnostic`]. Ordered so a max over a diagnostic set yields
/// the dominant severity (`Error` > `Warning` > `Info`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// A location for a diagnostic — a file and optional 1-based line.
#[derive(Debug, Clone)]
pub struct Location {
    pub file: PathBuf,
    pub line: Option<usize>,
}

impl Location {
    pub fn file(file: PathBuf) -> Self {
        Self { file, line: None }
    }
}

/// A mechanically-applicable fix: replace `path`'s contents with `replacement`.
/// The rule (or rewriter) that produced the diagnostic computes the corrected
/// bytes; the `--autofix` runner just lands them atomically per file.
#[derive(Debug, Clone)]
pub struct Fix {
    pub path: PathBuf,
    pub replacement: String,
}

/// One finding: a rule id, a severity, a human message that names the
/// file/field + the fix (principle V), an optional location, and an optional
/// mechanically-applicable [`Fix`].
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub rule_id: &'static str,
    pub severity: Severity,
    pub message: String,
    pub location: Option<Location>,
    pub autofix: Option<Fix>,
}

/// The SSOT `Diagnostic` → JSON finding shape shared by `lint --json`
/// (each object in its `findings[]`) and `convert --json` (the body of each
/// JSONL `type: "diagnostic"` line). Extracting the mapping into one function
/// guarantees the two verbs emit byte-identical per-finding fields — a caller
/// parsing lint findings can parse convert diagnostic lines the same way
/// (issue #299).
///
/// Field semantics (matched to lint's original finding shape):
/// - `rule` / `severity` / `message`: the diagnostic's own fields.
/// - `file`: the location's file path, or JSON `null` when the diagnostic
///   carries no location.
/// - `line`: the location's 1-based line, or JSON `null` when absent (no
///   location, or a location without a line).
/// - `autofixable`: whether the diagnostic carries a mechanically-applicable
///   [`Fix`].
///
/// `convert` wraps this object with a `type: "diagnostic"` discriminator to
/// preserve its JSONL envelope; `lint` uses it verbatim as a `findings[]` entry.
pub fn finding_json(d: &Diagnostic) -> serde_json::Value {
    serde_json::json!({
        "rule": d.rule_id,
        "severity": d.severity.as_str(),
        "message": d.message,
        "file": d.location.as_ref().map(|l| l.file.display().to_string()),
        "line": d.location.as_ref().and_then(|l| l.line),
        "autofixable": d.autofix.is_some(),
    })
}

impl Diagnostic {
    /// Build a diagnostic with the given severity (no location, no autofix).
    pub fn new(rule_id: &'static str, severity: Severity, message: impl Into<String>) -> Self {
        Self {
            rule_id,
            severity,
            message: message.into(),
            location: None,
            autofix: None,
        }
    }

    pub fn error(rule_id: &'static str, message: impl Into<String>) -> Self {
        Self::new(rule_id, Severity::Error, message)
    }

    pub fn warning(rule_id: &'static str, message: impl Into<String>) -> Self {
        Self::new(rule_id, Severity::Warning, message)
    }

    pub fn info(rule_id: &'static str, message: impl Into<String>) -> Self {
        Self::new(rule_id, Severity::Info, message)
    }

    /// Attach a location (builder style).
    #[must_use]
    pub fn at(mut self, location: Location) -> Self {
        self.location = Some(location);
        self
    }

    /// Attach an autofix (builder style); marks the diagnostic mechanically
    /// fixable.
    #[must_use]
    pub fn with_fix(mut self, fix: Fix) -> Self {
        self.autofix = Some(fix);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_orders_error_over_warning_over_info() {
        assert!(Severity::Error > Severity::Warning);
        assert!(Severity::Warning > Severity::Info);
        let max = [Severity::Info, Severity::Error, Severity::Warning]
            .into_iter()
            .max()
            .unwrap();
        assert_eq!(max, Severity::Error);
    }

    #[test]
    fn diagnostic_builders_set_fields() {
        let d = Diagnostic::warning("rule.x", "something")
            .at(Location::file(PathBuf::from("SKILL.md")))
            .with_fix(Fix {
                path: PathBuf::from("SKILL.md"),
                replacement: "fixed".into(),
            });
        assert_eq!(d.rule_id, "rule.x");
        assert_eq!(d.severity, Severity::Warning);
        assert!(d.location.is_some());
        assert!(d.autofix.is_some());
    }
}
