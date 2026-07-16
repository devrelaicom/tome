//! Shared value-validation for the optional metadata fields added to both
//! `tome-catalog.toml` and `tome-plugin.toml`. Cheap, structural, offline —
//! the same discipline as `catalog::manifest::looks_like_email`. Every
//! function returns `Err(message)` naming the field; the manifest parsers
//! wrap that into their own error type (Constitution §V).

use std::collections::BTreeMap;

pub const KEYWORDS_MAX: usize = 20;
pub const KEYWORD_MAX_LEN: usize = 40;
pub const DISPLAY_NAME_MAX_LEN: usize = 120;
pub const HANDLES_MAX: usize = 20;
pub const HANDLE_MAX_LEN: usize = 100;

/// Open map of social handles (`platform -> handle`). Keys are arbitrary
/// platform names; only the values are validated. `BTreeMap` keeps emit order
/// deterministic (byte-stable round-trip).
pub type Handles = BTreeMap<String, String>;

/// Structural `http`/`https` URL check: a recognised scheme plus a non-empty
/// host before the first `/`. No network I/O; does not verify reachability.
pub fn validate_http_url(field: &str, value: &str) -> Result<(), String> {
    let rest = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
        .ok_or_else(|| format!("`{field}` must be an http(s) URL: {value}"))?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() || host.chars().any(char::is_control) {
        return Err(format!("`{field}` has no host: {value}"));
    }
    Ok(())
}

/// Validate a keyword list: within count cap, each non-empty (trimmed),
/// control-free, within length cap. Duplicates are NOT rejected here — they
/// are a `lint` warning (Task B2), not a manifest error.
pub fn validate_keywords(keywords: &[String]) -> Result<(), String> {
    if keywords.len() > KEYWORDS_MAX {
        return Err(format!(
            "`keywords` has {} entries (max {KEYWORDS_MAX})",
            keywords.len()
        ));
    }
    for k in keywords {
        if k.trim().is_empty() {
            return Err("`keywords` contains an empty entry".to_owned());
        }
        if k.chars().any(char::is_control) {
            return Err(format!("`keywords` entry has a control character: {k:?}"));
        }
        if k.chars().count() > KEYWORD_MAX_LEN {
            return Err(format!(
                "`keywords` entry exceeds {KEYWORD_MAX_LEN} chars: {k:?}"
            ));
        }
    }
    Ok(())
}

/// Validate a display name: non-empty (trimmed), control-free, within cap.
/// Free text — NOT a path segment (unlike `name`).
pub fn validate_display_name(value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err("`display_name` must not be empty".to_owned());
    }
    if value.chars().any(char::is_control) {
        return Err("`display_name` must not contain control characters".to_owned());
    }
    if value.chars().count() > DISPLAY_NAME_MAX_LEN {
        return Err(format!(
            "`display_name` exceeds {DISPLAY_NAME_MAX_LEN} chars"
        ));
    }
    Ok(())
}

/// Validate a handles map: within count cap; each key + value non-empty,
/// control-free, within length cap. Keys are open (any platform); values are
/// display strings (per-platform format is NOT validated — that is the
/// machine-signal territory ruled out of scope).
pub fn validate_handles(handles: &Handles) -> Result<(), String> {
    if handles.len() > HANDLES_MAX {
        return Err(format!(
            "`handles` has {} entries (max {HANDLES_MAX})",
            handles.len()
        ));
    }
    for (k, v) in handles {
        if k.trim().is_empty() || v.trim().is_empty() {
            return Err("`handles` has an empty key or value".to_owned());
        }
        if k.chars().any(char::is_control) || v.chars().any(char::is_control) {
            return Err("`handles` has a control character".to_owned());
        }
        if k.chars().count() > HANDLE_MAX_LEN || v.chars().count() > HANDLE_MAX_LEN {
            return Err(format!("`handles` entry exceeds {HANDLE_MAX_LEN} chars"));
        }
    }
    Ok(())
}

/// Validate a relative-path field (`icon`): rejects URLs, absolute paths,
/// `..` traversal, and Windows drive prefixes. Shape-only — does NOT touch
/// the filesystem (file existence is a `lint` warning, Task B2). Mirrors the
/// first half of `catalog::manifest::validate_source` without its
/// canonicalize/existence/escape checks.
pub fn validate_relative_path(field: &str, value: &str) -> Result<(), String> {
    use std::path::{Component, Path};
    if value.contains("://") || value.starts_with("git@") {
        return Err(format!(
            "`{field}` must be a relative path, not a URL: {value}"
        ));
    }
    let bytes = value.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return Err(format!("`{field}` must not be an absolute path: {value}"));
    }
    let p = Path::new(value);
    if p.is_absolute() {
        return Err(format!("`{field}` must not be an absolute path: {value}"));
    }
    if p.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(format!("`{field}` must not contain `..`: {value}"));
    }
    if value.trim().is_empty() {
        return Err(format!("`{field}` must not be empty"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_url_accepts_http_and_https() {
        assert!(validate_http_url("homepage", "https://example.com").is_ok());
        assert!(validate_http_url("homepage", "http://example.com/path?x=1").is_ok());
    }

    #[test]
    fn http_url_rejects_other_schemes_and_garbage() {
        for bad in [
            "",
            "example.com",
            "ftp://example.com",
            "git@github.com:x/y",
            "https://",
            "https:///path",
            "mailto:a@b.co",
        ] {
            assert!(
                validate_http_url("homepage", bad).is_err(),
                "should reject {bad:?}"
            );
        }
    }

    #[test]
    fn keywords_accept_reasonable_lists() {
        assert!(validate_keywords(&["auth".into(), "oauth2".into()]).is_ok());
        assert!(validate_keywords(&[]).is_ok());
    }

    #[test]
    fn keywords_reject_empty_control_overlong_and_overcount() {
        assert!(validate_keywords(&["".into()]).is_err());
        assert!(validate_keywords(&["  ".into()]).is_err());
        assert!(validate_keywords(&["a\nb".into()]).is_err());
        assert!(validate_keywords(&["x".repeat(KEYWORD_MAX_LEN + 1)]).is_err());
        let too_many: Vec<String> = (0..=KEYWORDS_MAX).map(|i| format!("k{i}")).collect();
        assert!(validate_keywords(&too_many).is_err());
    }

    #[test]
    fn display_name_rejects_empty_control_overlong() {
        assert!(validate_display_name("Midnight Compact Expert").is_ok());
        assert!(validate_display_name("   ").is_err());
        assert!(validate_display_name("a\tb").is_err());
        assert!(validate_display_name(&"x".repeat(DISPLAY_NAME_MAX_LEN + 1)).is_err());
    }

    #[test]
    fn handles_reject_empty_control_and_overcount() {
        let mut ok = Handles::new();
        ok.insert("github".into(), "aaronbassett".into());
        assert!(validate_handles(&ok).is_ok());

        let mut empty_val = Handles::new();
        empty_val.insert("github".into(), "".into());
        assert!(validate_handles(&empty_val).is_err());

        let mut ctrl = Handles::new();
        ctrl.insert("x".into(), "a\nb".into());
        assert!(validate_handles(&ctrl).is_err());
    }

    #[test]
    fn relative_path_rejects_url_absolute_and_traversal() {
        assert!(validate_relative_path("icon", "assets/icon.png").is_ok());
        for bad in [
            "https://x/i.png",
            "/abs/i.png",
            "../escape.png",
            "C:\\i.png",
            "",
        ] {
            assert!(
                validate_relative_path("icon", bad).is_err(),
                "should reject {bad:?}"
            );
        }
    }
}
