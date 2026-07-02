//! Reusable plugin selection: variadic tokens, `*` wildcard globs, and a
//! `--catalog` scope, resolved against a candidate set of [`PluginId`]s.
//!
//! This is the FLAGSHIP helper for issue #314 (`plugin enable`/`disable`) and
//! is intentionally general so #316 (`reindex`) and #317 (tier bulk ops) reuse
//! it verbatim — hence the deliberately small, well-documented public surface:
//!
//! * [`glob_match`] — a hand-rolled `*`-only wildcard matcher (ZERO new
//!   dependency; the project forbids adding a glob crate). `*` matches
//!   zero-or-more of any character; every other char is literal.
//! * [`resolve`] — turn a list of user tokens into a deduped, ordered list of
//!   [`PluginId`]s plus a list of [`SelectorError`]s. It NEVER short-circuits:
//!   a bad token records an error and the rest keep resolving, so the caller
//!   can implement forward-progress ("proceed with the matches, surface the
//!   first error's exit code").
//!
//! Selection is a pure computation: [`resolve`] performs no I/O and does not
//! consult the filesystem or index. Existence of a slash-qualified literal id
//! is deliberately NOT checked here — that stays downstream at
//! `resolve_plugin_dir`, preserving the pre-#314 exit codes (a typo'd literal
//! id resolves to a `PluginId`, which the command then rejects with the same
//! `PluginNotFound` / `CatalogNotFound` it always did).

use crate::plugin::identity::PluginId;

/// Match `name` against `pattern`, where `*` in `pattern` matches zero-or-more
/// of ANY character and every other character is a literal. No other
/// metacharacters are special (`?`, `[`, `.` are all literal).
///
/// Semantics by shape:
/// * no `*` → exact equality (`glob_match("foo", "foo")` is `true`, `"foo"` vs
///   `"foobar"` is `false`);
/// * `*` alone → matches everything, including the empty string;
/// * leading / trailing / middle / multiple `*` all behave as expected
///   (`"*-expert"`, `"compact-*"`, `"a*b*c"`).
///
/// Implementation: an iterative two-pointer matcher with backtracking (the
/// classic linear-space, worst-case-quadratic wildcard algorithm) — NOT regex,
/// so there is nothing to escape and no ReDoS surface. Bytes, not chars: `*`
/// matches whole UTF-8 sequences transparently because a multi-byte character's
/// bytes are simply consumed by the `*`'s "advance by one" step, and literal
/// runs compare byte-for-byte (which is equivalent to char-for-char equality
/// for valid UTF-8).
pub fn glob_match(pattern: &str, name: &str) -> bool {
    let pat = pattern.as_bytes();
    let text = name.as_bytes();

    let (mut p, mut t) = (0usize, 0usize);
    // `star`/`mark` remember the most recent `*` position in the pattern and
    // the text index it was first matched against, so a later literal mismatch
    // can backtrack: let that `*` swallow one more character and retry.
    let mut star: Option<usize> = None;
    let mut mark = 0usize;

    while t < text.len() {
        if p < pat.len() && pat[p] == b'*' {
            // Record this `*` and provisionally match zero characters.
            star = Some(p);
            mark = t;
            p += 1;
        } else if p < pat.len() && pat[p] == text[t] {
            // Literal match: advance both.
            p += 1;
            t += 1;
        } else if let Some(sp) = star {
            // Mismatch, but a prior `*` can absorb one more character: reset the
            // pattern to just past that `*` and advance the text by one.
            p = sp + 1;
            mark += 1;
            t = mark;
        } else {
            // Mismatch with no `*` to fall back on.
            return false;
        }
    }

    // Text exhausted: the remaining pattern must be all `*` to match.
    while p < pat.len() && pat[p] == b'*' {
        p += 1;
    }
    p == pat.len()
}

/// Why a token failed to resolve. Mapped to a closed-set [`TomeError`] at the
/// COMMAND boundary (`SelectorError::into_tome_error`) — never a new exit code.
///
/// [`TomeError`]: crate::error::TomeError
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectorError {
    /// A bare literal name matched a plugin in more than one enrolled catalog.
    /// `candidates` lists the full `catalog/plugin` ids so the user can pick.
    Ambiguous {
        plugin: String,
        candidates: Vec<String>,
    },
    /// A glob token (one containing `*`) matched zero candidates. The pattern is
    /// echoed back verbatim; a zero-match glob is an ERROR, never a silent
    /// no-op (a user who typed `midnight/*` and got nothing wants to know).
    NoGlobMatch { pattern: String },
    /// A bare literal name matched no plugin in any enrolled catalog.
    NotFound { plugin: String },
}

impl SelectorError {
    /// Map onto the closed [`TomeError`] set, reusing existing variants (no new
    /// exit code per the constitution's closed-error-set rule):
    ///
    /// * [`SelectorError::Ambiguous`] → [`TomeError::Usage`] (exit 2) — the user
    ///   must disambiguate; the message lists every `catalog/plugin` candidate.
    /// * [`SelectorError::NoGlobMatch`] → [`TomeError::Usage`] (exit 2) — a
    ///   pattern that matched nothing is a usage-level mistake; the message
    ///   echoes the pattern.
    /// * [`SelectorError::NotFound`] → [`TomeError::PluginNotFound`] (exit 20) —
    ///   this is exactly the "no such plugin" case that variant names, so it
    ///   maps there rather than to a generic `Usage`, preserving the exit code a
    ///   single bare-name typo produced before #314.
    ///
    /// [`TomeError`]: crate::error::TomeError
    /// [`TomeError::Usage`]: crate::error::TomeError::Usage
    /// [`TomeError::PluginNotFound`]: crate::error::TomeError::PluginNotFound
    pub fn into_tome_error(self) -> crate::error::TomeError {
        use crate::error::TomeError;
        match self {
            SelectorError::Ambiguous { plugin, candidates } => TomeError::Usage(format!(
                "plugin name `{plugin}` is ambiguous across catalogs: {}\n\
                 hint: qualify it as `<catalog>/{plugin}`, or scope the command with `--catalog <name>`",
                candidates.join(", "),
            )),
            SelectorError::NoGlobMatch { pattern } => TomeError::Usage(format!(
                "pattern `{pattern}` matched no plugins\n\
                 hint: run `tome plugin list` to see available `<catalog>/<plugin>` ids",
            )),
            SelectorError::NotFound { plugin } => TomeError::PluginNotFound(plugin),
        }
    }
}

/// The outcome of [`resolve`]: the deduped, order-preserving list of matched
/// ids, plus every token that failed to resolve.
///
/// `matched` and `errors` are independent: a batch can have both (some tokens
/// matched, others didn't). The caller decides the policy — the #314 commands
/// proceed with `matched` when it is non-empty and surface the first mapped
/// error's exit code (forward-progress), or fail loudly when `matched` is
/// empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolution {
    /// Matched plugin ids, first-seen order, deduped across tokens.
    pub matched: Vec<PluginId>,
    /// Per-token resolution failures, in token order.
    pub errors: Vec<SelectorError>,
}

/// Resolve `tokens` against `candidates`, scoped by an optional `catalog`.
///
/// Per-token rules (a token is classified by whether it contains `*` and `/`):
///
/// 1. **Glob** (contains `*`): split on the first `/` into `(cat_pat,
///    plug_pat)`; with no `/`, `plug_pat = token` and `cat_pat` is the
///    `catalog` value (matched exactly) when `Some`, else `*` (all catalogs).
///    Every candidate with `glob_match(cat_pat, c.catalog) &&
///    glob_match(plug_pat, c.plugin)` matches. Zero matches ⇒
///    [`SelectorError::NoGlobMatch`] (never a silent no-op).
/// 2. **Literal `catalog/plugin`** (has `/`, no `*`): produces the `PluginId`
///    directly WITHOUT requiring it to be in `candidates` — existence is
///    checked downstream by `resolve_plugin_dir`, preserving the pre-#314 exit
///    codes. A slash-qualified token IGNORES `--catalog` (an explicit catalog
///    wins over the scope flag).
/// 3. **Literal bare `plugin`** (no `/`, no `*`): if `catalog` is `Some(c)`,
///    resolve to `c/plugin` (existence downstream); otherwise scan `candidates`
///    for catalogs holding a plugin named EXACTLY `plugin` — exactly one ⇒ that
///    id; multiple ⇒ [`SelectorError::Ambiguous`]; none ⇒
///    [`SelectorError::NotFound`].
///
/// `matched` is deduped preserving first-seen order, so a plugin matched by two
/// tokens (e.g. an explicit id and an overlapping glob) appears once.
pub fn resolve(tokens: &[String], candidates: &[PluginId], catalog: Option<&str>) -> Resolution {
    let mut matched: Vec<PluginId> = Vec::new();
    let mut errors: Vec<SelectorError> = Vec::new();

    // Order-preserving dedupe: only push an id not already in `matched`.
    let push_unique = |matched: &mut Vec<PluginId>, id: PluginId| {
        if !matched.contains(&id) {
            matched.push(id);
        }
    };

    for token in tokens {
        if token.contains('*') {
            // ---- GLOB -----------------------------------------------------
            let (cat_pat, plug_pat) = match token.split_once('/') {
                Some((c, p)) => (c.to_owned(), p.to_owned()),
                // No `/`: the plugin pattern is the whole token; the catalog
                // pattern is `--catalog` (exact) when set, else `*` (all).
                None => (
                    catalog.map(str::to_owned).unwrap_or_else(|| "*".to_owned()),
                    token.clone(),
                ),
            };

            let mut any = false;
            for cand in candidates {
                if glob_match(&cat_pat, &cand.catalog) && glob_match(&plug_pat, &cand.plugin) {
                    any = true;
                    push_unique(&mut matched, cand.clone());
                }
            }
            if !any {
                errors.push(SelectorError::NoGlobMatch {
                    pattern: token.clone(),
                });
            }
        } else if let Some((cat, plug)) = token.split_once('/') {
            // ---- LITERAL catalog/plugin -----------------------------------
            // Explicit catalog wins: `--catalog` is ignored for a slash token.
            // Existence is NOT checked here (downstream `resolve_plugin_dir`
            // owns that), so this always yields a `PluginId`.
            push_unique(
                &mut matched,
                PluginId {
                    catalog: cat.to_owned(),
                    plugin: plug.to_owned(),
                },
            );
        } else if let Some(cat) = catalog {
            // ---- LITERAL bare plugin, scoped by --catalog -----------------
            // Existence checked downstream (same as the slash form).
            push_unique(
                &mut matched,
                PluginId {
                    catalog: cat.to_owned(),
                    plugin: token.clone(),
                },
            );
        } else {
            // ---- LITERAL bare plugin, unscoped ----------------------------
            // Resolve the catalog by scanning candidates for an EXACT name.
            let hits: Vec<&PluginId> = candidates.iter().filter(|c| c.plugin == *token).collect();
            match hits.as_slice() {
                [] => errors.push(SelectorError::NotFound {
                    plugin: token.clone(),
                }),
                [only] => push_unique(&mut matched, (*only).clone()),
                many => errors.push(SelectorError::Ambiguous {
                    plugin: token.clone(),
                    candidates: many.iter().map(|c| c.to_string()).collect(),
                }),
            }
        }
    }

    Resolution { matched, errors }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(catalog: &str, plugin: &str) -> PluginId {
        PluginId {
            catalog: catalog.to_owned(),
            plugin: plugin.to_owned(),
        }
    }

    fn tok(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| (*s).to_owned()).collect()
    }

    // ---- glob_match --------------------------------------------------------

    #[test]
    fn glob_no_star_is_exact_equality() {
        assert!(glob_match("plugin", "plugin"));
        assert!(!glob_match("plugin", "plugins"));
        // pattern longer than text (common prefix) → no match
        assert!(!glob_match("plugin", "plug"));
        assert!(!glob_match("plugin", "widget"));
        assert!(glob_match("", ""));
        assert!(!glob_match("", "x"));
    }

    #[test]
    fn glob_star_alone_matches_everything() {
        assert!(glob_match("*", ""));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*", "a/b/c weird"));
    }

    #[test]
    fn glob_leading_star() {
        assert!(glob_match("*-expert", "midnight-expert"));
        assert!(glob_match("*-expert", "-expert"));
        assert!(!glob_match("*-expert", "expert"));
        assert!(!glob_match("*-expert", "midnight-expertise"));
    }

    #[test]
    fn glob_trailing_star() {
        assert!(glob_match("compact-*", "compact-lint"));
        assert!(glob_match("compact-*", "compact-"));
        assert!(!glob_match("compact-*", "compact"));
        assert!(!glob_match("compact-*", "acompact-x"));
    }

    #[test]
    fn glob_middle_star() {
        assert!(glob_match("a*c", "ac"));
        assert!(glob_match("a*c", "abc"));
        assert!(glob_match("a*c", "abbbbc"));
        assert!(!glob_match("a*c", "ab"));
        assert!(!glob_match("a*c", "xabc"));
    }

    #[test]
    fn glob_multiple_stars() {
        assert!(glob_match("a*b*c", "abc"));
        assert!(glob_match("a*b*c", "axxbyyc"));
        assert!(glob_match("**", "anything"));
        assert!(glob_match("*a*", "banana"));
        assert!(!glob_match("a*b*c", "acb"));
    }

    #[test]
    fn glob_matches_multibyte_utf8() {
        // A `*` transparently swallows multi-byte characters (a trailing `é`,
        // and whole CJK sequences).
        assert!(glob_match("valid-*", "valid-é"));
        assert!(glob_match("*é", "café"));
        assert!(glob_match("*", "日本語"));
        assert!(glob_match("日*語", "日本語"));
        // Literal comparison of an exact multibyte string, byte-for-byte.
        assert!(glob_match("café", "café"));
        assert!(!glob_match("café", "cafe"));
    }

    // ---- resolve -----------------------------------------------------------

    fn sample() -> Vec<PluginId> {
        vec![
            id("midnight", "compact-lint"),
            id("midnight", "compact-fmt"),
            id("midnight", "audit"),
            id("other", "audit"),
            id("other", "helper"),
        ]
    }

    #[test]
    fn resolve_literal_slash_id_bypasses_candidates() {
        // A slash-qualified literal need NOT be in candidates (existence is a
        // downstream concern).
        let r = resolve(&tok(&["ghost/plug"]), &sample(), None);
        assert_eq!(r.matched, vec![id("ghost", "plug")]);
        assert!(r.errors.is_empty());
    }

    #[test]
    fn resolve_bare_unique() {
        let r = resolve(&tok(&["helper"]), &sample(), None);
        assert_eq!(r.matched, vec![id("other", "helper")]);
        assert!(r.errors.is_empty());
    }

    #[test]
    fn resolve_bare_ambiguous_lists_candidates() {
        let r = resolve(&tok(&["audit"]), &sample(), None);
        assert!(r.matched.is_empty());
        assert_eq!(r.errors.len(), 1);
        match &r.errors[0] {
            SelectorError::Ambiguous { plugin, candidates } => {
                assert_eq!(plugin, "audit");
                assert_eq!(
                    candidates,
                    &vec!["midnight/audit".to_owned(), "other/audit".to_owned()]
                );
            }
            other => panic!("expected Ambiguous, got {other:?}"),
        }
    }

    #[test]
    fn resolve_bare_none_is_not_found() {
        let r = resolve(&tok(&["nope"]), &sample(), None);
        assert!(r.matched.is_empty());
        assert_eq!(
            r.errors,
            vec![SelectorError::NotFound {
                plugin: "nope".to_owned()
            }]
        );
    }

    #[test]
    fn resolve_glob_expands_multiple() {
        let r = resolve(&tok(&["midnight/compact-*"]), &sample(), None);
        assert_eq!(
            r.matched,
            vec![
                id("midnight", "compact-lint"),
                id("midnight", "compact-fmt")
            ],
        );
        assert!(r.errors.is_empty());
    }

    #[test]
    fn resolve_glob_zero_match_is_error() {
        let r = resolve(&tok(&["midnight/xyz-*"]), &sample(), None);
        assert!(r.matched.is_empty());
        assert_eq!(
            r.errors,
            vec![SelectorError::NoGlobMatch {
                pattern: "midnight/xyz-*".to_owned()
            }],
        );
    }

    #[test]
    fn resolve_bare_glob_spans_all_catalogs() {
        // `audit` with a `*` but no `/` and no --catalog → cat_pat = `*`.
        let r = resolve(&tok(&["audi*"]), &sample(), None);
        assert_eq!(
            r.matched,
            vec![id("midnight", "audit"), id("other", "audit")]
        );
        assert!(r.errors.is_empty());
    }

    #[test]
    fn resolve_dedup_across_two_tokens() {
        // An explicit id and an overlapping glob that both hit `compact-lint`.
        let r = resolve(
            &tok(&["midnight/compact-lint", "midnight/compact-*"]),
            &sample(),
            None,
        );
        assert_eq!(
            r.matched,
            vec![
                id("midnight", "compact-lint"),
                id("midnight", "compact-fmt")
            ],
            "compact-lint must appear once despite two matching tokens",
        );
        assert!(r.errors.is_empty());
    }

    #[test]
    fn resolve_catalog_scopes_bare_name() {
        // `--catalog other` makes a bare `audit` resolve to `other/audit` with
        // no ambiguity even though `midnight/audit` also exists.
        let r = resolve(&tok(&["audit"]), &sample(), Some("other"));
        assert_eq!(r.matched, vec![id("other", "audit")]);
        assert!(r.errors.is_empty());
    }

    #[test]
    fn resolve_catalog_scopes_bare_glob() {
        // `--catalog midnight` + bare glob `compact-*` → cat_pat = `midnight`.
        let r = resolve(&tok(&["compact-*"]), &sample(), Some("midnight"));
        assert_eq!(
            r.matched,
            vec![
                id("midnight", "compact-lint"),
                id("midnight", "compact-fmt")
            ],
        );
        assert!(r.errors.is_empty());
    }

    #[test]
    fn resolve_catalog_scoped_bare_ignores_existence() {
        // With --catalog, a bare name resolves directly (existence downstream)
        // — it does NOT need to be in candidates.
        let r = resolve(&tok(&["ghost"]), &sample(), Some("midnight"));
        assert_eq!(r.matched, vec![id("midnight", "ghost")]);
        assert!(r.errors.is_empty());
    }

    #[test]
    fn resolve_slash_token_ignores_catalog_flag() {
        // An explicit `catalog/plugin` wins over --catalog.
        let r = resolve(&tok(&["midnight/audit"]), &sample(), Some("other"));
        assert_eq!(r.matched, vec![id("midnight", "audit")]);
        assert!(r.errors.is_empty());
    }

    #[test]
    fn resolve_slash_glob_ignores_catalog_flag() {
        // A `catalog/plugin` glob also ignores --catalog: cat_pat comes from
        // the token, not the flag.
        let r = resolve(&tok(&["other/*"]), &sample(), Some("midnight"));
        assert_eq!(r.matched, vec![id("other", "audit"), id("other", "helper")]);
        assert!(r.errors.is_empty());
    }

    #[test]
    fn resolve_mixed_batch_collects_matches_and_errors() {
        // Good literal + good glob + a bad glob + an ambiguous bare name.
        let r = resolve(
            &tok(&[
                "other/helper",
                "midnight/compact-*",
                "midnight/xyz-*",
                "audit",
            ]),
            &sample(),
            None,
        );
        assert_eq!(
            r.matched,
            vec![
                id("other", "helper"),
                id("midnight", "compact-lint"),
                id("midnight", "compact-fmt"),
            ],
        );
        assert_eq!(r.errors.len(), 2);
        assert!(matches!(r.errors[0], SelectorError::NoGlobMatch { .. }));
        assert!(matches!(r.errors[1], SelectorError::Ambiguous { .. }));
    }

    // ---- SelectorError → TomeError mapping ---------------------------------

    #[test]
    fn ambiguous_maps_to_usage_and_lists_candidates() {
        use crate::error::TomeError;
        let err = SelectorError::Ambiguous {
            plugin: "audit".to_owned(),
            candidates: vec!["midnight/audit".to_owned(), "other/audit".to_owned()],
        }
        .into_tome_error();
        assert_eq!(err.exit_code(), 2, "Ambiguous must map to Usage (exit 2)");
        match err {
            TomeError::Usage(msg) => {
                assert!(msg.contains("midnight/audit"), "msg: {msg}");
                assert!(msg.contains("other/audit"), "msg: {msg}");
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn no_glob_match_maps_to_usage_and_echoes_pattern() {
        use crate::error::TomeError;
        let err = SelectorError::NoGlobMatch {
            pattern: "midnight/xyz-*".to_owned(),
        }
        .into_tome_error();
        match err {
            TomeError::Usage(msg) => assert!(msg.contains("midnight/xyz-*"), "msg: {msg}"),
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn not_found_maps_to_plugin_not_found() {
        use crate::error::TomeError;
        let err = SelectorError::NotFound {
            plugin: "nope".to_owned(),
        }
        .into_tome_error();
        assert!(matches!(&err, TomeError::PluginNotFound(p) if p == "nope"));
        assert_eq!(err.exit_code(), 20);
    }
}
