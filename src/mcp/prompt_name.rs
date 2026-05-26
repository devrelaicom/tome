//! Phase 5 — prompt-name derivation for MCP prompts.
//!
//! Algorithm: `<plugin>__<entry>`, with each portion sanitised and
//! truncated independently. A frontmatter `prompt_name` override
//! replaces BOTH portions (no separator inserted; the override IS the
//! entire Tome contribution per the contract).
//!
//! Pure functions only — no I/O, no rmcp.
//!
//! Contract: `specs/005-phase-5-commands-prompts/contracts/mcp-prompts.md`
//! § Prompt name derivation.

/// Maximum length of the `<plugin>` portion of the derived prompt name.
/// Per NFR-003.
pub const PLUGIN_PORTION_MAX: usize = 16;

/// Maximum length of the `<entry>` portion of the derived prompt name.
pub const ENTRY_PORTION_MAX: usize = 32;

/// Maximum length of the override-replacement prompt name. The override
/// substitutes for BOTH portions, so its budget is the sum of the per-
/// portion caps (16 + 32 = 48) minus the would-be separator length.
pub const OVERRIDE_MAX: usize = 48;

/// Separator joining the plugin portion to the entry-name portion.
pub const SEPARATOR: &str = "__";

/// Sanitise a free-form string into the prompt-name character set:
///
/// 1. ASCII-lowercase every character.
/// 2. Replace any character not in `[a-z0-9_-]` with `_`.
/// 3. Collapse runs of `_` into a single `_`.
/// 4. Strip leading and trailing `_`.
///
/// The contract names `[a-z0-9_-]` as the surviving character set;
/// hyphens are preserved (Claude Code's harness accepts them).
pub fn sanitise(input: &str) -> String {
    let lowered: String = input
        .chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();

    // Collapse runs of `_`.
    let mut collapsed = String::with_capacity(lowered.len());
    let mut prev_underscore = false;
    for c in lowered.chars() {
        let is_underscore = c == '_';
        if is_underscore && prev_underscore {
            continue;
        }
        collapsed.push(c);
        prev_underscore = is_underscore;
    }

    // Strip leading / trailing `_`.
    collapsed.trim_matches('_').to_owned()
}

/// Sanitise + truncate at the char boundary to at most `max` characters.
/// Any trailing `_` left behind by the truncation step is stripped.
pub fn sanitise_trunc(input: &str, max: usize) -> String {
    let s = sanitise(input);
    if s.chars().count() <= max {
        return s;
    }
    let truncated: String = s.chars().take(max).collect();
    truncated.trim_end_matches('_').to_owned()
}

/// Derive the Tome-side prompt name for an entry.
///
/// When `name_override` is `Some`, the override replaces BOTH portions —
/// the `__` separator is NOT inserted. The override is sanitised and
/// truncated to [`OVERRIDE_MAX`] characters.
///
/// When `name_override` is `None`, the standard `<plugin>__<entry>`
/// composition is built with each portion sanitised + truncated to its
/// per-portion cap.
pub fn derive_name(plugin: &str, entry: &str, name_override: Option<&str>) -> String {
    if let Some(o) = name_override {
        return sanitise_trunc(o, OVERRIDE_MAX);
    }
    let p = sanitise_trunc(plugin, PLUGIN_PORTION_MAX);
    let e = sanitise_trunc(entry, ENTRY_PORTION_MAX);
    format!("{p}{SEPARATOR}{e}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitise_lowercases_ascii() {
        assert_eq!(sanitise("FooBar"), "foobar");
    }

    #[test]
    fn sanitise_replaces_non_word_chars_with_underscore() {
        assert_eq!(sanitise("foo.bar"), "foo_bar");
        assert_eq!(sanitise("foo bar"), "foo_bar");
    }

    #[test]
    fn sanitise_collapses_runs_of_underscores() {
        assert_eq!(sanitise("foo___bar"), "foo_bar");
        assert_eq!(sanitise("foo...bar"), "foo_bar");
    }

    #[test]
    fn sanitise_strips_leading_and_trailing_underscores() {
        assert_eq!(sanitise("__foo__"), "foo");
        assert_eq!(sanitise(".foo."), "foo");
    }

    #[test]
    fn sanitise_preserves_hyphens() {
        assert_eq!(sanitise("review-my-pr"), "review-my-pr");
    }

    #[test]
    fn derive_name_uses_separator_between_portions() {
        assert_eq!(
            derive_name("midnight-expert", "fix-issue", None),
            "midnight-expert__fix-issue"
        );
    }

    #[test]
    fn derive_name_override_replaces_both_portions() {
        assert_eq!(
            derive_name("midnight-expert", "fix-issue", Some("review-my-pr")),
            "review-my-pr"
        );
    }
}
