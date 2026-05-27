//! Argument substitution per Claude Code semantics.
//!
//! Phase 5 / US3.a — Stage 3 of the substitution pipeline. Handles
//! `$ARGUMENTS`, `$ARGUMENTS[N]`, `$N`, and `$<name>` references in
//! command/skill bodies per `contracts/substitution-engine.md` § Stage 3.
//!
//! ## Single-sweep design (US2.d B2 + US3 extension)
//!
//! Stage 3 is folded into the unified regex sweep in
//! [`super::regex_sets::combined_regex`] (Option A from the US3 brief:
//! structural fix preventing NFR-007 violations between stages 1+2 and 3).
//! Resolved values emit directly into the output buffer and never
//! re-enter the scanner — a hostile plugin setting
//! `"plugin_name": "$0"` cannot cause Stage 3 to substitute its own
//! Stage-1 output.
//!
//! This module exposes two helpers consumed by [`super::render`]:
//!
//! - [`coerce_arguments`] — apply the caller-coercion table per
//!   `contracts/substitution-engine.md` § Stage 3 (single-string +
//!   shell-split vs object-with-named vs catch-all vs mismatch).
//! - [`apply_arguments_match`] — resolve one regex capture against the
//!   coerced [`ResolvedArguments`] to a string + a "did it substitute"
//!   sentinel.

use std::collections::HashMap;

use super::{ArgumentValues, SubstitutionError};

/// Positional + named view of caller-supplied arguments after the
/// coercion rules in `contracts/substitution-engine.md` § Stage 3.
///
/// - `positional` is indexed by `$ARGUMENTS[N]` / `$N` references and
///   joined by spaces for bare `$ARGUMENTS`.
/// - `named` is indexed by `$<name>` references.
///
/// Both maps may be empty: `positional` is empty when the caller
/// supplied no positional values; `named` is empty when the entry
/// declared no named arguments (or when the caller supplied a single
/// string for an entry with no named declarations).
pub(super) struct ResolvedArguments {
    pub(super) positional: Vec<String>,
    pub(super) named: HashMap<String, String>,
}

/// Shell-style quoting parser per research §R-10.
///
/// - Whitespace separates tokens.
/// - Single OR double quotes preserve internal whitespace.
/// - No escape sequences (`\` is literal).
/// - No nested quoting (a quote inside a same-type quote ends the
///   token; the contract names this explicitly).
///
/// Trailing unterminated quotes yield the token's contents up to EOI
/// (we don't error — Claude Code itself tolerates this; the caller has
/// no rich error channel here).
///
/// Matches the documented behaviour of Claude Code's `$ARGUMENTS`
/// shell-splitting so prompts authored against Claude Code work on
/// Tome unmodified. Intentionally re-implemented inline (~30 LoC)
/// rather than depending on `shell-words` per research §R-10.
pub(super) fn shell_split(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quote: Option<char> = None;
    let mut have_token = false;

    for c in s.chars() {
        match in_quote {
            Some(q) => {
                if c == q {
                    // End of quoted run; remain inside the same token —
                    // adjacent quoted/unquoted fragments concatenate
                    // (matching shell-words semantics, e.g.
                    // `a"b c"d` → `ab cd`).
                    in_quote = None;
                } else {
                    current.push(c);
                }
            }
            None => {
                if c == '\'' || c == '"' {
                    in_quote = Some(c);
                    have_token = true;
                } else if c.is_whitespace() {
                    if have_token {
                        out.push(std::mem::take(&mut current));
                        have_token = false;
                    }
                } else {
                    current.push(c);
                    have_token = true;
                }
            }
        }
    }

    if have_token {
        out.push(current);
    }
    out
}

/// Apply the caller-coercion table per `contracts/substitution-engine.md`
/// § Stage 3 to produce a [`ResolvedArguments`] ready for per-match
/// dispatch.
///
/// Returns [`SubstitutionError::PromptArgumentMismatch`] (exit 26) for
/// the two named-mismatch rows in the table:
/// - `Object{named}` with a key not in `declared` (any declared shape).
/// - `Object` with a non-`args` key and `declared` is empty.
///
/// The expected/supplied fields encode the surface for the operator's
/// error envelope: `expected` is the count of declared names (0 when
/// catch-all expected), `supplied` is the count of named keys the
/// caller submitted.
///
/// MCP's `prompts/get` handler already validates this shape upstream
/// (`mcp::prompts::map_caller_arguments`) — the re-validation here
/// covers library API and future surface consumers (`tome render`,
/// scripted callers, …) that don't pass through the MCP coercion.
pub(super) fn coerce_arguments(
    args: &ArgumentValues,
    declared: &[String],
) -> Result<ResolvedArguments, SubstitutionError> {
    match args {
        ArgumentValues::Single(s) => {
            if declared.is_empty() {
                // Whole-string single positional. `$ARGUMENTS` resolves
                // to the entire string; `$ARGUMENTS[0]` likewise.
                Ok(ResolvedArguments {
                    positional: vec![s.clone()],
                    named: HashMap::new(),
                })
            } else {
                // Shell-split; bind positionally to declared names in
                // declaration order. Extra tokens beyond the declared
                // count are kept as additional positional entries
                // (accessible via `$ARGUMENTS[N]`).
                let tokens = shell_split(s);
                let mut named = HashMap::with_capacity(declared.len());
                for (i, name) in declared.iter().enumerate() {
                    let value = tokens.get(i).cloned().unwrap_or_default();
                    named.insert(name.clone(), value);
                }
                Ok(ResolvedArguments {
                    positional: tokens,
                    named,
                })
            }
        }
        ArgumentValues::Object {
            named,
            declared_order,
        } => {
            if declared.is_empty() {
                // Catch-all case for entries with no declared named
                // args. Object MUST be a single `args` key — anything
                // else surfaces `PromptArgumentMismatch`.
                if named.len() == 1
                    && let Some(s) = named.get("args")
                {
                    // Coerce to Single + recurse — declared is empty.
                    return coerce_arguments(&ArgumentValues::Single(s.clone()), declared);
                }
                return Err(SubstitutionError::PromptArgumentMismatch {
                    expected: 0,
                    supplied: named.len(),
                });
            }

            // Declared has names. Validate every key in `named` is in
            // `declared` (i.e. no unknown caller keys). Missing
            // declared keys are bound to empty strings per the
            // contract's "partial named" row.
            for key in named.keys() {
                if !declared.iter().any(|d| d == key) {
                    return Err(SubstitutionError::PromptArgumentMismatch {
                        expected: declared.len(),
                        supplied: named.len(),
                    });
                }
            }

            // Prefer the caller's `declared_order` (carries the entry's
            // declaration order at the time the args were constructed);
            // fall back to the resolver's `declared` (which is what's
            // currently active for this render). They should agree;
            // when they do, `declared_order` wins.
            let order = if declared_order.is_empty() {
                declared
            } else {
                declared_order.as_slice()
            };
            let mut positional = Vec::with_capacity(order.len());
            let mut named_out = HashMap::with_capacity(order.len());
            for name in order {
                let value = named.get(name).cloned().unwrap_or_default();
                positional.push(value.clone());
                named_out.insert(name.clone(), value);
            }
            Ok(ResolvedArguments {
                positional,
                named: named_out,
            })
        }
    }
}

/// Resolve one Stage 3 capture to its substituted string.
///
/// Returns `(value, substituted)` where `substituted = true` if any
/// Stage 3 alternative produced a value (even the empty string per
/// FR-040 "empty if out of range / not provided"). When `substituted`
/// is `false`, the caller should leave the original match text
/// verbatim in the output.
///
/// Dispatch shape:
/// - Group 4 set (`$ARGUMENTS[N]`) → positional lookup, empty if out of
///   range.
/// - Group 5 set (`$N`) → same as `$ARGUMENTS[N]`.
/// - Group 6 set (`$<name>`) → named lookup, empty if not provided.
/// - Bare `$ARGUMENTS` (no Stage-3 capture group set) → positional
///   values joined by single space (FR-042).
///
/// The "bare $ARGUMENTS" branch is recognised at the caller in
/// [`super::render`] by inspecting `m.as_str()` rather than a capture
/// group — the unified pattern's bare-`$ARGUMENTS` alternative has no
/// capture; see [`super::regex_sets::combined_regex`] for the pattern.
pub(super) fn apply_arguments_match(
    caps: &regex::Captures<'_>,
    args: &ResolvedArguments,
) -> (String, bool) {
    use super::regex_sets::{ARG_INDEX_GROUP, NAMED_GROUP, POSITIONAL_GROUP};

    // Polish m-3 (Phase 5): `unwrap_or(usize::MAX)` sentinels the
    // overflow path — the `\d+` regex match parses cleanly for any
    // input that fits a usize, but a 20+-digit value (e.g.
    // `$99999999999999999999`) would otherwise panic. MAX falls
    // through `positional.get(MAX)` → None → empty, matching the
    // contract's "out-of-range index renders empty" rule.
    if let Some(idx) = caps.get(ARG_INDEX_GROUP) {
        let n: usize = idx.as_str().parse().unwrap_or(usize::MAX);
        let value = args.positional.get(n).cloned().unwrap_or_default();
        return (value, true);
    }
    if let Some(idx) = caps.get(POSITIONAL_GROUP) {
        let n: usize = idx.as_str().parse().unwrap_or(usize::MAX);
        let value = args.positional.get(n).cloned().unwrap_or_default();
        return (value, true);
    }
    if let Some(name) = caps.get(NAMED_GROUP) {
        let value = args.named.get(name.as_str()).cloned().unwrap_or_default();
        return (value, true);
    }
    // Bare `$ARGUMENTS` — recognised by the caller via `m.as_str()`
    // rather than a capture group. Returning `("", false)` would be
    // wrong here; the caller routes the bare match directly to
    // `bare_arguments_value` instead. This branch is defensive only.
    (String::new(), false)
}

/// Resolve bare `$ARGUMENTS` to its substituted string per FR-042
/// (positional values joined by single space).
///
/// Always returns `true` for the substitution sentinel — even when
/// `positional` is empty the resolved value (empty string) counts as a
/// Stage 3 replacement and suppresses the Stage 4 append-fallback.
pub(super) fn bare_arguments_value(args: &ResolvedArguments) -> String {
    args.positional.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- shell_split (R-10) ----------------------------------------------

    #[test]
    fn shell_split_empty_input_yields_empty_vec() {
        assert_eq!(shell_split(""), Vec::<String>::new());
    }

    #[test]
    fn shell_split_simple_whitespace_separation() {
        assert_eq!(shell_split("a b c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn shell_split_collapses_runs_of_whitespace() {
        assert_eq!(shell_split("a   b\t\tc"), vec!["a", "b", "c"]);
    }

    #[test]
    fn shell_split_leading_trailing_whitespace_ignored() {
        assert_eq!(shell_split("   a b  "), vec!["a", "b"]);
    }

    #[test]
    fn shell_split_single_quotes_preserve_whitespace() {
        assert_eq!(shell_split("a 'b c' d"), vec!["a", "b c", "d"]);
    }

    #[test]
    fn shell_split_double_quotes_preserve_whitespace() {
        assert_eq!(shell_split(r#"a "b c" d"#), vec!["a", "b c", "d"]);
    }

    #[test]
    fn shell_split_adjacent_quoted_and_unquoted_concatenate() {
        // Matches shell-words / sh semantics.
        assert_eq!(shell_split(r#"a"b c"d"#), vec!["ab cd"]);
    }

    #[test]
    fn shell_split_backslash_is_literal_no_escape() {
        // R-10: no escape sequences. `\` carries through verbatim.
        assert_eq!(shell_split(r"a\ b"), vec![r"a\", "b"]);
    }

    #[test]
    fn shell_split_unterminated_quote_emits_contents_to_eoi() {
        // Tolerant of malformed input — Claude Code's behaviour.
        assert_eq!(shell_split("a 'b c"), vec!["a", "b c"]);
    }
}
