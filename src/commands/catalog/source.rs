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
//! branch and is treated as a local path (`file://<cwd>/gl:foo`). That path
//! won't exist, so the later `git clone` fails clearly; we never silently
//! synthesise a forge URL from a malformed shorthand.

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
    // NOT expand it — fall through to the local-path branch, where it is
    // treated as a local path (`file://<cwd>/gl:foo`) and the later
    // `git clone` fails clearly rather than us producing a bad forge URL.
    // The host in each pair is HARDCODED and must stay the sole authority:
    // everything after the prefix is a path (`{owner}/{repo}`), never a host.
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

    /// Security-critical: the forge host is HARDCODED — everything after the
    /// prefix is a PATH (`{owner}/{repo}`), never a host. These assertions pin
    /// that authority so a future refactor of `looks_like_owner_repo` (or the
    /// expansion loop) can't silently let attacker-controlled input move the
    /// resolved host off the intended forge.
    #[test]
    fn forge_prefix_host_is_immutable() {
        // A host-shaped first segment is still just the OWNER path segment —
        // the resolved host stays `gitlab.com`, and `evil.com` is appended as
        // path, never substituted as the authority.
        assert_eq!(
            resolve("gl:evil.com/x").unwrap(),
            "https://gitlab.com/evil.com/x",
        );

        // Three-segment input is not `owner/repo` (`parts.len() != 2`), so it
        // must NOT expand — it falls through to the local-path branch. In
        // particular it must never yield a two-slash forge URL where the
        // trailing segment could read as a path escape.
        let three = resolve("gh:a/b/c").unwrap();
        assert!(
            three.starts_with("file://"),
            "`gh:a/b/c` (3 segments) must fall through to a local path; got {}",
            three,
        );
        assert!(!three.contains("github.com"), "got {}", three);

        // An `@` in the remainder isn't a valid `owner/repo` character, so a
        // userinfo-injection-ish shorthand does NOT expand — it falls through
        // and never synthesises a `user@host`-style forge URL.
        let at = resolve("gl:a@b/c").unwrap();
        assert!(
            at.starts_with("file://"),
            "`gl:a@b/c` must fall through to a local path; got {}",
            at,
        );
        assert!(!at.contains("gitlab.com"), "got {}", at);
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
