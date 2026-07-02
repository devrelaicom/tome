//! Git shell-outs and credential scrubbing. Every byte stream captured from
//! a spawned `git` process passes through `scrub_credentials` before it
//! reaches `tracing`, `anyhow::Error`, or any display path (FR-024, FR-025).
//!
//! Signal handling: a global `AtomicBool` is flipped by a `ctrlc` handler.
//! In-flight child processes are killed and `TomeError::Interrupted` is
//! returned, exit code 8 (FR-026a).

use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use regex::bytes::Regex;

use crate::error::TomeError;

static CANCELLED: AtomicBool = AtomicBool::new(false);
static HANDLER_INSTALLED: OnceLock<()> = OnceLock::new();

/// Install the SIGINT handler once. Idempotent; safe to call from `main` or
/// from tests.
pub fn install_signal_handler() {
    HANDLER_INSTALLED.get_or_init(|| {
        let _ = ctrlc::set_handler(|| {
            CANCELLED.store(true, Ordering::SeqCst);
        });
    });
}

pub fn was_cancelled() -> bool {
    CANCELLED.load(Ordering::SeqCst)
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn reset_cancellation_for_tests() {
    CANCELLED.store(false, Ordering::SeqCst);
}

/// Scrub credential-bearing patterns from a captured stderr/stdout byte
/// stream. The rules are applied in the order documented in research.md R-8.
///
/// Phase 12 (FR-014a) extends this — the ONE shared scrubber — to also redact
/// remote-provider credentials wherever they surface, including bare reflection
/// in a JSON response body:
/// - `x-api-key` header values (folded into the `kv_secret` key alternation,
///   alongside the existing `authorization`/`bearer`/`api-key` keys).
/// - Provider key FORMATS as bare tokens (the `provider_key` step below):
///   OpenAI `sk-…` (covers `sk-ant-…`/`sk-proj-…`), Voyage `pa-…`, Google
///   `AIza…`. This runs AFTER the bearer/header KV step so a key in a
///   `Authorization: Bearer <k>` / `x-api-key: <k>` / `?key=<k>` context is
///   first collapsed by the KV/url rules, then the format step catches anything
///   that was reflected raw with no surrounding key context.
pub fn scrub_credentials(input: &[u8]) -> Vec<u8> {
    static URL_LOGIN: OnceLock<Regex> = OnceLock::new();
    static SSH_LOGIN: OnceLock<Regex> = OnceLock::new();
    static KV_SECRET: OnceLock<Regex> = OnceLock::new();
    static LONG_HEX: OnceLock<Regex> = OnceLock::new();
    static PROVIDER_KEY: OnceLock<Regex> = OnceLock::new();

    // Match any URI scheme (RFC 3986 §3.1) followed by `userinfo@`. Covers
    // `https://`, `http://`, `git://`, `ssh://`, and — relevant for
    // `tome catalog add file://user:token@/path` — `file://`. Tools like
    // `git` silently ignore userinfo for local transports, but the user
    // typed it and we promised not to persist it.
    let url_login = URL_LOGIN.get_or_init(|| {
        Regex::new(r"(?P<scheme>[a-z][a-z0-9+.-]*://)[^/@\s]+@").expect("valid regex")
    });
    let ssh_login =
        SSH_LOGIN.get_or_init(|| Regex::new(r"(?P<at>\bgit@)[^\s:]+:").expect("valid regex"));
    // The optional `(?:...\s+)?` permits "Authorization: Bearer <token>" to
    // match as one unit — otherwise the keyword "Bearer" would split the
    // match and leave the actual token leaking after the replacement.
    //
    // The signed-URL alternatives (`x-amz-signature`, `x-amz-credential`,
    // `x-amz-security-token`, plain `signature`) cover the presigned-URL
    // query-string form that model-host CDNs serve (AWS S3, R2). These
    // arrive via `?key=value&key2=value2` so the existing `=` separator
    // already matches; we just teach the key alternation to recognise them.
    let kv_secret = KV_SECRET.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?P<key>x-api-key|token|password|api[-_]?key|bearer|authorization|signature|x-amz-signature|x-amz-credential|x-amz-security-token)(?P<sep>\s*[:=]\s*)(?:(?:token|password|api[-_]?key|bearer|authorization)\s+)?[^\s&]+",
        )
        .expect("valid regex")
    });
    // Long hex (40+ chars). The alternation lets the regex catch *both* the
    // safe context (`<word>[:=]\s*<hex>` — a SHA1 reference) and the unsafe
    // context (a bare hex blob in prose). A closure-replacer below decides
    // what to do per match: preserve safe, scrub unsafe.
    let long_hex = LONG_HEX.get_or_init(|| {
        Regex::new(
            r"(?P<safe>\b\w+\s*[:=]\s*[0-9a-fA-F]{40,}\b)|(?P<unsafe_hex>\b[0-9a-fA-F]{40,}\b)",
        )
        .expect("valid regex")
    });
    // Phase 12 (FR-014a): bare provider key FORMATS, redacted wherever they
    // appear — including raw reflection in a JSON response body with no
    // surrounding `key=` context. The three alternatives cover:
    //   * OpenAI:  `sk-` + ≥16 url-safe chars (also matches `sk-ant-…`,
    //              `sk-proj-…` — the hyphen is inside the char class).
    //   * Voyage:  `pa-` + ≥16 url-safe chars.
    //   * Google:  `AIza` + ≥20 url-safe chars (the AIza… API-key shape; the
    //              value the `?key=AIza…` query also carries).
    // The whole match is replaced with `<scrubbed>`. Runs LAST so a key already
    // collapsed by the KV/url steps is untouched, and a raw reflected one is
    // still caught.
    let provider_key = PROVIDER_KEY.get_or_init(|| {
        Regex::new(r"(?:sk-[A-Za-z0-9_-]{16,}|pa-[A-Za-z0-9_-]{16,}|AIza[A-Za-z0-9_-]{20,})")
            .expect("valid regex")
    });

    let step1 = url_login.replace_all(input, &b"${scheme}"[..]);
    let step2 = ssh_login.replace_all(&step1, &b"${at}<host>:"[..]);
    let step3 = kv_secret.replace_all(&step2, &b"${key}${sep}<scrubbed>"[..]);
    let step4 = long_hex.replace_all(&step3, |caps: &regex::bytes::Captures| -> Vec<u8> {
        if caps.name("safe").is_some() {
            caps.get(0).expect("full match").as_bytes().to_vec()
        } else {
            b"<scrubbed>".to_vec()
        }
    });
    let step5 = provider_key.replace_all(&step4, &b"<scrubbed>"[..]);
    step5.into_owned()
}

/// Convert captured `git` stderr into a UTF-8-ish, scrubbed `String` suitable
/// for embedding in `TomeError::GitFailed.detail`. Lossy decoding is
/// acceptable here — `git` emits human-readable error text.
pub fn scrub_to_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&scrub_credentials(bytes)).into_owned()
}

/// Helper around `std::process::Command` for the small set of Git operations
/// Tome performs. Every command runs synchronously; cancellation is observed
/// after the child exits or, in long-running paths, between sub-steps.
pub struct Git {
    catalog: String,
}

impl Git {
    pub fn new(catalog: impl Into<String>) -> Self {
        Self {
            catalog: catalog.into(),
        }
    }

    fn run<I, S>(&self, args: I, cwd: Option<&Path>) -> Result<Vec<u8>, TomeError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        if was_cancelled() {
            return Err(TomeError::Interrupted);
        }
        let mut cmd = Command::new("git");
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        let mut child = cmd.spawn().map_err(TomeError::Io)?;
        let output = loop {
            if was_cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(TomeError::Interrupted);
            }
            match child.try_wait().map_err(TomeError::Io)? {
                Some(_status) => break child.wait_with_output().map_err(TomeError::Io)?,
                None => std::thread::sleep(std::time::Duration::from_millis(25)),
            }
        };
        if !output.status.success() {
            let detail = scrub_to_string(&output.stderr);
            return Err(TomeError::GitFailed {
                catalog: self.catalog.clone(),
                detail,
            });
        }
        Ok(output.stdout)
    }

    /// Shallow clone `url` into `dest`, optionally tracking `ref_`. The
    /// destination must not exist; caller is responsible for using a temp dir.
    ///
    /// The `--` end-of-options separator is inserted before `url` and `dest`
    /// so that a third-party URL (e.g. from a marketplace `plugins[].source`)
    /// can never be parsed as a git option — argument-injection defence.
    /// `--branch` accepts branch/tag names only; a commit-SHA pin will fail
    /// the clone and degrade to the fetch-failed warning.
    pub fn clone_shallow(
        &self,
        url: &str,
        dest: &Path,
        ref_: Option<&str>,
    ) -> Result<(), TomeError> {
        let mut args: Vec<String> = vec!["clone".into(), "--depth".into(), "1".into()];
        if let Some(r) = ref_ {
            args.push("--branch".into());
            args.push(r.to_string());
        }
        // End-of-options: a third-party URL can never be parsed as a git
        // option (argument-injection defence; the marketplace controls it).
        args.push("--".into());
        args.push(url.into());
        args.push(dest.display().to_string());
        self.run(args, None).map(|_| ())
    }

    /// `git -C <repo> fetch origin`.
    pub fn fetch(&self, repo: &Path) -> Result<(), TomeError> {
        self.run(["fetch", "origin"], Some(repo)).map(|_| ())
    }

    /// `git -C <repo> reset --hard <target>`. Used by `update` to advance to
    /// `origin/<branch>` or a specific tag/SHA.
    pub fn reset_hard(&self, repo: &Path, target: &str) -> Result<(), TomeError> {
        self.run(["reset", "--hard", target], Some(repo))
            .map(|_| ())
    }

    /// `git -C <repo> rev-parse HEAD` — returns the commit SHA as a hex
    /// string. Used by `update` to compute the "advanced N commits" counter.
    pub fn rev_parse_head(&self, repo: &Path) -> Result<String, TomeError> {
        let bytes = self.run(["rev-parse", "HEAD"], Some(repo))?;
        Ok(String::from_utf8_lossy(&bytes).trim().to_string())
    }

    /// Best-effort HEAD SHA read: `git -C <repo> rev-parse HEAD`, degrading to
    /// `None` on ANY failure (git error, empty output). Used by `catalog add`
    /// to echo the resolved commit — a purely informational display field, so
    /// a failure here must never fail the add (the catalog is already
    /// registered by the time this runs). Output is a plain hex string with no
    /// credential surface; the underlying `run` still routes any `git` stderr
    /// through `scrub_credentials`, but that error is dropped here.
    pub fn rev_parse_head_opt(&self, repo: &Path) -> Option<String> {
        let sha = self.rev_parse_head(repo).ok()?;
        if sha.is_empty() { None } else { Some(sha) }
    }

    /// `git -C <repo> log -1 --format=%cI -- <rel_path>` — the committer date
    /// (strict-ISO-8601 / RFC-3339-compatible) of the most recent commit that
    /// touched `rel_path`, relative to the repo root. Returns `None` when the
    /// path has no history in the (possibly shallow-cloned) working tree, when
    /// `git` produces no output, or when the output isn't RFC-3339-parseable.
    ///
    /// Best-effort by contract: this powers a purely informational display
    /// field (`plugin list` / `plugin show`). It is DISPLAY-time only — the
    /// value is never persisted, so there is no schema/migration cost. Callers
    /// degrade to the `indexed_at` value on any failure; a hard `git` error
    /// (`GitFailed`) still propagates so the caller can distinguish "no
    /// history" (`Ok(None)`) from "git blew up" and choose to swallow it.
    ///
    /// `--` terminates options so a plugin-supplied `source` path can never be
    /// parsed as a git flag (argument-injection defence, mirroring
    /// `clone_shallow`). `%cI` is used over `%aI` because the committer date is
    /// the timestamp that advances on a `reset --hard`/rebase — i.e. the one
    /// that reflects "changed upstream since we last pulled".
    pub fn last_commit_iso(
        &self,
        repo: &Path,
        rel_path: &str,
    ) -> Result<Option<String>, TomeError> {
        let bytes = self.run(
            [
                "log",
                "-1",
                "--format=%cI",
                "--",
                if rel_path.is_empty() { "." } else { rel_path },
            ],
            Some(repo),
        )?;
        let iso = String::from_utf8_lossy(&bytes).trim().to_string();
        Ok(if iso.is_empty() { None } else { Some(iso) })
    }
}

/// `^[0-9a-f]{7,40}$` — caller-side test for SHA-shaped refs. Used by
/// `tome catalog update` to no-op on SHA-pinned catalogs (FR-008).
pub fn looks_like_sha(s: &str) -> bool {
    static SHA: OnceLock<regex::Regex> = OnceLock::new();
    SHA.get_or_init(|| regex::Regex::new(r"^[0-9a-fA-F]{7,40}$").expect("valid"))
        .is_match(s)
}
