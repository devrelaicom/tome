//! Phase 5 / US1.b — prompt-name derivation.
//!
//! Covers the contract's `Prompt name derivation` section: sanitisation,
//! per-portion truncation, and the override-replaces-both-portions
//! shape. Aligned to `contracts/mcp-prompts.md` and
//! `src/mcp/prompt_name.rs`.

use tome::mcp::prompt_name::{
    ENTRY_PORTION_MAX, OVERRIDE_MAX, PLUGIN_PORTION_MAX, SEPARATOR, derive_name, sanitise,
    sanitise_trunc,
};

#[test]
fn sanitise_lowercases_ascii_and_replaces_non_word_chars() {
    assert_eq!(sanitise("FooBar"), "foobar");
    assert_eq!(sanitise("Foo Bar"), "foo_bar");
    assert_eq!(sanitise("foo.bar.baz"), "foo_bar_baz");
}

#[test]
fn sanitise_collapses_runs_of_underscores() {
    assert_eq!(sanitise("foo___bar"), "foo_bar");
    assert_eq!(sanitise("a..b..c"), "a_b_c");
}

#[test]
fn sanitise_strips_leading_and_trailing_underscores() {
    assert_eq!(sanitise("__foo__"), "foo");
    assert_eq!(sanitise("...bar..."), "bar");
}

#[test]
fn sanitise_preserves_hyphens() {
    // Per the contract, the surviving character set is `[a-z0-9_-]`.
    assert_eq!(sanitise("review-my-pr"), "review-my-pr");
    assert_eq!(
        sanitise("MidnightExpert-CompactDev"),
        "midnightexpert-compactdev"
    );
}

#[test]
fn sanitise_trunc_truncates_at_char_boundary_and_strips_trailing_underscores() {
    // Truncate then strip trailing underscores left behind.
    let s = "abc__def__ghi";
    let t = sanitise_trunc(s, 5);
    // sanitise("abc__def__ghi") == "abc_def_ghi"; trim to 5 -> "abc_d"
    assert_eq!(t, "abc_d");

    // A truncation that lands on an underscore drops it.
    let t2 = sanitise_trunc("abcde_fgh", 6);
    // sanitise unchanged "abcde_fgh"; trim to 6 -> "abcde_" -> "abcde"
    assert_eq!(t2, "abcde");
}

#[test]
fn derive_name_composes_plugin_and_entry_with_separator() {
    assert_eq!(
        derive_name("midnight-expert", "fix-issue", None),
        format!("midnight-expert{SEPARATOR}fix-issue")
    );
}

#[test]
fn derive_name_truncates_plugin_portion_at_cap() {
    let very_long_plugin = "this-is-a-very-long-plugin-name-that-exceeds-the-cap";
    let out = derive_name(very_long_plugin, "x", None);
    // Plugin portion is sanitised + truncated to PLUGIN_PORTION_MAX.
    let (head, sep_tail) = out.split_once(SEPARATOR).expect("contains separator");
    assert!(
        head.chars().count() <= PLUGIN_PORTION_MAX,
        "plugin portion `{head}` exceeds cap {PLUGIN_PORTION_MAX}"
    );
    assert_eq!(sep_tail, "x");
}

#[test]
fn derive_name_truncates_entry_portion_at_cap() {
    let long_entry = "a-very-long-entry-name-that-pushes-past-the-cap-by-a-lot";
    let out = derive_name("p", long_entry, None);
    let (head, tail) = out.split_once(SEPARATOR).expect("contains separator");
    assert_eq!(head, "p");
    assert!(
        tail.chars().count() <= ENTRY_PORTION_MAX,
        "entry portion `{tail}` exceeds cap {ENTRY_PORTION_MAX}"
    );
}

#[test]
fn derive_name_override_replaces_both_portions_without_separator() {
    // Override does NOT inject the `__` separator — it IS the entire
    // Tome contribution.
    let out = derive_name("p-very-long", "e-also-long", Some("review-my-pr"));
    assert_eq!(out, "review-my-pr");
    assert!(!out.contains(SEPARATOR));
}

#[test]
fn derive_name_override_is_sanitised_and_truncated_to_combined_cap() {
    // Override sanitised + truncated to OVERRIDE_MAX.
    let long = "An-Override-That-Is-Definitely-Way-Beyond-The-Combined-Cap-XXX";
    let out = derive_name("p", "e", Some(long));
    assert!(
        out.chars().count() <= OVERRIDE_MAX,
        "override-derived name `{out}` exceeds cap {OVERRIDE_MAX}"
    );
    // Lowercased and word-char-set-only.
    for ch in out.chars() {
        assert!(
            ch.is_ascii_alphanumeric() || ch == '_' || ch == '-',
            "unexpected char `{ch}` in `{out}`"
        );
    }
}
