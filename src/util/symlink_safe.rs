//! Symlink-safe write-path guard (FR-007) — the single source of truth.
//!
//! Every Tome-managed write sink (hooks `settings.local.json`, guardrails
//! regions + the Cursor sibling, native agent files, the rules file, the MCP
//! config, the atomic populated-directory landing, the catalog registry) lands
//! bytes onto an operator-owned path. Before any of them writes, it MUST refuse
//! to traverse a **symlinked path component** — not only a symlinked *final
//! node* (the protection the project already had via
//! `symlink_metadata().is_symlink()`), but also a symlinked **intermediate
//! directory** on the way to the target.
//!
//! This module is that guard, in ONE place. Before Phase 7 each sink carried
//! its own copy of the final-node-only `refuse_symlink` check; that is the
//! exact "fix one sink, miss its parallel" hazard the project has been bitten
//! by twice. The sinks now delegate here, so the intermediate-component
//! hardening lands on every sink at once and can never again drift between
//! parallels.
//!
//! ## Why intermediate components matter (defence-in-depth, not a normal path)
//!
//! The trust model (FR-010): the operator owns and creates the harness/project
//! directory trees; plugin/frontmatter content never supplies a path
//! *component* — only the final filename, already validated elsewhere as one
//! safe path segment plus a `target.parent() == Some(dir)` write-site check.
//! An attacker who can swap an intermediate-dir symlink mid-sync already holds
//! operator filesystem privileges. So this is **defence-in-depth**: it closes
//! the residual TOCTOU window where a directory Tome is about to create-and-
//! write-through is replaced by a symlink, redirecting the write outside the
//! intended tree (e.g. clobbering `~/.ssh/authorized_keys`). The honest
//! posture is refusal, never silent traversal.
//!
//! ## Approach (the F2 spike, research §R-1)
//!
//! * **Linux** (`#[cfg(target_os = "linux")]`): `rustix::fs::openat2` with
//!   `ResolveFlags::NO_SYMLINKS` resolves the whole tail in-kernel and refuses
//!   ANY symlinked component in one syscall. `openat2` is Linux-only (the
//!   `linux_raw`/`linux_kernel` backend), which is exactly why this arm is
//!   Linux-gated; macOS uses the portable walk.
//! * **Portable** (macOS + any non-Linux Unix + the fallback): a per-component
//!   `rustix::fs::openat` + `OFlags::NOFOLLOW` directory walk. Each interior
//!   component is the *final* node of its own `openat`, and `O_NOFOLLOW`
//!   refuses a final symlink on every Unix, so a symlinked intermediate is
//!   rejected. The final node is opened `O_NOFOLLOW` too, preserving the
//!   final-node guarantee through the same primitive.
//!
//! ## Two findings the spike forced into this primitive
//!
//! 1. **The refusal errno is a SET, not one value.** macOS returns `ENOTDIR`
//!    for `O_NOFOLLOW | O_DIRECTORY` on a symlinked *directory* component (the
//!    symlink "is not a directory") but `ELOOP` for `O_NOFOLLOW`-only and for a
//!    symlinked *final* node; Linux's `RESOLVE_NO_SYMLINKS` returns `ELOOP`. The
//!    guard tests **membership** in `{ELOOP, ENOTDIR}`, never equality.
//! 2. **Anchor at a trusted, canonical root.** The walk starts from an
//!    operator-owned anchor directory opened by its CANONICAL path *without*
//!    `O_NOFOLLOW` on the anchor itself, and validates only the components
//!    *below* it. Otherwise irrelevant system symlinks (macOS `/tmp →
//!    /private/tmp`) trip the guard. The anchor here is the deepest ancestor of
//!    the target that is a real directory (probed no-follow, so a symlinked
//!    intermediate is NOT absorbed into the anchor — it falls into the walked
//!    tail and is refused). Canonicalising the anchor resolves only the trusted
//!    operator-owned prefix above it. This *matches* the FR-007 trust model
//!    rather than weakening it.
//!
//! ## What "OK" means
//!
//! A component that does not yet exist is not a symlink and cannot be followed;
//! the walk stops there and permits the write (the sink's own `create_dir_all`
//! and atomic rename create it). Only an existing symlinked component is
//! refused. This preserves the prior "missing path → permit" semantics exactly,
//! so non-symlink writes behave identically and the `SyncOutcome` JSON pins
//! stay green.
//!
//! Sync-only — `tests/sync_boundary.rs` enforces the constitution's sync
//! discipline on this tree. Unix-only behaviour by construction (the `rustix`
//! relative-open primitives); on non-Unix the check is a permissive no-op (the
//! threat model is Unix symlinks, and `tempfile`/`rename` carry their own
//! semantics on Windows).

use std::io;
use std::path::{Component, Path, PathBuf};

/// Refuse to write through a symlinked path component.
///
/// Returns `Ok(())` when no existing component of `target` (below the trusted
/// canonical anchor) is a symlink — including when `target` itself does not yet
/// exist. Returns an `io::Error` with [`io::ErrorKind::InvalidInput`] when a
/// symlinked component is found; the message names `target`. This is the
/// **sole** symlink-safe pre-write check; every sink delegates here and maps
/// the refusal onto its own dedicated [`TomeError`](crate::error::TomeError)
/// variant (hooks → exit 44, guardrails → 46, agents → 45, others → `Io` 7).
///
/// Benign walk conditions (a component that is absent, or a transient access
/// error short of a symlink detection) resolve to `Ok(())`, preserving the
/// pre-Phase-7 "missing path → permit" behaviour so normal writes are
/// unaffected.
pub fn refuse_symlinked_component(target: &Path) -> Result<(), io::Error> {
    #[cfg(unix)]
    {
        unix::check(target)
    }
    #[cfg(not(unix))]
    {
        // Threat model is Unix symlinks; non-Unix targets carry their own
        // rename/no-follow semantics. Permissive no-op keeps the cross-platform
        // signature uniform.
        let _ = target;
        Ok(())
    }
}

/// The refusal `io::Error` every sink expects for a symlinked component.
/// `InvalidInput` preserves the exact `ErrorKind` the pre-Phase-7 per-sink
/// `refuse_symlink` copies returned, so callers that already mapped that error
/// (and the `tests/exit_codes.rs` expectations) are unchanged.
#[cfg(unix)]
fn refusal(target: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "refusing to write through symlinked path component: {}",
            target.display()
        ),
    )
}

#[cfg(unix)]
mod unix {
    use super::{Component, Path, PathBuf, io, refusal};

    use rustix::fs::{CWD, Mode, OFlags, openat};
    use rustix::io::Errno;

    /// Errnos meaning "an `O_NOFOLLOW` open refused a symlinked component."
    ///
    /// `ELOOP` is the POSIX answer for a symlinked final node and for Linux's
    /// `RESOLVE_NO_SYMLINKS`; macOS additionally returns `ENOTDIR` when
    /// `O_DIRECTORY` is combined with `O_NOFOLLOW` on a symlinked directory
    /// component (the symlink is not itself a directory). The guard matches the
    /// SET, never a single value (spike finding #1).
    fn is_symlink_refusal(err: Errno) -> bool {
        err == Errno::LOOP || err == Errno::NOTDIR
    }

    /// Split `target` into (trusted canonical anchor dir, tail components below
    /// it). The anchor is the deepest ancestor of `target` that is a *real
    /// directory* when probed **no-follow** (`symlink_metadata`), so a
    /// symlinked intermediate is never absorbed into the anchor — it lands in
    /// `tail` and gets refused by the walk. The anchor is then canonicalised,
    /// which resolves only the operator-owned prefix above it (neutralising
    /// system symlinks like macOS `/tmp → /private/tmp`; spike finding #2).
    ///
    /// Returns `None` when no real-directory ancestor exists at all (e.g. a
    /// relative path with no existing prefix, or a parent-less root). The
    /// caller treats that as "nothing existing to traverse" → permit.
    fn anchor_and_tail(target: &Path) -> Option<(PathBuf, Vec<std::ffi::OsString>)> {
        // Walk ancestors from `target` upward. `Path::ancestors()` yields the
        // path itself first, then each parent, ending at "" / "/".
        for ancestor in target.ancestors() {
            if ancestor.as_os_str().is_empty() {
                continue;
            }
            // No-follow probe: a symlinked directory must NOT qualify as the
            // anchor (that would canonicalise it away and miss the attack); it
            // belongs in the walked tail.
            match std::fs::symlink_metadata(ancestor) {
                Ok(meta) if meta.file_type().is_dir() => {
                    // Found the deepest real-directory ancestor. Canonicalise
                    // it for a trusted, symlink-free anchor. If canonicalisation
                    // fails (racy unlink, permissions), fall back to the lexical
                    // path — the subsequent walk still NOFOLLOW-checks the tail.
                    let canonical =
                        std::fs::canonicalize(ancestor).unwrap_or_else(|_| ancestor.to_path_buf());
                    let tail = tail_components(ancestor, target);
                    return Some((canonical, tail));
                }
                // Not a directory (symlink, regular file, or absent): keep
                // walking up to find the real-directory anchor. A symlinked or
                // file ancestor will be re-encountered as a tail component and
                // refused/permitted by the walk.
                _ => continue,
            }
        }
        None
    }

    /// The `Normal` components of `target` strictly below `anchor` (the
    /// suffix of `target` after the `anchor` prefix). Both are derived from the
    /// same `target`, so the prefix relationship holds lexically.
    fn tail_components(anchor: &Path, target: &Path) -> Vec<std::ffi::OsString> {
        target
            .strip_prefix(anchor)
            .unwrap_or(target)
            .components()
            .filter_map(|c| match c {
                Component::Normal(seg) => Some(seg.to_os_string()),
                // `.`/`..`/prefix/root never appear in our operator-composed
                // sink paths below the anchor; ignore them defensively rather
                // than panicking in production.
                _ => None,
            })
            .collect()
    }

    /// Linux: resolve the whole tail under the trusted anchor in one syscall
    /// with `RESOLVE_NO_SYMLINKS`. A symlinked component anywhere in the tail is
    /// refused in-kernel (`ELOOP`).
    #[cfg(target_os = "linux")]
    fn walk_refuses_symlink(
        anchor: &Path,
        tail: &[std::ffi::OsString],
        target: &Path,
    ) -> Result<(), io::Error> {
        use rustix::fs::{ResolveFlags, openat2};

        if tail.is_empty() {
            return Ok(());
        }
        let rel: PathBuf = tail.iter().collect();

        // Open the trusted anchor WITHOUT NOFOLLOW (finding #2): it is the
        // operator-owned root, opened by canonical path.
        let anchor_fd = match openat(
            CWD,
            anchor,
            OFlags::DIRECTORY | OFlags::RDONLY | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => fd,
            // Anchor vanished/unreadable under a race — nothing we can prove is
            // a symlink; permit (the write's own ops will surface real errors).
            Err(_) => return Ok(()),
        };

        match openat2(
            &anchor_fd,
            &rel,
            OFlags::RDONLY | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS,
        ) {
            Ok(_) => Ok(()),
            Err(e) if is_symlink_refusal(e) => Err(refusal(target)),
            // ENOENT (a not-yet-created tail component) and any other non-
            // symlink errno mean "no symlink proven here" → permit, matching
            // the prior missing-path semantics.
            Err(_) => Ok(()),
        }
    }

    /// Portable (macOS + non-Linux Unix): walk the tail one component at a time
    /// from the trusted anchor, each interior component opened
    /// `O_NOFOLLOW | O_DIRECTORY` and the final node `O_NOFOLLOW`. A symlinked
    /// component is refused (`ELOOP`/`ENOTDIR`); an absent component stops the
    /// walk and permits.
    #[cfg(not(target_os = "linux"))]
    fn walk_refuses_symlink(
        anchor: &Path,
        tail: &[std::ffi::OsString],
        target: &Path,
    ) -> Result<(), io::Error> {
        let Some((leaf, dirs)) = tail.split_last() else {
            return Ok(());
        };

        // Anchor: trusted operator-owned root, opened by canonical path WITHOUT
        // NOFOLLOW (finding #2). A racy failure → permit.
        let mut parent = match openat(
            CWD,
            anchor,
            OFlags::DIRECTORY | OFlags::RDONLY | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => fd,
            Err(_) => return Ok(()),
        };

        let dir_flags = OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::RDONLY | OFlags::CLOEXEC;
        for seg in dirs {
            // Each interior component is the final node of THIS openat, so
            // NOFOLLOW refuses it if it is a symlink → the intermediate-symlink
            // refusal.
            match openat(&parent, seg.as_os_str(), dir_flags, Mode::empty()) {
                Ok(fd) => parent = fd,
                Err(e) if is_symlink_refusal(e) => return Err(refusal(target)),
                // Absent intermediate (ENOENT) or other non-symlink errno:
                // nothing planted here → permit (the write will create it).
                Err(_) => return Ok(()),
            }
        }

        // Final node: read-only, still NOFOLLOW so a symlinked leaf is refused.
        match openat(
            &parent,
            leaf.as_os_str(),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(_) => Ok(()),
            Err(e) if is_symlink_refusal(e) => Err(refusal(target)),
            Err(_) => Ok(()),
        }
    }

    /// Entry point: locate the trusted anchor + tail, then run the
    /// platform-appropriate walk. No existing prefix → permit.
    pub(super) fn check(target: &Path) -> Result<(), io::Error> {
        match anchor_and_tail(target) {
            Some((anchor, tail)) => walk_refuses_symlink(&anchor, &tail, target),
            None => Ok(()),
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    //! Focused unit tests for the primitive itself. They reuse the fixture
    //! pattern from `tests/symlink_intermediate_guard.rs` (real `tempfile`
    //! tempdir + `std::os::unix::fs::symlink`, canonicalised base) but exercise
    //! the SSOT entry point `refuse_symlinked_component` rather than the spike's
    //! local walk. The Linux `openat2` arm and the portable walk arm are both
    //! routed through this one function, so these tests prove BOTH backends on
    //! whichever OS runs them.

    use super::refuse_symlinked_component;
    use std::fs;
    use std::os::unix::fs::symlink;

    /// A clean path through real directories to a real (or not-yet-existing)
    /// final node is permitted.
    #[test]
    fn clean_path_is_permitted() {
        let root = tempfile::tempdir().expect("tempdir");
        let base = root.path().canonicalize().expect("canonicalize");
        let real_dir = base.join("real_dir");
        fs::create_dir(&real_dir).expect("mkdir");
        fs::write(real_dir.join("leaf.txt"), b"x").expect("write leaf");

        refuse_symlinked_component(&real_dir.join("leaf.txt"))
            .expect("clean existing path must be permitted");
        // A not-yet-existing final node under a real dir is also permitted —
        // the write will create it.
        refuse_symlinked_component(&real_dir.join("new.txt"))
            .expect("absent final node under a real dir must be permitted");
    }

    /// A symlinked INTERMEDIATE directory component is refused
    /// (`InvalidInput`), on both the Linux `openat2` arm and the portable walk.
    #[test]
    fn symlinked_intermediate_component_is_refused() {
        let root = tempfile::tempdir().expect("tempdir");
        let base = root.path().canonicalize().expect("canonicalize");
        let real_dir = base.join("real_dir");
        fs::create_dir(&real_dir).expect("mkdir");
        fs::write(real_dir.join("leaf.txt"), b"x").expect("write leaf");
        symlink(&real_dir, base.join("link_dir")).expect("symlink intermediate");

        let err = refuse_symlinked_component(&base.join("link_dir").join("leaf.txt"))
            .expect_err("symlinked intermediate component must be refused");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        // The leaf is genuinely reachable via the symlink at the OS level, so
        // the refusal is about the symlinked COMPONENT, not a missing file.
        assert!(base.join("link_dir").join("leaf.txt").exists());
    }

    /// A symlinked FINAL node is refused — the prior final-node guarantee,
    /// expressed through the SSOT primitive.
    #[test]
    fn symlinked_final_node_is_refused() {
        let root = tempfile::tempdir().expect("tempdir");
        let base = root.path().canonicalize().expect("canonicalize");
        fs::write(base.join("target.txt"), b"x").expect("write target");
        symlink(base.join("target.txt"), base.join("link.txt")).expect("symlink final");

        let err = refuse_symlinked_component(&base.join("link.txt"))
            .expect_err("symlinked final node must be refused");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    /// A symlinked DEEPER intermediate (two real dirs then a symlink) is
    /// refused — proves the walk validates beyond the first tail component.
    #[test]
    fn deeper_symlinked_intermediate_is_refused() {
        let root = tempfile::tempdir().expect("tempdir");
        let base = root.path().canonicalize().expect("canonicalize");
        let a = base.join("a");
        let real = a.join("real");
        fs::create_dir_all(&real).expect("mkdir a/real");
        fs::write(real.join("leaf.txt"), b"x").expect("write leaf");
        // a/link -> a/real ; so a/link/leaf.txt traverses a symlinked component.
        symlink(&real, a.join("link")).expect("symlink a/link -> a/real");

        let err = refuse_symlinked_component(&a.join("link").join("leaf.txt"))
            .expect_err("deeper symlinked intermediate must be refused");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    /// An entirely-absent tail (parent dir does not exist yet) is permitted —
    /// nothing planted to follow; the sink's `create_dir_all` will build it.
    #[test]
    fn absent_parent_chain_is_permitted() {
        let root = tempfile::tempdir().expect("tempdir");
        let base = root.path().canonicalize().expect("canonicalize");
        refuse_symlinked_component(&base.join("nope").join("deeper").join("file.txt"))
            .expect("absent parent chain must be permitted");
    }
}
