//! Guardrails soft-fallback writer (Phase 6 / US3).
//!
//! Contract: `specs/006-phase-6-hooks-agents/contracts/guardrails.md`.
//!
//! Guardrails are the honest degradation path for pre/post-action
//! constraints everywhere a real Claude Code JSON hook cannot run. A plugin
//! optionally ships `<plugin-root>/hooks/GUARDRAILS.md`; Tome copies its body
//! **verbatim** (never parses it) into a per-plugin marker region in each
//! harness's guardrails target.
//!
//! ## Marker region (FR-011, FR-011a)
//!
//! ```text
//! <!-- START GUARDRAILS: <catalog>:<plugin> -->
//! <verbatim body>
//! <!-- END GUARDRAILS: <catalog>:<plugin> -->
//! ```
//!
//! Distinct from the Phase 4 `tome:begin/end` rules block — both coexist on
//! the same file (R-5). The `<catalog>:<plugin>` text is the sole per-plugin
//! removal key; state is filesystem-inferred from the marker pairs, no
//! sidecar (FR-015, NFR-004).
//!
//! ## Targets (FR-012)
//!
//! Per-harness placement comes from `HarnessModule::guardrails_target`:
//! an in-file region (`CLAUDE.md`, shared `AGENTS.md`, `GEMINI.md`) or the
//! Cursor standalone sibling `.cursor/rules/TOME_GUARDRAILS.md`. Claude Code
//! suppresses a plugin's `CLAUDE.md` region when that plugin ships real JSON
//! hooks (FR-013) — computed by the sync orchestrator, which knows the hooks
//! set, and passed in as the per-file suppression filter.
//!
//! ## Determinism + idempotence (FR-011, FR-014, NFR-001)
//!
//! Within a file: the `tome:begin/end` block first, then guardrails regions
//! in lexicographic `<catalog>:<plugin>` order. Existing regions are
//! overwritten between their markers in place (never duplicated, never
//! reordered); new regions are appended in lex order; orphaned regions are
//! removed. A re-sync with no change rewrites nothing.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

use crate::error::TomeError;
use crate::harness::rules_file::{self, MarkerRegion, MarkerSpec};

/// Line-anchored START regex; captures the `<catalog>:<plugin>` provenance
/// as the named group `key` (catalog has no `:`; plugin may). Trailing
/// whitespace tolerated.
const START_REGEX: &str =
    r"^<!-- START GUARDRAILS: (?P<key>(?P<catalog>[^:]+):(?P<plugin>.+)) -->\s*$";

/// Line-anchored END regex (key-agnostic). The reconciler verifies the
/// matched END's key against the open START via the canonical renderer.
const END_REGEX: &str = r"^<!-- END GUARDRAILS: .+ -->\s*$";

/// The compiled marker spec for guardrails regions. Compiled once.
fn guardrails_spec() -> &'static MarkerSpec {
    static SPEC: OnceLock<MarkerSpec> = OnceLock::new();
    SPEC.get_or_init(|| {
        let start = Regex::new(START_REGEX).expect("guardrails START regex compiles");
        let end_any = Regex::new(END_REGEX).expect("guardrails END regex compiles");
        MarkerSpec::new(start, end_any, begin_marker, end_marker)
    })
}

/// The compiled guardrails START regex. Compiled once. Shared with the
/// body-validation scan in [`read_guardrails_source`] so the literal lives in
/// exactly one place (`START_REGEX`).
fn start_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(START_REGEX).expect("guardrails START regex compiles"))
}

/// The compiled key-agnostic guardrails END regex. Compiled once. Shared with
/// the body-validation scan in [`read_guardrails_source`].
fn end_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(END_REGEX).expect("guardrails END regex compiles"))
}

/// The compiled `tome:begin/end` block-marker regex (owned by `rules_file`).
/// Reused here so a guardrails body cannot smuggle a Phase 4 rules-block
/// marker into the merged file.
fn block_marker_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(rules_file::BLOCK_MARKER_REGEX).expect("rules block marker regex compiles")
    })
}

/// Render the canonical START marker line for `key` (`<catalog>:<plugin>`).
fn begin_marker(key: &str) -> String {
    format!("<!-- START GUARDRAILS: {key} -->")
}

/// Render the canonical END marker line for `key`.
fn end_marker(key: &str) -> String {
    format!("<!-- END GUARDRAILS: {key} -->")
}

/// The provenance key for a `(catalog, plugin)` pair.
pub fn region_key(catalog: &str, plugin: &str) -> String {
    format!("{catalog}:{plugin}")
}

/// Read a plugin's `hooks/GUARDRAILS.md` body verbatim.
///
/// Returns `Ok(None)` when the plugin ships no `GUARDRAILS.md` (it
/// contributes no region). The source is bounded-read and symlink-refused;
/// Tome NEVER parses the body. A read failure other than "absent" surfaces
/// [`TomeError::GuardrailsWriteFailed`] (exit 46) naming the source file.
///
/// # Fail-closed marker validation (B-1)
///
/// The body is copied **verbatim** between Tome's managed markers and is
/// re-parsed on every sync. A body line that itself looks like a guardrails
/// START / END marker, or like a Phase 4 `tome:begin/end` block marker, would
/// let a plugin escape its own region, wedge the file (a stray END makes the
/// next parse fail), or corrupt the rules block. Escaping the body is wrong
/// (it is contractually verbatim), so the honest defence is refusal: any such
/// line surfaces [`TomeError::GuardrailsWriteFailed`] (exit 46) naming the
/// source. The reconcile loop records this on its forward-progress error slot
/// and keeps reconciling sibling plugins (FR-084).
pub fn read_guardrails_source(plugin_root: &Path) -> Result<Option<String>, TomeError> {
    let source = plugin_root.join("hooks").join("GUARDRAILS.md");
    rules_file::refuse_symlink(&source).map_err(|_| TomeError::GuardrailsWriteFailed {
        path: source.clone(),
    })?;
    let body = match crate::util::bounded_read_to_string(&source, crate::util::HARNESS_RULES_MAX) {
        Ok(body) => body,
        Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(TomeError::GuardrailsWriteFailed { path: source }),
    };

    if body_contains_marker_line(&body) {
        return Err(TomeError::GuardrailsWriteFailed { path: source });
    }
    Ok(Some(body))
}

/// Whether any line of a verbatim guardrails body is itself a managed marker:
/// a guardrails START or END line, or a `tome:begin/end` block marker. Uses
/// the same compiled regexes the reconciler parses with, so the scan and the
/// parse can never disagree about what counts as a marker.
fn body_contains_marker_line(body: &str) -> bool {
    body.split('\n').any(|line| {
        start_regex().is_match(line)
            || end_regex().is_match(line)
            || block_marker_regex().is_match(line)
    })
}

/// Whether a guardrails reconciliation changed the target on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardrailsAction {
    /// The target gained at least one region (and existed before).
    Updated,
    /// The target was created (an in-file target's first region, or the
    /// Cursor sibling's first contributor).
    Created,
    /// A region was removed (and nothing was written/updated).
    Removed,
    /// No on-disk change.
    LeftAlone,
}

/// Reconcile guardrails regions in an in-file target (`CLAUDE.md`, shared
/// `AGENTS.md`, `GEMINI.md`). `desired` maps `<catalog>:<plugin>` → verbatim
/// body for every plugin contributing to THIS file (suppression already
/// applied by the caller).
///
/// Existing regions are overwritten in place; orphaned regions are removed;
/// new regions are appended in lexicographic key order. Surrounding content
/// (user prose, the `tome:begin/end` block) is preserved verbatim. Atomic,
/// mode-preserving, symlink-refusing write. A render/write failure surfaces
/// [`TomeError::GuardrailsWriteFailed`] (exit 46).
pub fn reconcile_in_file_region(
    target: &Path,
    desired: &BTreeMap<String, String>,
) -> Result<GuardrailsAction, TomeError> {
    rules_file::refuse_symlink(target).map_err(|_| TomeError::GuardrailsWriteFailed {
        path: target.to_path_buf(),
    })?;

    let existing = match crate::util::bounded_read_to_string(target, crate::util::HARNESS_RULES_MAX)
    {
        Ok(s) => Some(s),
        Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => {
            return Err(TomeError::GuardrailsWriteFailed {
                path: target.to_path_buf(),
            });
        }
    };

    // Absent file + nothing desired → nothing to do (do NOT create an empty
    // in-file target; it is developer-authored).
    if existing.is_none() && desired.is_empty() {
        return Ok(GuardrailsAction::LeftAlone);
    }

    let prior = existing.clone().unwrap_or_default();
    let Composed {
        contents: new_contents,
        prior_had_regions,
    } = compose_in_file(target, &prior, desired)?;

    if existing.as_deref() == Some(new_contents.as_str()) {
        return Ok(GuardrailsAction::LeftAlone);
    }

    rules_file::atomic_write(target, new_contents.as_bytes()).map_err(|_| {
        TomeError::GuardrailsWriteFailed {
            path: target.to_path_buf(),
        }
    })?;

    Ok(classify(existing.is_some(), prior_had_regions, desired))
}

/// Reconcile the Cursor standalone sibling (`TOME_GUARDRAILS.md`).
///
/// The file is fully Tome-owned: it is rebuilt from `desired` in lexicographic
/// key order. When `desired` is empty the file is deleted entirely (FR-015).
pub fn reconcile_standalone_sibling(
    target: &Path,
    desired: &BTreeMap<String, String>,
) -> Result<GuardrailsAction, TomeError> {
    rules_file::refuse_symlink(target).map_err(|_| TomeError::GuardrailsWriteFailed {
        path: target.to_path_buf(),
    })?;

    let existed = target.exists();

    if desired.is_empty() {
        // Delete the sibling entirely when no plugin contributes.
        if existed {
            std::fs::remove_file(target).map_err(|_| TomeError::GuardrailsWriteFailed {
                path: target.to_path_buf(),
            })?;
            return Ok(GuardrailsAction::Removed);
        }
        return Ok(GuardrailsAction::LeftAlone);
    }

    let spec = guardrails_spec();
    let mut out = String::new();
    for (key, body) in desired {
        out.push_str(&rules_file::format_marker_region(spec, key, body));
        out.push('\n');
    }

    // Idempotence: identical bytes → no write.
    if existed
        && let Ok(prior) =
            crate::util::bounded_read_to_string(target, crate::util::HARNESS_RULES_MAX)
        && prior == out
    {
        return Ok(GuardrailsAction::LeftAlone);
    }

    rules_file::atomic_write(target, out.as_bytes()).map_err(|_| {
        TomeError::GuardrailsWriteFailed {
            path: target.to_path_buf(),
        }
    })?;

    Ok(if existed {
        GuardrailsAction::Updated
    } else {
        GuardrailsAction::Created
    })
}

/// The product of an in-file compose: the new contents plus whether the prior
/// contents already held guardrails regions (so [`classify`] need not re-parse).
#[derive(Debug)]
struct Composed {
    contents: String,
    prior_had_regions: bool,
}

/// Build the new contents for an in-file target: preserve everything outside
/// guardrails regions, overwrite surviving regions in place, drop orphaned
/// regions (with their preceding blank separator), and append brand-new
/// regions in lexicographic key order.
///
/// `target` is threaded through purely so a parse failure of the EXISTING
/// contents names the real file in [`TomeError::GuardrailsWriteFailed`] — this
/// is the most likely failure (a hand-mangled or marker-poisoned region) and
/// an empty path would be a useless diagnostic.
fn compose_in_file(
    target: &Path,
    existing: &str,
    desired: &BTreeMap<String, String>,
) -> Result<Composed, TomeError> {
    let spec = guardrails_spec();
    let regions = rules_file::find_marker_regions(spec, existing).map_err(|_| {
        TomeError::GuardrailsWriteFailed {
            path: target.to_path_buf(),
        }
    })?;
    let prior_had_regions = !regions.is_empty();

    let lines: Vec<&str> = existing.split('\n').collect();

    // Map each existing region's lines for quick membership tests + the
    // preceding-blank-separator removal (mirrors `rules_file::remove_block`).
    let mut region_by_begin: BTreeMap<usize, &MarkerRegion> = BTreeMap::new();
    for r in &regions {
        region_by_begin.insert(r.begin_line, r);
    }

    let mut emitted: Vec<String> = Vec::with_capacity(lines.len());
    let mut seen_keys: Vec<String> = Vec::new();
    let mut idx = 0;
    while idx < lines.len() {
        if let Some(region) = region_by_begin.get(&idx) {
            let key = &region.key;
            if let Some(body) = desired.get(key) {
                // Surviving region: overwrite body in place.
                emitted.push(begin_marker(key));
                for bl in body.split('\n') {
                    emitted.push(bl.to_string());
                }
                emitted.push(end_marker(key));
                seen_keys.push(key.clone());
            } else {
                // Orphaned region: drop it AND a single immediately-preceding
                // blank separator line, mirroring `remove_block`.
                if let Some(last) = emitted.last()
                    && last.is_empty()
                {
                    emitted.pop();
                }
            }
            idx = region.end_line + 1;
            continue;
        }
        emitted.push(lines[idx].to_string());
        idx += 1;
    }

    // Append brand-new regions (desired but not already present) in lex order.
    // `desired` is a BTreeMap, so iteration is already lexicographic.
    let mut body = emitted.join("\n");
    for (key, region_body) in desired {
        if seen_keys.iter().any(|k| k == key) {
            continue;
        }
        if !body.is_empty() {
            if !body.ends_with('\n') {
                body.push('\n');
            }
            body.push('\n');
        }
        body.push_str(&rules_file::format_marker_region(spec, key, region_body));
        body.push('\n');
    }

    Ok(Composed {
        contents: body,
        prior_had_regions,
    })
}

/// Classify the on-disk change for an in-file target after a write.
///
/// `prior_had_regions` is the parse result already computed by
/// [`compose_in_file`]; reusing it avoids a second full parse (and the
/// swallowed-error it would require).
fn classify(
    existed: bool,
    prior_had_regions: bool,
    desired: &BTreeMap<String, String>,
) -> GuardrailsAction {
    if !existed {
        return GuardrailsAction::Created;
    }
    // The file existed; whether this is an update or a removal depends on
    // whether any region survives in the new content. If `desired` is empty
    // and the prior had regions, this was a pure removal.
    if desired.is_empty() && prior_had_regions {
        GuardrailsAction::Removed
    } else {
        GuardrailsAction::Updated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desired(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// Test shim: compose against a dummy target and return only the contents.
    /// The target path only matters for the parse-error diagnostic (R3-1),
    /// which these compose tests never trigger.
    fn compose(existing: &str, desired: &BTreeMap<String, String>) -> String {
        compose_in_file(Path::new("CLAUDE.md"), existing, desired)
            .unwrap()
            .contents
    }

    #[test]
    fn compose_appends_region_to_empty_file() {
        let out = compose("", &desired(&[("cat:plug", "be careful")]));
        assert!(out.contains("<!-- START GUARDRAILS: cat:plug -->"));
        assert!(out.contains("be careful"));
        assert!(out.contains("<!-- END GUARDRAILS: cat:plug -->"));
    }

    #[test]
    fn compose_preserves_user_prose_and_tome_block() {
        let prior = "# my rules\n\n<!-- tome:begin -->\n@.tome/RULES.md\n<!-- tome:end -->\n";
        let out = compose(prior, &desired(&[("cat:plug", "x")]));
        assert!(out.contains("# my rules"));
        assert!(out.contains("<!-- tome:begin -->"));
        assert!(out.contains("<!-- START GUARDRAILS: cat:plug -->"));
    }

    #[test]
    fn compose_overwrites_body_in_place_not_duplicated() {
        let prior =
            "<!-- START GUARDRAILS: cat:plug -->\nold body\n<!-- END GUARDRAILS: cat:plug -->\n";
        let out = compose(prior, &desired(&[("cat:plug", "new body")]));
        assert!(out.contains("new body"));
        assert!(!out.contains("old body"));
        // Exactly one START marker — not duplicated.
        assert_eq!(
            out.matches("<!-- START GUARDRAILS: cat:plug -->").count(),
            1
        );
    }

    #[test]
    fn compose_removes_orphaned_region() {
        let prior = "<!-- START GUARDRAILS: cat:gone -->\nbye\n<!-- END GUARDRAILS: cat:gone -->\n";
        let out = compose(prior, &desired(&[]));
        assert!(!out.contains("cat:gone"));
        assert!(!out.contains("bye"));
    }

    #[test]
    fn compose_orders_new_regions_lexicographically() {
        let out = compose("", &desired(&[("cat:zeta", "z"), ("cat:alpha", "a")]));
        let alpha = out.find("cat:alpha").unwrap();
        let zeta = out.find("cat:zeta").unwrap();
        assert!(
            alpha < zeta,
            "alpha region must precede zeta region:\n{out}"
        );
    }

    #[test]
    fn compose_is_idempotent_on_second_pass() {
        let d = desired(&[("cat:a", "body a"), ("cat:b", "body b")]);
        let first = compose("", &d);
        let second = compose(&first, &d);
        assert_eq!(first, second, "second compose must be byte-identical");
    }

    #[test]
    fn compose_reports_prior_had_regions() {
        let prior = "<!-- START GUARDRAILS: cat:plug -->\nx\n<!-- END GUARDRAILS: cat:plug -->\n";
        let with_regions = compose_in_file(Path::new("CLAUDE.md"), prior, &desired(&[])).unwrap();
        assert!(
            with_regions.prior_had_regions,
            "a prior with a region must report prior_had_regions = true"
        );
        let no_regions =
            compose_in_file(Path::new("CLAUDE.md"), "# just prose\n", &desired(&[])).unwrap();
        assert!(
            !no_regions.prior_had_regions,
            "a prior without regions must report prior_had_regions = false"
        );
    }

    #[test]
    fn compose_parse_error_names_the_target() {
        // A stray END line with no open START is malformed; the error must
        // carry the real target path, not an empty PathBuf (R3-1).
        let prior = "<!-- END GUARDRAILS: cat:plug -->\n";
        let err = compose_in_file(Path::new("CLAUDE.md"), prior, &desired(&[]))
            .expect_err("a stray END must be malformed");
        match err {
            TomeError::GuardrailsWriteFailed { path } => {
                assert_eq!(path, Path::new("CLAUDE.md"), "error must name the target");
            }
            other => panic!("expected GuardrailsWriteFailed, got {other:?}"),
        }
    }

    #[test]
    fn region_key_joins_catalog_and_plugin() {
        assert_eq!(region_key("cat", "plug"), "cat:plug");
    }

    // ----- B-1: fail-closed marker validation in the source body -----

    #[test]
    fn body_with_guardrails_start_line_is_rejected() {
        assert!(body_contains_marker_line(
            "ok\n<!-- START GUARDRAILS: x:y -->\nmore\n"
        ));
    }

    #[test]
    fn body_with_guardrails_end_line_is_rejected() {
        assert!(body_contains_marker_line(
            "prose\n<!-- END GUARDRAILS: c:p -->\n"
        ));
    }

    #[test]
    fn body_with_tome_block_marker_is_rejected() {
        assert!(body_contains_marker_line("intro\n<!-- tome:begin -->\n"));
        assert!(body_contains_marker_line("intro\n<!-- tome:end -->\n"));
    }

    #[test]
    fn body_with_marker_and_trailing_whitespace_is_rejected() {
        // The marker regexes tolerate trailing whitespace; the scan must too.
        assert!(body_contains_marker_line(
            "<!-- START GUARDRAILS: x:y -->   \n"
        ));
    }

    #[test]
    fn ordinary_body_passes_validation() {
        let body = "---\ntitle: My rules\n---\n# Heading\n@some/include\nBe careful.   \n";
        assert!(
            !body_contains_marker_line(body),
            "a body with frontmatter, a heading, an include, and trailing ws is fine"
        );
    }

    #[test]
    fn read_source_rejects_marker_poisoned_body() {
        let dir = tempfile::tempdir().expect("tempdir");
        let hooks = dir.path().join("hooks");
        std::fs::create_dir_all(&hooks).expect("mkdir hooks");
        std::fs::write(
            hooks.join("GUARDRAILS.md"),
            "be careful\n<!-- END GUARDRAILS: c:p -->\n",
        )
        .expect("write source");

        let err = read_guardrails_source(dir.path()).expect_err("poisoned body must be rejected");
        match err {
            TomeError::GuardrailsWriteFailed { path } => {
                assert_eq!(path, hooks.join("GUARDRAILS.md"), "error names the source");
            }
            other => panic!("expected GuardrailsWriteFailed, got {other:?}"),
        }
    }

    #[test]
    fn read_source_accepts_clean_body() {
        let dir = tempfile::tempdir().expect("tempdir");
        let hooks = dir.path().join("hooks");
        std::fs::create_dir_all(&hooks).expect("mkdir hooks");
        std::fs::write(hooks.join("GUARDRAILS.md"), "be careful with deletes\n")
            .expect("write source");
        let body = read_guardrails_source(dir.path()).expect("clean body reads");
        assert_eq!(body.as_deref(), Some("be careful with deletes\n"));
    }
}
