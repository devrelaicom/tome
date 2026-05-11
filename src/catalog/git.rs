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
pub fn scrub_credentials(input: &[u8]) -> Vec<u8> {
    static URL_LOGIN: OnceLock<Regex> = OnceLock::new();
    static SSH_LOGIN: OnceLock<Regex> = OnceLock::new();
    static KV_SECRET: OnceLock<Regex> = OnceLock::new();
    static LONG_HEX: OnceLock<Regex> = OnceLock::new();

    let url_login = URL_LOGIN
        .get_or_init(|| Regex::new(r"(?P<scheme>https?://)[^/@\s]+@").expect("valid regex"));
    let ssh_login =
        SSH_LOGIN.get_or_init(|| Regex::new(r"(?P<at>\bgit@)[^\s:]+:").expect("valid regex"));
    // The optional `(?:...\s+)?` permits "Authorization: Bearer <token>" to
    // match as one unit — otherwise the keyword "Bearer" would split the
    // match and leave the actual token leaking after the replacement.
    let kv_secret = KV_SECRET.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?P<key>token|password|api[-_]?key|bearer|authorization)(?P<sep>\s*[:=]\s*)(?:(?:token|password|api[-_]?key|bearer|authorization)\s+)?\S+",
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
    step4.into_owned()
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
}

/// `^[0-9a-f]{7,40}$` — caller-side test for SHA-shaped refs. Used by
/// `tome catalog update` to no-op on SHA-pinned catalogs (FR-008).
pub fn looks_like_sha(s: &str) -> bool {
    static SHA: OnceLock<regex::Regex> = OnceLock::new();
    SHA.get_or_init(|| regex::Regex::new(r"^[0-9a-fA-F]{7,40}$").expect("valid"))
        .is_match(s)
}
