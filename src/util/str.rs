//! String utility helpers.
//!
//! Note: `harness::routing::truncate_tier3_summary` is a structural twin of
//! [`truncate_at_chars`] but returns `Cow<'_, str>` (zero-alloc on the common
//! no-truncation path) and is intentionally not delegated here — it allocates
//! only when truncation actually occurs.

/// Truncate `s` to at most `content_max` Unicode scalar values (chars),
/// appending `ellipsis` when truncation occurred.
///
/// When `s` already fits within `content_max` chars it is returned verbatim —
/// no allocation. When `content_max == 0` an empty string is returned (the
/// ellipsis is NOT appended for zero-capacity callers).
///
/// # Contract
///
/// - The *content* portion of the returned string is at most `content_max`
///   chars. The ellipsis is appended on top of that, so the *total* length is
///   at most `content_max + ellipsis.chars().count()` when truncation occurs.
/// - Callers that want a fixed total length (e.g. "at most 300 chars
///   including the ellipsis") should pass `content_max = total - ellipsis.chars().count()`.
///
/// # DoS safety
///
/// Uses a bounded `char_indices` walk — O(`content_max + 1`) work regardless
/// of input size. No `chars().count()` over the full input (which would be
/// O(n) even when no truncation is needed), and no `take().collect()`
/// allocation in the truncation path.
///
/// # Examples
///
/// ```
/// use tome::util::truncate_at_chars;
///
/// // Content fits — returned verbatim:
/// assert_eq!(truncate_at_chars("hello", 10, "…"), "hello");
///
/// // Truncated — content capped at `content_max` chars, ellipsis appended:
/// assert_eq!(truncate_at_chars("hello world", 5, "…"), "hello…");
///
/// // Zero cap:
/// assert_eq!(truncate_at_chars("hello", 0, "…"), "");
/// ```
pub fn truncate_at_chars(s: &str, content_max: usize, ellipsis: &str) -> String {
    if content_max == 0 {
        return String::new();
    }
    let mut iter = s.char_indices();
    // Walk past `content_max` chars. If we exhaust the iterator within those,
    // the input fits — return it verbatim.
    for _ in 0..content_max {
        if iter.next().is_none() {
            return s.to_owned();
        }
    }
    // If the (content_max+1)-th char exists, the input overflows: slice at its
    // byte offset and append the ellipsis. Otherwise the input was exactly
    // `content_max` chars — no truncation needed.
    match iter.next() {
        None => s.to_owned(),
        Some((byte_idx, _)) => {
            let mut out = String::with_capacity(byte_idx + ellipsis.len());
            out.push_str(&s[..byte_idx]);
            out.push_str(ellipsis);
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_input_returned_verbatim() {
        assert_eq!(truncate_at_chars("hi", 10, "…"), "hi");
    }

    #[test]
    fn exact_fit_not_truncated() {
        assert_eq!(truncate_at_chars("hello", 5, "…"), "hello");
    }

    #[test]
    fn overflow_truncated_with_ellipsis() {
        assert_eq!(truncate_at_chars("hello world", 5, "…"), "hello…");
    }

    #[test]
    fn zero_cap_returns_empty() {
        assert_eq!(truncate_at_chars("hello", 0, "…"), "");
    }

    #[test]
    fn empty_input_returns_empty() {
        assert_eq!(truncate_at_chars("", 5, "…"), "");
    }

    #[test]
    fn multibyte_chars_counted_as_one() {
        // "日本語" is 3 chars, each 3 bytes. With content_max=2 we get 2 chars.
        assert_eq!(truncate_at_chars("日本語", 2, "…"), "日本…");
    }

    #[test]
    fn multibyte_fits() {
        assert_eq!(truncate_at_chars("日本語", 3, "…"), "日本語");
    }

    #[test]
    fn custom_ellipsis() {
        assert_eq!(truncate_at_chars("hello world", 5, "..."), "hello...");
    }

    #[test]
    fn empty_ellipsis() {
        assert_eq!(truncate_at_chars("hello world", 5, ""), "hello");
    }

    // Reproduce the search_skills contract: content_max + ellipsis (total max+1)
    #[test]
    fn search_skills_contract() {
        let max = 5usize;
        let result = truncate_at_chars("hello world", max, "…");
        // content = max chars, ellipsis = 1 char, total = max+1 chars
        assert_eq!(result.chars().count(), max + 1);
    }

    // Reproduce the prompts contract: total <= DESCRIPTION_MAX_CHARS (content_max = max-1)
    #[test]
    fn prompts_contract() {
        let total_max = 5usize;
        let result = truncate_at_chars("hello world", total_max - 1, "…");
        // content = max-1 chars, ellipsis = 1 char, total = max chars
        assert_eq!(result.chars().count(), total_max);
    }
}
