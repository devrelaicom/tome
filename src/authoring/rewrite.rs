//! Harness-ism rewrite — literal-token swaps over body strings (the
//! `data-model.md §7` table): `${CLAUDE_PLUGIN_ROOT}` → `${TOME_PLUGIN_DIR}`,
//! `${CLAUDE_PROJECT_DIR}` → `${TOME_PROJECT_DIR}`, legacy 1-based `$1..$9` →
//! Tome's 0-based positional form (for Claude Code *command* sources), and
//! warn-unmappable for the rest (`${CLAUDE_SESSION_ID}`, `` !`cmd` ``, `@file`,
//! `${user_config.*}`, …).
//!
//! Shared by `convert` (which applies the rewrite) and `lint --autofix` (which
//! applies the rewritable subset). This is a plain regex/substring pass over
//! known tokens — **not** Tome's runtime substitution engine, so there is no
//! single-sweep concern. Callers must pass valid UTF-8 (fail-closed upstream,
//! FR-011a): a token rewrite over a lossily-decoded body would corrupt
//! U+FFFD'd bytes into the deterministic snapshot output.
//!
//! [`rewrite_body`] returns the rewritten text plus location-less, fix-less
//! [`Diagnostic`]s describing each harness-ism found; the caller (`convert`'s
//! report or `lint`'s rule) attaches a location + autofix as appropriate.

use std::sync::LazyLock;

use regex::Regex;

use crate::authoring::ir::{Diagnostic, Severity};

// Rule ids — stable identifiers surfaced in `--json` findings and reused by the
// lint registry's "residual harness-isms" rule.
pub mod rule {
    pub const PLUGIN_ROOT: &str = "harness-ism/claude-plugin-root";
    pub const PLUGIN_DATA: &str = "harness-ism/claude-plugin-data";
    pub const SKILL_DIR: &str = "harness-ism/claude-skill-dir";
    pub const PROJECT_DIR: &str = "harness-ism/claude-project-dir";
    pub const LEGACY_POSITIONAL: &str = "harness-ism/legacy-positional";
    pub const SESSION_ID: &str = "harness-ism/claude-session-id";
    pub const EFFORT: &str = "harness-ism/claude-effort";
    pub const USER_CONFIG: &str = "harness-ism/user-config";
    pub const SHELL_EXEC: &str = "harness-ism/shell-exec";
    pub const FILE_REF: &str = "harness-ism/file-ref";
    pub const CLAUDECODE_PROBE: &str = "harness-ism/claudecode-probe";
}

/// Options controlling source-kind-specific rewrites.
#[derive(Debug, Clone, Copy, Default)]
pub struct RewriteOptions {
    /// True when the source is a Claude Code *command* (legacy `$1..$9` are
    /// 1-based positional args). Enables the `$1..$9` → 0-based rewrite. When
    /// false (e.g. `lint`, which has no source context), legacy positionals
    /// are flagged but not rewritten — their intent is ambiguous.
    pub legacy_command_args: bool,
}

/// Result of a rewrite: the (possibly) rewritten text + the harness-isms found.
#[derive(Debug, Clone)]
pub struct RewriteOutcome {
    pub text: String,
    pub diagnostics: Vec<Diagnostic>,
}

impl RewriteOutcome {
    /// True when the rewrite changed the text.
    pub fn changed(&self, original: &str) -> bool {
        self.text != original
    }
}

/// The four rewritable `${CLAUDE_*}` → `${TOME_*}` literal swaps (§7).
const REWRITES: &[(&str, &str, &str)] = &[
    (
        "${CLAUDE_PLUGIN_ROOT}",
        "${TOME_PLUGIN_DIR}",
        rule::PLUGIN_ROOT,
    ),
    (
        "${CLAUDE_PLUGIN_DATA}",
        "${TOME_PLUGIN_DATA}",
        rule::PLUGIN_DATA,
    ),
    ("${CLAUDE_SKILL_DIR}", "${TOME_SKILL_DIR}", rule::SKILL_DIR),
    (
        "${CLAUDE_PROJECT_DIR}",
        "${TOME_PROJECT_DIR}",
        rule::PROJECT_DIR,
    ),
];

/// Unmappable literal tokens (no Tome equivalent) — flagged, never rewritten.
const UNMAPPABLE_LITERALS: &[(&str, &str)] = &[
    ("${CLAUDE_SESSION_ID}", rule::SESSION_ID),
    ("${CLAUDE_EFFORT}", rule::EFFORT),
];

// Legacy 1-based positional `$1..$9`. The `\b` after the digit avoids matching
// `$10`+ (a word boundary requires a non-word char or end) and the `1` inside
// `$ARGUMENTS[1]` (preceded by `[`, not `$`). The `regex` crate has no
// lookaround, so the word boundary is how we exclude multi-digit forms.
static LEGACY_POSITIONAL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$([1-9])\b").expect("valid regex"));

// `${user_config.X}` or `CLAUDE_PLUGIN_OPTION_X` — plugin option references.
static USER_CONFIG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\$\{user_config\.[A-Za-z0-9_]+\}|CLAUDE_PLUGIN_OPTION_[A-Za-z0-9_]+")
        .expect("valid regex")
});

// `` !`cmd` `` — Claude Code "run command, inject output". Tome does not execute.
static SHELL_EXEC_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"!`[^`]*`").expect("valid regex"));

// `@file` references — `@` after whitespace/start followed by a path containing
// a `/` (so `a@b.com` emails and `@mention`s are not matched). Tome does not
// inject file contents.
static FILE_REF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|\s)@(?:\.{0,2}/[\w./\-]+|[\w\-]+/[\w./\-]+)").expect("valid regex")
});

/// Apply the harness-ism rewrite to `text`, returning the rewritten text + the
/// diagnostics describing every harness-ism found.
pub fn rewrite_body(text: &str, opts: RewriteOptions) -> RewriteOutcome {
    let mut out = text.to_owned();
    let mut diagnostics = Vec::new();

    // 1. Rewritable `${CLAUDE_*}` → `${TOME_*}` (Warning; the rewrite is applied).
    for (from, to, rule_id) in REWRITES {
        let count = out.matches(from).count();
        if count > 0 {
            out = out.replace(from, to);
            diagnostics.push(Diagnostic::warning(
                rule_id,
                format!("rewrote {count} occurrence(s) of `{from}` → `{to}`"),
            ));
        }
    }

    // 2. Legacy 1-based positional `$1..$9`.
    let legacy_count = LEGACY_POSITIONAL_RE.find_iter(&out).count();
    if legacy_count > 0 {
        if opts.legacy_command_args {
            // 1-based → 0-based: $K → $(K-1). Single pass; the closure maps each
            // original digit independently (no re-scan of its own output).
            out = LEGACY_POSITIONAL_RE
                .replace_all(&out, |caps: &regex::Captures<'_>| {
                    let d: u32 = caps[1].parse().expect("regex guarantees 1..9");
                    format!("${}", d - 1)
                })
                .into_owned();
            diagnostics.push(Diagnostic::warning(
                rule::LEGACY_POSITIONAL,
                format!(
                    "rewrote {legacy_count} legacy 1-based positional argument(s) `$1..$9` → Tome's 0-based form"
                ),
            ));
        } else {
            diagnostics.push(Diagnostic::warning(
                rule::LEGACY_POSITIONAL,
                format!(
                    "found {legacy_count} ambiguous legacy positional argument(s) `$1..$9`; Tome positionals are 0-based (`$0`-based) — verify intent"
                ),
            ));
        }
    }

    // 3. Unmappable literal tokens (Warning; not rewritten).
    for (token, rule_id) in UNMAPPABLE_LITERALS {
        let count = out.matches(token).count();
        if count > 0 {
            diagnostics.push(Diagnostic::warning(
                rule_id,
                format!("`{token}` has no Tome equivalent — remove or replace it manually"),
            ));
        }
    }

    // 4. Pattern-matched unmappables (Warning; not rewritten).
    if USER_CONFIG_RE.is_match(&out) {
        diagnostics.push(Diagnostic::warning(
            rule::USER_CONFIG,
            "plugin-option reference (`${user_config.*}` / `CLAUDE_PLUGIN_OPTION_*`) has no Tome equivalent",
        ));
    }
    if SHELL_EXEC_RE.is_match(&out) {
        diagnostics.push(Diagnostic::warning(
            rule::SHELL_EXEC,
            "shell-execution injection (`` !`cmd` ``) is not supported — Tome does not execute commands in bodies",
        ));
    }
    if FILE_REF_RE.is_match(&out) {
        diagnostics.push(Diagnostic::warning(
            rule::FILE_REF,
            "file-reference injection (`@path`) is not supported — Tome does not inject file contents",
        ));
    }

    // 5. The `$CLAUDECODE` env probe (Info — harmless, just noted).
    if out.contains("$CLAUDECODE") {
        diagnostics.push(Diagnostic::info(
            rule::CLAUDECODE_PROBE,
            "the `$CLAUDECODE` environment probe has no Tome equivalent (Tome is not Claude Code)",
        ));
    }

    RewriteOutcome {
        text: out,
        diagnostics,
    }
}

/// True for the rule ids that mean "Tome cannot represent this" — so
/// `convert --strict` aborts and `lint` treats them as residual harness-isms.
/// The four rewritable tokens are deliberately excluded (they are handled by
/// the rewrite). The legacy-positional id is excluded too: it is mechanically
/// rewritten on `convert` and only advisory on `lint`.
pub fn is_unsupported_harness_ism(rule_id: &str) -> bool {
    matches!(
        rule_id,
        rule::SESSION_ID | rule::EFFORT | rule::USER_CONFIG | rule::SHELL_EXEC | rule::FILE_REF
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rewrite(text: &str) -> RewriteOutcome {
        rewrite_body(text, RewriteOptions::default())
    }

    fn rewrite_cmd(text: &str) -> RewriteOutcome {
        rewrite_body(
            text,
            RewriteOptions {
                legacy_command_args: true,
            },
        )
    }

    fn has_rule(o: &RewriteOutcome, id: &str) -> bool {
        o.diagnostics.iter().any(|d| d.rule_id == id)
    }

    #[test]
    fn rewrites_plugin_root() {
        let o = rewrite("see ${CLAUDE_PLUGIN_ROOT}/x and ${CLAUDE_PLUGIN_ROOT}/y");
        assert_eq!(o.text, "see ${TOME_PLUGIN_DIR}/x and ${TOME_PLUGIN_DIR}/y");
        assert!(has_rule(&o, rule::PLUGIN_ROOT));
    }

    #[test]
    fn rewrites_all_four_claude_vars() {
        let o = rewrite(
            "${CLAUDE_PLUGIN_ROOT} ${CLAUDE_PLUGIN_DATA} ${CLAUDE_SKILL_DIR} ${CLAUDE_PROJECT_DIR}",
        );
        assert_eq!(
            o.text,
            "${TOME_PLUGIN_DIR} ${TOME_PLUGIN_DATA} ${TOME_SKILL_DIR} ${TOME_PROJECT_DIR}"
        );
        for id in [
            rule::PLUGIN_ROOT,
            rule::PLUGIN_DATA,
            rule::SKILL_DIR,
            rule::PROJECT_DIR,
        ] {
            assert!(has_rule(&o, id), "missing diagnostic for {id}");
        }
    }

    #[test]
    fn legacy_positional_rewritten_for_commands() {
        let o = rewrite_cmd("$1 then $2 then $9");
        assert_eq!(o.text, "$0 then $1 then $8");
        assert!(has_rule(&o, rule::LEGACY_POSITIONAL));
    }

    #[test]
    fn legacy_positional_only_warns_without_command_context() {
        let o = rewrite("$1 then $2");
        assert_eq!(o.text, "$1 then $2", "no rewrite without command context");
        assert!(has_rule(&o, rule::LEGACY_POSITIONAL));
    }

    #[test]
    fn legacy_positional_does_not_touch_arguments_index_or_multidigit() {
        let o = rewrite_cmd("$ARGUMENTS[1] and $10 and $name");
        assert_eq!(o.text, "$ARGUMENTS[1] and $10 and $name");
    }

    #[test]
    fn unmappable_session_id_and_effort_warn() {
        let o = rewrite("${CLAUDE_SESSION_ID} ${CLAUDE_EFFORT}");
        assert!(has_rule(&o, rule::SESSION_ID));
        assert!(has_rule(&o, rule::EFFORT));
        assert!(
            o.text.contains("${CLAUDE_SESSION_ID}"),
            "unmappable not rewritten"
        );
    }

    #[test]
    fn user_config_warns() {
        assert!(has_rule(
            &rewrite("x ${user_config.api_key} y"),
            rule::USER_CONFIG
        ));
        assert!(has_rule(
            &rewrite("x CLAUDE_PLUGIN_OPTION_TOKEN y"),
            rule::USER_CONFIG
        ));
    }

    #[test]
    fn shell_exec_warns() {
        assert!(has_rule(&rewrite("run !`ls -la` now"), rule::SHELL_EXEC));
    }

    #[test]
    fn file_ref_warns_but_not_emails() {
        assert!(has_rule(&rewrite("see @./docs/x.md"), rule::FILE_REF));
        assert!(has_rule(&rewrite("see @src/lib.rs"), rule::FILE_REF));
        // An email address must NOT trip the file-ref heuristic.
        assert!(!has_rule(&rewrite("mail a@b.com"), rule::FILE_REF));
        // A bare @mention must NOT trip it.
        assert!(!has_rule(&rewrite("cc @teammate"), rule::FILE_REF));
    }

    #[test]
    fn claudecode_probe_is_info() {
        let o = rewrite("if [ -n \"$CLAUDECODE\" ]");
        let d = o
            .diagnostics
            .iter()
            .find(|d| d.rule_id == rule::CLAUDECODE_PROBE)
            .expect("probe diagnostic");
        assert_eq!(d.severity, Severity::Info);
    }

    #[test]
    fn clean_body_has_no_diagnostics() {
        let o = rewrite("# Title\n\nJust normal prose with $ARGUMENTS and ${TOME_SKILL_DIR}.\n");
        assert!(o.diagnostics.is_empty(), "got {:?}", o.diagnostics);
        assert!(
            !o.changed("# Title\n\nJust normal prose with $ARGUMENTS and ${TOME_SKILL_DIR}.\n")
        );
    }

    #[test]
    fn unsupported_classifier() {
        assert!(is_unsupported_harness_ism(rule::SESSION_ID));
        assert!(is_unsupported_harness_ism(rule::SHELL_EXEC));
        assert!(!is_unsupported_harness_ism(rule::PLUGIN_ROOT));
        assert!(!is_unsupported_harness_ism(rule::LEGACY_POSITIONAL));
    }
}
