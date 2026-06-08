//! Untrusted source-tree read guard (FR-009a, FR-011a, NFR-004) — the single
//! boundary every `convert` importer routes its reads and copies through.
//!
//! A `convert` source is a **foreign artifact tree we do not control**. Left
//! unguarded, an importer that joins source-supplied names onto a base path and
//! reads them is a classic path-traversal / symlink-escape sink: a `..`
//! component, an absolute path, or a symlinked directory could redirect a read
//! (info disclosure into the converted output) or a copy (pulling
//! `/etc/passwd` into the emitted artifact) outside the source root.
//!
//! [`UntrustedRoot`] closes that off in one place so no importer has to
//! re-derive the policy:
//!
//! 1. **Containment by construction** — the root is canonicalised once
//!    ([`UntrustedRoot::open`]); every read path is composed of `Normal`
//!    components only (no `..`, no absolute, no root/prefix) joined onto that
//!    canonical root, so the result is lexically within the root.
//! 2. **Symlink refusal — a per-component walk from the trusted root.** Every
//!    resolved path is walked one component at a time *from the canonical
//!    root*, refusing the first symlinked component at any depth (final node
//!    *or* any intermediate). This deliberately does **not** reuse the
//!    write-side SSOT `util::refuse_symlinked_component`: that guard finds its
//!    anchor with `symlink_metadata`, which *follows* an intermediate symlink,
//!    leaving a grandparent-escape gap (`escape -> /outside`, then
//!    `escape/foo/SKILL.md` resolves under `/outside/foo` and is permitted).
//!    That gap is acceptable for the write guard's operator-owned trust model
//!    but **not** for an untrusted read. Walking from the known-canonical root —
//!    where every prefix is verified non-symlink before we descend, so
//!    `symlink_metadata` never follows a symlink to reach the next component —
//!    closes it. With `Normal`-only components and no symlinked component, the
//!    resolved path is provably within the root. Directory listing additionally
//!    refuses a symlinked child outright (no-follow `file_type`), fail-closed.
//!    (TOCTOU posture: a `symlink_metadata` check then a read has a swap window;
//!    for a one-shot convert of a static source by a single-user CLI this is
//!    benign — an attacker mutating the tree mid-convert already holds local FS
//!    access. The refusal, not a lock, is the boundary.)
//! 3. **Bounded, UTF-8-fail-closed reads** — bodies are read through
//!    [`bounded_read_to_string`] capped by class ([`ENTRY_BODY_MAX`] for entry
//!    bodies, [`PLUGIN_MANIFEST_MAX`] for manifests/frontmatter), so a hostile
//!    multi-GiB or non-UTF-8 file is refused, never slurped.
//! 4. **Safe-segment validation on *emitted* names** — [`validate_name`] reuses
//!    the project's `plugin::identity::validate_segment` so a source-supplied
//!    name that becomes a *Tome* file/dir (a skill dir, a command/agent stem,
//!    the vendored plugin's own name) is a single safe segment before it is
//!    composed into a write path. Supporting-file leaf names are not
//!    `validate_name`-checked (a dot-prefixed support file is legal to copy);
//!    their *containment* is guaranteed instead — by `list_dir`'s no-follow
//!    single-segment names and the emit sink's `ensure_in_bounds` `Normal`-only
//!    assertion.
//!
//! [`validate_name`]: UntrustedRoot::validate_name
//!
//! All refusals surface as [`TomeError::Io`] (exit 7, `InvalidInput`/
//! `InvalidData`) — the same fail-closed convention the write-side symlink
//! guard already uses — so the whole layer reads uniformly and a malformed or
//! malicious source aborts the convert with a named error and nothing on disk.
//!
//! Sync-only (the constitution's async island is `src/mcp/` only).

use std::path::{Component, Path, PathBuf};

use crate::error::TomeError;
use crate::plugin::identity::validate_segment;
use crate::util::{ENTRY_BODY_MAX, bounded_read_to_string};

/// A canonicalised source root that bounds every importer read.
///
/// Construct with [`UntrustedRoot::open`]; thereafter all access is relative to
/// the root and validated. The root path itself is operator-supplied (the
/// `SOURCE` argument or a temp clone of it), so canonicalising it resolves only
/// the trusted prefix above the tree — exactly the anchor model the write-side
/// symlink guard uses.
#[derive(Debug, Clone)]
pub struct UntrustedRoot {
    /// Canonical absolute path of the source root.
    root: PathBuf,
}

/// One child of a listed directory, with its path relative to the root and a
/// no-follow `is_dir` flag (symlinked children are refused before reaching
/// here, so the flag reflects a real file/dir).
#[derive(Debug, Clone)]
pub struct DirChild {
    /// Bare file name (a single safe-to-read segment; may be dot-prefixed,
    /// which is legal to *read* — `.claude-plugin`, `.mcp.json`).
    pub name: String,
    /// Path relative to the root (`parent.join(name)`), for further guarded
    /// access.
    pub rel: PathBuf,
    /// Whether the child is a real directory (no-follow).
    pub is_dir: bool,
}

/// Build the uniform fail-closed refusal error (exit 7).
fn refuse(msg: String) -> TomeError {
    TomeError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, msg))
}

impl UntrustedRoot {
    /// Open `root` as the containment boundary for an untrusted source tree.
    /// Canonicalises the root (resolving the trusted operator-owned prefix) and
    /// requires it to be a directory.
    ///
    /// # Errors
    /// [`TomeError::Io`] if `root` is missing/unreadable or is not a directory.
    pub fn open(root: &Path) -> Result<Self, TomeError> {
        let canonical = std::fs::canonicalize(root).map_err(TomeError::Io)?;
        let meta = std::fs::symlink_metadata(&canonical).map_err(TomeError::Io)?;
        if !meta.file_type().is_dir() {
            return Err(refuse(format!(
                "source root is not a directory: {}",
                canonical.display()
            )));
        }
        Ok(Self { root: canonical })
    }

    /// The canonical root path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Validate a source-supplied name that will become a *Tome* file/dir name
    /// (a skill directory, a supporting-file segment). Reuses the project's
    /// safe-segment validator (no empty/`/`/`\`/NUL/`.`/`..`/leading-dot).
    ///
    /// This is intentionally *stricter* than [`resolve`](Self::resolve), which
    /// permits dot-prefixed names because reading `.claude-plugin/plugin.json`
    /// is legitimate; an *emitted* leading-dot name is not.
    ///
    /// # Errors
    /// [`TomeError::Io`] naming the rejected segment.
    pub fn validate_name(name: &str) -> Result<(), TomeError> {
        validate_segment(name)
            .map_err(|kind| refuse(format!("unsafe name from source: `{name}` ({kind})")))
    }

    /// Resolve a root-relative path to a validated in-root absolute path.
    ///
    /// Refuses any non-`Normal` component (`..`, absolute, root/prefix) and any
    /// symlinked component (final or intermediate) via a walk from the canonical
    /// root. The returned path is guaranteed to live within the root.
    ///
    /// # Errors
    /// [`TomeError::Io`] on an escape attempt or a symlinked component.
    pub fn resolve(&self, relative: &Path) -> Result<PathBuf, TomeError> {
        // Walk component-by-component from the trusted canonical root. Each
        // prefix is verified non-symlink before we descend, so `symlink_metadata`
        // never follows a symlink to reach the next component — closing the
        // grandparent-escape gap an anchor-finding guard would leave.
        let mut cur = self.root.clone();
        for comp in relative.components() {
            let seg = match comp {
                Component::Normal(seg) => seg,
                Component::CurDir => continue, // `.` is a harmless no-op segment
                _ => {
                    return Err(refuse(format!(
                        "refusing source path that escapes the root: {}",
                        relative.display()
                    )));
                }
            };
            cur.push(seg);
            match std::fs::symlink_metadata(&cur) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(refuse(format!(
                        "refusing symlinked component under source root: {}",
                        cur.display()
                    )));
                }
                // A real (non-symlink) component: descend and keep checking.
                Ok(_) => {}
                // An absent component breaks the chain — nothing deeper exists to
                // follow, so the path is contained. A later read of an absent
                // file surfaces its own `NotFound`.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                Err(e) => return Err(TomeError::Io(e)),
            }
        }
        Ok(self.root.join(relative))
    }

    /// Whether an in-root relative path exists (false on any refusal).
    pub fn exists(&self, relative: &Path) -> bool {
        self.resolve(relative).map(|p| p.exists()).unwrap_or(false)
    }

    /// Whether an in-root relative path is a real directory (no-follow; false
    /// on any refusal).
    pub fn is_dir(&self, relative: &Path) -> bool {
        match self.resolve(relative) {
            Ok(p) => std::fs::symlink_metadata(&p)
                .map(|m| m.file_type().is_dir())
                .unwrap_or(false),
            Err(_) => false,
        }
    }

    /// Whether an in-root relative path is a real file (no-follow; false on any
    /// refusal).
    pub fn is_file(&self, relative: &Path) -> bool {
        match self.resolve(relative) {
            Ok(p) => std::fs::symlink_metadata(&p)
                .map(|m| m.file_type().is_file())
                .unwrap_or(false),
            Err(_) => false,
        }
    }

    /// Read an entry's Markdown body: bounded by [`ENTRY_BODY_MAX`] and
    /// **UTF-8-fail-closed** (a non-UTF-8 body is a named error, never lossy).
    ///
    /// # Errors
    /// [`TomeError::Io`] on a refusal, an over-cap file, or non-UTF-8 content.
    pub fn read_body(&self, relative: &Path) -> Result<String, TomeError> {
        let abs = self.resolve(relative)?;
        bounded_read_to_string(&abs, ENTRY_BODY_MAX)
    }

    /// Read a bounded UTF-8 text file at the caller's per-class cap (e.g.
    /// [`PLUGIN_MANIFEST_MAX`](crate::util::PLUGIN_MANIFEST_MAX) for a manifest
    /// or a frontmatter-bearing config).
    ///
    /// # Errors
    /// [`TomeError::Io`] on a refusal, an over-cap file, or non-UTF-8 content.
    pub fn read_text(&self, relative: &Path, cap: u64) -> Result<String, TomeError> {
        let abs = self.resolve(relative)?;
        bounded_read_to_string(&abs, cap)
    }

    /// List the immediate children of an in-root directory, sorted by name for
    /// deterministic conversion (FR-027). A **symlinked child is refused**
    /// (fail-closed) — the honest posture for an untrusted tree.
    ///
    /// # Errors
    /// [`TomeError::Io`] on a refusal (escape, symlinked component, symlinked
    /// child, non-UTF-8 name) or an underlying read error.
    pub fn list_dir(&self, relative: &Path) -> Result<Vec<DirChild>, TomeError> {
        let abs = self.resolve(relative)?;
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&abs).map_err(TomeError::Io)? {
            let entry = entry.map_err(TomeError::Io)?;
            let file_type = entry.file_type().map_err(TomeError::Io)?;
            let name_os = entry.file_name();
            let name = name_os.to_str().ok_or_else(|| {
                refuse(format!(
                    "refusing non-UTF-8 filename under {}",
                    abs.display()
                ))
            })?;
            // A symlinked child cannot be read/copied safely; refuse outright
            // rather than silently skip (fail-closed; `file_type` is no-follow).
            if file_type.is_symlink() {
                return Err(refuse(format!(
                    "refusing symlinked entry in source tree: {}",
                    abs.join(name).display()
                )));
            }
            out.push(DirChild {
                rel: relative.join(name),
                name: name.to_owned(),
                is_dir: file_type.is_dir(),
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::util::PLUGIN_MANIFEST_MAX;
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::Path;

    /// A canonicalised tempdir root + a couple of real files/dirs.
    fn fixture() -> (tempfile::TempDir, UntrustedRoot) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let base = tmp.path().canonicalize().expect("canonicalize");
        fs::create_dir(base.join("skills")).unwrap();
        fs::create_dir(base.join("skills/foo")).unwrap();
        fs::write(base.join("skills/foo/SKILL.md"), b"# body\n").unwrap();
        fs::write(base.join(".mcp.json"), b"{}").unwrap();
        let root = UntrustedRoot::open(&base).expect("open root");
        (tmp, root)
    }

    #[test]
    fn opens_a_dir_and_canonicalizes() {
        let (tmp, root) = fixture();
        assert_eq!(root.root(), tmp.path().canonicalize().unwrap());
    }

    #[test]
    fn open_refuses_a_non_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("file.txt");
        fs::write(&f, b"x").unwrap();
        let err = UntrustedRoot::open(&f).unwrap_err();
        assert_eq!(err.exit_code(), 7);
    }

    #[test]
    fn reads_a_clean_in_root_body() {
        let (_tmp, root) = fixture();
        let body = root.read_body(Path::new("skills/foo/SKILL.md")).unwrap();
        assert_eq!(body, "# body\n");
    }

    #[test]
    fn reads_a_dot_prefixed_file() {
        // Reading (not emitting) a dot-prefixed name is legitimate.
        let (_tmp, root) = fixture();
        assert_eq!(
            root.read_text(Path::new(".mcp.json"), PLUGIN_MANIFEST_MAX)
                .unwrap(),
            "{}"
        );
    }

    #[test]
    fn refuses_parent_dir_escape() {
        let (_tmp, root) = fixture();
        let err = root.resolve(Path::new("../outside.txt")).unwrap_err();
        assert_eq!(err.exit_code(), 7);
    }

    #[test]
    fn refuses_absolute_path() {
        let (_tmp, root) = fixture();
        let err = root.resolve(Path::new("/etc/passwd")).unwrap_err();
        assert_eq!(err.exit_code(), 7);
    }

    #[test]
    fn refuses_symlinked_final_node() {
        let (tmp, root) = fixture();
        let base = tmp.path().canonicalize().unwrap();
        // secret lives outside the root entirely.
        let secret = tmp.path().parent().unwrap().join("secret.txt");
        let _ = fs::write(&secret, b"top secret");
        symlink(&secret, base.join("leak.txt")).unwrap();
        let err = root.read_body(Path::new("leak.txt")).unwrap_err();
        assert_eq!(err.exit_code(), 7);
    }

    #[test]
    fn refuses_symlinked_intermediate_component() {
        let (tmp, root) = fixture();
        let base = tmp.path().canonicalize().unwrap();
        // `escape` -> a real in-root dir; reading through the symlinked `escape`
        // component is refused even though the underlying file is reachable at
        // the OS level (in-root symlinks are refused too — strict + consistent).
        symlink(base.join("skills"), base.join("escape")).unwrap();
        let err = root
            .read_body(Path::new("escape/foo/SKILL.md"))
            .unwrap_err();
        assert_eq!(err.exit_code(), 7);
    }

    #[test]
    fn refuses_grandparent_symlink_escaping_to_a_real_external_dir() {
        // The dangerous case the write-side anchor-finding guard MISSES: a
        // symlinked *grandparent* pointing OUTSIDE the root to a real directory
        // with a real subtree. An anchor-finding guard would `symlink_metadata`-
        // probe `root/escape/sub` (following `escape`), anchor at the external
        // `/outside/sub`, and permit the read. The walk-from-root guard refuses
        // at the `escape` component.
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("root");
        fs::create_dir(&base).unwrap();
        let base = base.canonicalize().unwrap();
        // An external real dir tree holding a secret, OUTSIDE the source root.
        let outside = tmp.path().join("outside");
        fs::create_dir_all(outside.join("sub")).unwrap();
        fs::write(outside.join("sub/secret.md"), b"exfiltrated").unwrap();
        // root/escape -> /outside (a real external dir).
        symlink(&outside, base.join("escape")).unwrap();

        let root = UntrustedRoot::open(&base).unwrap();
        let err = root
            .read_body(Path::new("escape/sub/secret.md"))
            .unwrap_err();
        assert_eq!(err.exit_code(), 7, "grandparent escape must be refused");
    }

    #[test]
    fn list_dir_returns_sorted_children_including_dotfiles() {
        let (_tmp, root) = fixture();
        let children = root.list_dir(Path::new("")).unwrap();
        let names: Vec<&str> = children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec![".mcp.json", "skills"]);
        let skills = children.iter().find(|c| c.name == "skills").unwrap();
        assert!(skills.is_dir);
    }

    #[test]
    fn list_dir_refuses_a_symlinked_child() {
        let (tmp, root) = fixture();
        let base = tmp.path().canonicalize().unwrap();
        symlink(base.join("skills"), base.join("linkdir")).unwrap();
        let err = root.list_dir(Path::new("")).unwrap_err();
        assert_eq!(err.exit_code(), 7);
    }

    #[test]
    fn read_body_refuses_over_cap() {
        let (tmp, root) = fixture();
        let base = tmp.path().canonicalize().unwrap();
        let big = vec![b'A'; (ENTRY_BODY_MAX + 1) as usize];
        fs::write(base.join("big.md"), &big).unwrap();
        let err = root.read_body(Path::new("big.md")).unwrap_err();
        assert_eq!(err.exit_code(), 7);
    }

    #[test]
    fn read_body_refuses_non_utf8() {
        let (tmp, root) = fixture();
        let base = tmp.path().canonicalize().unwrap();
        fs::write(base.join("bin.md"), [0xff, 0xfe, 0x00]).unwrap();
        let err = root.read_body(Path::new("bin.md")).unwrap_err();
        assert_eq!(err.exit_code(), 7);
    }

    #[test]
    fn validate_name_accepts_safe_and_rejects_unsafe() {
        UntrustedRoot::validate_name("my-skill").unwrap();
        for bad in ["..", ".hidden", "a/b", "", "."] {
            assert!(
                UntrustedRoot::validate_name(bad).is_err(),
                "expected `{bad}` to be rejected"
            );
        }
    }
}
