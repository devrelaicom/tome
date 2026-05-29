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

/// Derive a `<base>-<suffix>` persona-style prompt name that always ends
/// in `-<suffix>`, applying the Phase 5 sanitisation and keeping the
/// whole thing within [`OVERRIDE_MAX`] chars.
///
/// Phase 6 / US4 (C4-2): the prior `derive_name(base, "persona",
/// Some("{base}-persona"))` routed the whole `<base>-persona` override
/// through [`sanitise_trunc`] at 48 chars, which amputates the `-persona`
/// suffix for a long base (e.g. `…-perso`). The user-facing contract is
/// that a persona prompt is named `<name>-persona`; the suffix is
/// load-bearing. So we sanitise the suffix, reserve room for it (plus the
/// single joining `-`), then truncate ONLY the base portion to fill the
/// remainder — the result is sanitised end-to-end and always terminates
/// in `-<suffix>`.
///
/// When the sanitised suffix alone already meets or exceeds the budget
/// (degenerate / not expected in practice — `persona` is 7 chars) the
/// suffix is returned truncated on its own; correctness of the tail
/// shape is preserved over including any base characters.
pub fn derive_suffixed_name(base: &str, suffix: &str) -> String {
    let suffix = sanitise_trunc(suffix, OVERRIDE_MAX);
    if suffix.is_empty() {
        // No usable suffix — fall back to a plain truncated base.
        return sanitise_trunc(base, OVERRIDE_MAX);
    }
    // Budget for the base = total cap minus the suffix and the joining
    // `-`. `saturating_sub` keeps us safe if the suffix consumed it all.
    let suffix_len = suffix.chars().count();
    let base_budget = OVERRIDE_MAX.saturating_sub(suffix_len + 1);
    if base_budget == 0 {
        // Nothing left for the base — emit the suffix alone (still a
        // valid, suffix-terminated slug, just with no base).
        return suffix;
    }
    let base = sanitise_trunc(base, base_budget);
    if base.is_empty() {
        return suffix;
    }
    format!("{base}-{suffix}")
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

    #[test]
    fn suffixed_name_short_base_keeps_full_shape() {
        assert_eq!(
            derive_suffixed_name("reviewer", "persona"),
            "reviewer-persona"
        );
    }

    #[test]
    fn suffixed_name_preserves_suffix_for_long_base() {
        // A base longer than the 48-char budget must NOT amputate the
        // `-persona` suffix (C4-2). The base is truncated to fill the
        // remainder; the slug always terminates in `-persona`.
        let long_base = "a".repeat(80);
        let name = derive_suffixed_name(&long_base, "persona");
        assert!(name.ends_with("-persona"), "suffix preserved; got {name:?}",);
        assert!(
            name.chars().count() <= OVERRIDE_MAX,
            "within the override cap; got {} chars: {name:?}",
            name.chars().count(),
        );
        // base budget = 48 - (7 + 1) = 40 `a`s, then `-persona`.
        assert_eq!(name, format!("{}-persona", "a".repeat(40)));
    }

    #[test]
    fn suffixed_name_sanitises_base() {
        assert_eq!(
            derive_suffixed_name("My Plugin.Name", "persona"),
            "my_plugin_name-persona",
        );
    }
}
