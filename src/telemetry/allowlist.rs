//! The compiled-in catalog-attribution allowlist + the source canonicalizer
//! (Phase 10 / US4, FR-051..055).
//!
//! Attribution is a PURELY local, emit-time decision: a plugin action is
//! attributed to a catalog ONLY if the catalog's enrolled source URL — resolved
//! from the registry Tome already holds — canonicalizes to one of the
//! [`ATTRIBUTED_TELEMETRY_CATALOGS`] entries. There is NO remote allowlist fetch
//! (FR-055: a remote config endpoint would be a silent attribution-widening
//! backdoor) and NO stored attribution column (FR-052 / NFR-003: nothing about
//! attribution touches the SQLite schema, so `SCHEMA_VERSION` stays 4). Because
//! the allowlist is a `const`, de-allowlisting takes effect the moment the user
//! upgrades to a binary that dropped the entry (FR-053) — attribution follows the
//! running binary, never what was true at enable time.

use crate::catalog::git::scrub_credentials;

/// The attributed-catalog allowlist (FR-051): `(short_id, canonical_source)`
/// pairs, auditable purely via Git history (the only way to change it is a PR
/// that ships in a release — FR-055, no remote config).
///
/// `short_id` becomes the attributed event prefix (`catalog.<short_id>.*`).
/// `canonical_source` is ALREADY in canonical form — no scheme, no credentials,
/// lowercase host, no trailing `.git`/`/` — so it equals what [`canonicalize`]
/// produces for the catalog's enrolled URL. The enrolled URL for the Midnight
/// catalog is `github.com/devrelaicom/midnight-expert-tome` (per the README's
/// `tome catalog add devrelaicom/midnight-expert-tome`); [`canonicalize`] absorbs
/// scheme/case/`.git`/SSH-form/credential differences so only host/path must
/// match (R-16).
pub const ATTRIBUTED_TELEMETRY_CATALOGS: &[(&str, &str)] =
    &[("midnight", "github.com/devrelaicom/midnight-expert-tome")];

/// Canonicalize a (possibly credential-bearing, scheme-prefixed, SSH-form) source
/// URL to the bare `host/path` form used for allowlist comparison (FR-054).
///
/// The order is LOAD-BEARING and matches the contract:
/// 1. **scrub** credentials via [`scrub_credentials`] (operates on BYTES, so it
///    runs before any UTF-8 assumption) — credentials never participate in the
///    match;
/// 2. **UTF-8 decode** the scrubbed bytes, FAIL-CLOSED on non-UTF-8 → `None` (a
///    non-decodable source simply yields no attribution; it is NOT a panic and
///    NOT an error — the silent telemetry path must never crash);
/// 3. lowercase the host; strip the scheme (`https://`/`http://`/`ssh://`/
///    `git://`); rewrite the SSH `git@host:path` form to `host/path`; strip a
///    trailing `.git`; strip a trailing `/`.
///
/// Both comparison sides pass through THIS function, so the const is self-checking
/// (canonicalizing an already-canonical value is idempotent).
///
/// WHY the schemeless SSH scp-form rewrite happens BEFORE the scrub (a deliberate
/// deviation from a *literal* "scrub is always step 1"): `scrub_credentials`'s
/// SSH rule rewrites `git@<host>:` → `git@<host-redacted>:` to stop a host
/// leaking into LOG output. That redaction is correct for logging but would
/// DEFEAT legitimate SSH-form attribution — `git@github.com:…` would scrub to
/// `git@<host>:…` and never match the allowlist, breaking the FR-054 / AC-4.4
/// equivalence requirement. The schemeless scp form carries NO `user:password@`
/// credential (`git` is the conventional SSH user, not a secret), so normalising
/// it to `host/path` first removes nothing sensitive and leaves no credential for
/// the scrub to act on. Scheme-form credentials (`https://user:token@…`) are
/// untouched by this pre-step and are still fully scrubbed below — so "credentials
/// never participate" holds exactly.
pub fn canonicalize(raw: &str) -> Option<String> {
    let pre = rewrite_ssh_scp_form(raw.trim());

    // 1. Scrub on bytes — any scheme-form `user:token@` credential is removed
    //    before we ever treat the input as text, so it can't leak into the
    //    canonical form. (The schemeless SSH `git@host` was already normalised
    //    above and carries no credential.)
    let scrubbed = scrub_credentials(pre.as_bytes());

    // 2. UTF-8 decode, fail-closed. `scrub_credentials` only ever removes bytes
    //    or inserts ASCII markers, so a valid-UTF-8 input stays valid — this `?`
    //    is the defensive FR-054 step-2 guard rather than a reachable path.
    let decoded = std::str::from_utf8(&scrubbed).ok()?;

    let mut s = decoded.trim();

    // 3a. Strip a known scheme prefix.
    for scheme in ["https://", "http://", "ssh://", "git://"] {
        if let Some(rest) = strip_prefix_ascii_ci(s, scheme) {
            s = rest;
            break;
        }
    }

    // 3b. Strip a leading `user@` userinfo segment (the schemeless SSH `git@` was
    //     already consumed by the scp rewrite, but a `ssh://git@host/…` scheme
    //     form still has it). Only treat the pre-`@` part as userinfo when it has
    //     no '/' (a '/' would mean the '@' is inside the path).
    if let Some(at) = s.find('@') {
        let (userinfo, rest) = s.split_at(at);
        if !userinfo.contains('/') {
            s = &rest[1..]; // skip the '@'
        }
    }

    // 3c. Strip a trailing '/' then a trailing '.git' (order: `…/repo.git/` drops
    //     the slash first, then `.git`).
    let mut t = s.trim_end_matches('/');
    t = t.strip_suffix(".git").unwrap_or(t);
    t = t.trim_end_matches('/');

    // 3d. Lowercase the host segment (everything up to the first '/'); leave the
    //     path case-sensitive (forge repo paths can be case-sensitive).
    let canonical = lowercase_host(t);

    if canonical.is_empty() {
        return None;
    }
    Some(canonical)
}

/// Rewrite the schemeless SSH scp-like form `git@host:path` → `host/path`. A no-op
/// for anything with a `://` scheme or no schemeless `user@host:` shape. See the
/// WHY in [`canonicalize`]: this runs before the scrub so the scrubber's
/// SSH-host redaction can't break legitimate SSH-form attribution.
fn rewrite_ssh_scp_form(s: &str) -> std::borrow::Cow<'_, str> {
    // A scheme form (`scheme://…`) is not scp-like; leave it for scheme handling.
    if s.contains("://") {
        return std::borrow::Cow::Borrowed(s);
    }
    let at = match s.find('@') {
        Some(a) => a,
        None => return std::borrow::Cow::Borrowed(s),
    };
    let userinfo = &s[..at];
    let rest = &s[at + 1..];
    // `userinfo` must be a bare user (no '/'), and `rest` must be `host:path`
    // (the FIRST ':' separates host from path). Anything else is not scp-like.
    if userinfo.contains('/') {
        return std::borrow::Cow::Borrowed(s);
    }
    let colon = match rest.find(':') {
        Some(c) => c,
        None => return std::borrow::Cow::Borrowed(s),
    };
    // Drop the `user@`, turn the host/path ':' into '/'.
    let mut out = String::with_capacity(rest.len());
    out.push_str(&rest[..colon]);
    out.push('/');
    out.push_str(&rest[colon + 1..]);
    std::borrow::Cow::Owned(out)
}

/// Resolve a raw catalog source URL to its allowlist short id, if any (FR-052).
///
/// Canonicalizes `raw_source` and linear-scans [`ATTRIBUTED_TELEMETRY_CATALOGS`]
/// for a matching `canonical_source`. The const side is ALSO canonicalized
/// defensively, so the match holds even if a future const entry is added in a
/// slightly non-canonical form. `Some(short_id)` ⇒ emit attributed; `None` ⇒
/// anonymous only.
pub fn match_source(raw_source: &str) -> Option<&'static str> {
    let canonical = canonicalize(raw_source)?;
    ATTRIBUTED_TELEMETRY_CATALOGS
        .iter()
        .find(|(_, source)| {
            // Defensive: canonicalize the const side too. The const is already
            // canonical, so this is idempotent, but it guarantees the two sides
            // are compared under identical rules (the const is self-checking).
            canonicalize(source).as_deref() == Some(canonical.as_str())
        })
        .map(|(short_id, _)| *short_id)
}

/// Case-insensitive ASCII prefix strip (schemes are ASCII). Returns the remainder
/// after `prefix` if `s` starts with it (ignoring ASCII case), else `None`.
fn strip_prefix_ascii_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

/// Lowercase only the host (up to the first '/'); leave the path untouched.
fn lowercase_host(s: &str) -> String {
    match s.find('/') {
        Some(slash) => {
            let mut out = s[..slash].to_ascii_lowercase();
            out.push_str(&s[slash..]);
            out
        }
        None => s.to_ascii_lowercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIDNIGHT: &str = "github.com/devrelaicom/midnight-expert-tome";

    #[test]
    fn const_is_already_canonical() {
        // The self-checking invariant: every const entry equals its own
        // canonicalization (FR-054).
        for (_, source) in ATTRIBUTED_TELEMETRY_CATALOGS {
            assert_eq!(
                canonicalize(source).as_deref(),
                Some(*source),
                "allowlist const entry {source:?} is not already canonical"
            );
        }
    }

    #[test]
    fn equivalence_class_all_canonicalize_equal_and_match(/* AC-4.4 */) {
        // Every one of these differs from the canonical form ONLY by scheme,
        // case, `.git`, SSH-vs-HTTPS form, trailing slash, or credentials — they
        // MUST all canonicalize equal and match `Some("midnight")` (FR-054).
        let equivalents = [
            "https://github.com/devrelaicom/midnight-expert-tome",
            "https://github.com/devrelaicom/midnight-expert-tome.git",
            "git@github.com:devrelaicom/midnight-expert-tome.git",
            "ssh://git@github.com/devrelaicom/midnight-expert-tome",
            // Case + trailing slash.
            "https://GitHub.com/devrelaicom/midnight-expert-tome/",
            // Embedded credentials (scrubbed before participating).
            "https://user:token@github.com/devrelaicom/midnight-expert-tome",
            // The bare canonical form itself (idempotence).
            MIDNIGHT,
        ];
        for src in equivalents {
            assert_eq!(
                canonicalize(src).as_deref(),
                Some(MIDNIGHT),
                "canonicalize({src:?}) must equal the canonical Midnight source"
            );
            assert_eq!(
                match_source(src),
                Some("midnight"),
                "match_source({src:?}) must attribute to midnight"
            );
        }
    }

    #[test]
    fn different_repo_is_not_attributed() {
        let other = "github.com/devrelaicom/other";
        assert_eq!(canonicalize(other).as_deref(), Some(other));
        assert_eq!(
            match_source(other),
            None,
            "a different repo ⇒ no attribution"
        );
        // Same host, different org/repo, with scheme + `.git` — still no match.
        assert_eq!(
            match_source("https://github.com/someone/midnight-expert-tome.git"),
            None
        );
    }

    #[test]
    fn ssh_scp_form_colon_becomes_slash() {
        // The `git@host:path` ':' after the host becomes '/'; verify the bare
        // (non-allowlisted) shape rewrites correctly too.
        assert_eq!(
            canonicalize("git@example.com:org/repo.git").as_deref(),
            Some("example.com/org/repo")
        );
    }

    #[test]
    fn host_lowercased_path_preserved() {
        // Host is lowercased; the path keeps its case (forge paths may be
        // case-sensitive).
        assert_eq!(
            canonicalize("https://GitHub.com/DevRelAICom/Mixed-Case").as_deref(),
            Some("github.com/DevRelAICom/Mixed-Case")
        );
    }

    #[test]
    fn empty_and_garbage_inputs_yield_none_or_no_match() {
        // An input that canonicalizes to empty ⇒ None (no attribution, no panic).
        assert_eq!(canonicalize("").as_deref(), None);
        assert_eq!(canonicalize("https://").as_deref(), None);
        assert_eq!(match_source(""), None);
    }

    #[test]
    fn non_utf8_after_scrub_fails_closed() {
        // The fail-closed UTF-8 path: `canonicalize` takes a `&str` (already valid
        // UTF-8 by Rust's type system), and `scrub_credentials` is a redaction
        // that only ever REMOVES bytes / inserts ASCII `REDACTED` markers — it
        // cannot introduce invalid UTF-8 into a previously-valid-UTF-8 input. So a
        // non-UTF-8 source string is UNREPRESENTABLE at this entry point: the
        // fail-closed `?` on `from_utf8` is a defensive belt-and-braces guard that
        // upholds the FR-054 "scrub → utf8 → canonicalize" order even if
        // `scrub_credentials`'s contract ever changed. We document this rather
        // than fabricate an unreachable input; the guard is asserted by code
        // review, not a synthetic test (there is no safe way to hand a `&str`
        // invalid bytes). The `from_utf8(&scrubbed).ok()?` line is the FR-054
        // step-2 fail-closed behaviour.
        //
        // We DO assert the benign path: scrubbing a credential-bearing input still
        // decodes and canonicalizes.
        assert_eq!(
            canonicalize(
                "https://x-access-token:ghp_secret@github.com/devrelaicom/midnight-expert-tome"
            )
            .as_deref(),
            Some(MIDNIGHT)
        );
    }
}
