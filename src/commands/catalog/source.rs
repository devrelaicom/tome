//! Source-resolution helper: convert a user-supplied source string into a
//! canonical Git URL.
//!
//! Recognised shapes (per `contracts/catalog-add.md`):
//!
//! - `owner/repo` → `https://github.com/owner/repo`
//! - `gh:owner/repo` → `https://github.com/owner/repo`
//! - `gl:owner/repo` → `https://gitlab.com/owner/repo`
//! - `bb:owner/repo` → `https://bitbucket.org/owner/repo`
//! - `https://…`, `http://…`, `git@…`, `file://…` → kept verbatim
//! - any other value → treated as a local path and converted to `file://`
//!   after canonicalisation
//!
//! A forge-prefixed value whose remainder does not look like `owner/repo`
//! (e.g. `gl:foo`) is NOT expanded — it falls through to the local-path
//! branch so a malformed shorthand never silently produces a bad URL.

use std::path::{Path, PathBuf};

use crate::error::TomeError;

pub fn resolve(input: &str) -> Result<String, TomeError> {
    if input.starts_with("https://")
        || input.starts_with("http://")
        || input.starts_with("file://")
        || input.starts_with("git@")
        || input.starts_with("ssh://")
        || input.starts_with("git://")
    {
        return Ok(input.to_string());
    }

    // Forge-prefixed shorthands: `gh:`/`gl:`/`bb:owner/repo`. The remainder
    // must still look like `owner/repo`; if it doesn't (e.g. `gl:foo`) we do
    // NOT expand it — fall through to the local-path branch so a malformed
    // shorthand fails clearly rather than producing a bad URL.
    for (prefix, host) in [
        ("gh:", "github.com"),
        ("gl:", "gitlab.com"),
        ("bb:", "bitbucket.org"),
    ] {
        if let Some(rest) = input.strip_prefix(prefix)
            && looks_like_owner_repo(rest)
        {
            return Ok(format!("https://{}/{}", host, rest));
        }
    }

    // `owner/repo` shorthand: a single `/`, no leading slash, no whitespace,
    // and segments that look like Git identifiers.
    if looks_like_owner_repo(input) {
        return Ok(format!("https://github.com/{}", input));
    }

    // Otherwise: treat as a local path.
    let p = Path::new(input);
    let abs = if p.is_absolute() {
        PathBuf::from(input)
    } else {
        std::env::current_dir().map_err(TomeError::Io)?.join(p)
    };
    // Canonicalise so `..` and symlinks are normalised. Falls back to the raw
    // absolute path if the target doesn't exist — `git clone` will then fail
    // with a clearer error than ours could provide.
    let canonical = abs.canonicalize().unwrap_or(abs);
    Ok(format!("file://{}", canonical.display()))
}

fn looks_like_owner_repo(s: &str) -> bool {
    if s.contains(char::is_whitespace) || s.starts_with('/') || s.starts_with('.') {
        return false;
    }
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() != 2 {
        return false;
    }
    let valid = |p: &str| -> bool {
        !p.is_empty()
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    valid(parts[0]) && valid(parts[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_repo_expands_to_github() {
        assert_eq!(
            resolve("midnight/midnight-experts").unwrap(),
            "https://github.com/midnight/midnight-experts"
        );
    }

    #[test]
    fn gh_prefix_expands_to_github() {
        assert_eq!(
            resolve("gh:owner/repo").unwrap(),
            "https://github.com/owner/repo"
        );
    }

    #[test]
    fn gl_prefix_expands_to_gitlab() {
        assert_eq!(
            resolve("gl:owner/repo").unwrap(),
            "https://gitlab.com/owner/repo"
        );
    }

    #[test]
    fn bb_prefix_expands_to_bitbucket() {
        assert_eq!(
            resolve("bb:owner/repo").unwrap(),
            "https://bitbucket.org/owner/repo"
        );
    }

    #[test]
    fn malformed_forge_prefix_is_not_expanded() {
        // `gl:foo` has no `owner/repo` after the prefix, so it must NOT expand
        // to a forge URL — it falls through to the local-path branch instead.
        let r = resolve("gl:foo").unwrap();
        assert!(
            r.starts_with("file://"),
            "malformed `gl:foo` must not become a gitlab URL; got {}",
            r
        );
        assert!(!r.contains("gitlab.com"), "got {}", r);
    }

    #[test]
    fn https_url_kept_verbatim() {
        assert_eq!(
            resolve("https://example/owner/repo").unwrap(),
            "https://example/owner/repo"
        );
    }

    #[test]
    fn ssh_url_kept_verbatim() {
        assert_eq!(
            resolve("git@github.com:owner/repo").unwrap(),
            "git@github.com:owner/repo"
        );
    }

    #[test]
    fn file_url_kept_verbatim() {
        assert_eq!(resolve("file:///x/y").unwrap(), "file:///x/y");
    }

    #[test]
    fn path_with_dot_is_local() {
        // Anything starting with `.` is a path, never `owner/repo`.
        let r = resolve("./relative/path").unwrap();
        assert!(r.starts_with("file://"), "got {}", r);
    }
}
