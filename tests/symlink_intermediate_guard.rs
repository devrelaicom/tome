//! SPIKE (Phase 7, F2 / FR-007): symlink-safe write-open primitive reachability.
//!
//! This is the gating spike for the long-deferred intermediate-component
//! symlink hardening (`contracts/symlink-guard.md`, research §R-1). It proves —
//! against the `rustix/fs` feature set ALREADY enabled transitively via
//! `tempfile` — that the two primitives the full guard (R2, later slice) needs
//! are reachable and actually refuse a symlinked path *component*, not just the
//! final node:
//!
//!   1. Linux: `rustix::fs::openat2` with `ResolveFlags::NO_SYMLINKS` resolves
//!      the whole path in-kernel and refuses ANY symlinked component in one
//!      syscall.
//!   2. Portable (incl. macOS, where `openat2` is Linux-only): a per-component
//!      `rustix::fs::openat` + `OFlags::NOFOLLOW` directory walk refuses a
//!      symlinked intermediate component, because each component is the *final*
//!      node of its own `openat` and `O_NOFOLLOW` rejects a final symlink on
//!      every Unix.
//!
//! Spike finding worth carrying into the R2 primitive: the *refusal Errno*
//! differs by platform/flag combination. On macOS, `openat` of a symlinked
//! directory component with `O_NOFOLLOW | O_DIRECTORY` returns `ENOTDIR`
//! (the symlink is "not a directory"), whereas `O_NOFOLLOW` *without*
//! `O_DIRECTORY`, and a symlinked final node, return `ELOOP`. Linux's
//! `RESOLVE_NO_SYMLINKS` returns `ELOOP`. The primitive must therefore treat the
//! *set* {`ELOOP`, `ENOTDIR`} as "refused symlinked component" rather than
//! pinning a single errno — the spike asserts membership in that set.
//!
//! Second finding: the walk must start from a *trusted, canonical* anchor
//! directory (the operator-owned root), not the filesystem root — otherwise it
//! trips over irrelevant system symlinks like macOS's `/tmp -> /private/tmp`.
//! This matches the FR-007 trust model exactly (operator owns the harness/
//! project tree; plugin content never supplies a path *component*), so the walk
//! anchors at the canonicalized base and validates only the components below it.
//!
//! This is intentionally MINIMAL — a reachability + refusal proof, not the SSOT
//! `src/util/symlink_safe.rs` primitive nor the all-sinks consolidation (those
//! land in the R2 slice, after the `harness/sync.rs` decomposition). No new
//! package is involved either way: `rustix` was promoted transitive→direct.
//!
//! Fixtures use a real `tempfile::tempdir` + `std::os::unix::fs::symlink`; no
//! mocks. Unix-only by construction (the symlink syscall + `rustix::fs` relative
//! opens); the suite is `#![cfg(unix)]`.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

use rustix::fd::OwnedFd;
use rustix::fs::{CWD, Mode, OFlags, openat};
use rustix::io::Errno;
use tempfile::tempdir;

/// Errnos that mean "an `O_NOFOLLOW` open refused a symlinked component."
/// `ELOOP` is the POSIX answer for a symlinked final node; macOS additionally
/// returns `ENOTDIR` when `O_DIRECTORY` is combined with `O_NOFOLLOW` on a
/// symlinked directory component (the symlink is not itself a directory).
fn is_symlink_refusal(err: Errno) -> bool {
    err == Errno::LOOP || err == Errno::NOTDIR
}

/// Open `anchor_fd`-relative `rel` by walking it one component at a time,
/// refusing to traverse a symlinked component. Each directory component is
/// opened relative to its parent's fd with `O_NOFOLLOW | O_DIRECTORY`, so a
/// symlinked intermediate is rejected (it is the final node of *its* `openat`,
/// and `O_NOFOLLOW` refuses a final symlink on every Unix — including macOS,
/// where `openat2` is unavailable). The final component is opened read-only,
/// also with `O_NOFOLLOW`, so a symlinked target is refused too.
///
/// `anchor_fd` is a *trusted* directory fd (in the real guard: the operator-
/// owned harness/project root, opened by canonical path); only the components
/// of `rel` below it are validated. Returns the opened final-node fd, or the
/// `Errno` from the first refused/failed `openat`. This mirrors the portable
/// arm the FR-007 SSOT primitive will adopt; here it is local to the spike.
fn open_no_follow_walk(anchor_fd: &OwnedFd, rel: &Path) -> Result<OwnedFd, Errno> {
    assert!(rel.is_relative(), "walk takes an anchor-relative path");

    let dir_flags = OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::RDONLY | OFlags::CLOEXEC;

    // Only plain `Normal` segments are expected from our fixtures; anything else
    // (`.`/`..`) is intentionally unsupported in this minimal spike.
    let segments: Vec<&std::ffi::OsStr> = rel
        .components()
        .map(|c| match c {
            std::path::Component::Normal(seg) => seg,
            other => panic!("spike walk only supports Normal components, got {other:?}"),
        })
        .collect();

    let Some((leaf, dirs)) = segments.split_last() else {
        panic!("walk requires a non-empty relative path");
    };

    // Re-open `.` on the anchor to get an independent owned fd to the same dir
    // (NOFOLLOW cleared — `.` is never a symlink) so the loop can uniformly
    // reassign `parent` each iteration without consuming the caller's anchor.
    let mut parent = openat(anchor_fd, ".", dir_flags & !OFlags::NOFOLLOW, Mode::empty())?;

    for seg in dirs {
        // Each interior component is the *final* node of this `openat`, so
        // `O_NOFOLLOW` refuses it if it is a symlink → the intermediate-symlink
        // refusal we are proving.
        parent = openat(&parent, *seg, dir_flags, Mode::empty())?;
    }

    // Final node: read-only, still NOFOLLOW so a symlinked leaf is refused.
    openat(
        &parent,
        *leaf,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
}

/// Open the operator-owned anchor directory by canonical path. No `O_NOFOLLOW`
/// here: the anchor is trusted (this is the FR-007 trust model — the operator
/// owns the tree root). Components *below* it are what the walk validates.
fn open_anchor(canonical_base: &Path) -> OwnedFd {
    openat(
        CWD,
        canonical_base,
        OFlags::DIRECTORY | OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .expect("open trusted canonical anchor dir")
}

/// PORTABLE (must pass on macOS): a per-component `openat` + `O_NOFOLLOW` walk
/// REFUSES a symlinked intermediate directory component, and ACCEPTS the same
/// relative path once the component is a real directory.
#[test]
fn portable_walk_refuses_symlinked_intermediate_component() {
    let root = tempdir().expect("tempdir");
    // Canonicalize so the anchor has no symlinked *system* components (macOS
    // `/tmp -> /private/tmp`); the walk below validates only what we plant.
    let base = root.path().canonicalize().expect("canonicalize base");
    let anchor = open_anchor(&base);

    // Real tree:  <base>/real_dir/leaf.txt
    let real_dir = base.join("real_dir");
    fs::create_dir(&real_dir).expect("mkdir real_dir");
    fs::write(real_dir.join("leaf.txt"), b"spike").expect("write leaf");

    // Sanity: the clean relative path through a real directory is ACCEPTED.
    open_no_follow_walk(&anchor, Path::new("real_dir/leaf.txt"))
        .expect("clean path through real dir must be accepted");

    // Symlinked intermediate:  <base>/link_dir -> <base>/real_dir
    // so `link_dir/leaf.txt` resolves to the same leaf via a symlinked
    // *component*. The walk must REFUSE at `link_dir`.
    symlink(&real_dir, base.join("link_dir")).expect("symlink link_dir -> real_dir");

    let err = open_no_follow_walk(&anchor, Path::new("link_dir/leaf.txt"))
        .expect_err("traversing a symlinked intermediate component must be refused");
    assert!(
        is_symlink_refusal(err),
        "expected a symlink refusal (ELOOP/ENOTDIR), got {err:?}"
    );

    // Defence-in-depth sanity: the leaf is genuinely reachable via the symlink
    // at the OS level, so the refusal is about the *component*, not a missing
    // file.
    assert!(
        base.join("link_dir/leaf.txt").exists(),
        "symlinked path resolves at the OS level"
    );
}

/// PORTABLE: a symlinked FINAL node is also refused by the walk's `O_NOFOLLOW`
/// leaf open — the final-node guarantee the project already had, expressed
/// through the same primitive.
#[test]
fn portable_walk_refuses_symlinked_final_node() {
    let root = tempdir().expect("tempdir");
    let base = root.path().canonicalize().expect("canonicalize base");
    let anchor = open_anchor(&base);

    fs::write(base.join("target.txt"), b"spike").expect("write target");
    symlink(base.join("target.txt"), base.join("link.txt"))
        .expect("symlink link.txt -> target.txt");

    let err = open_no_follow_walk(&anchor, Path::new("link.txt"))
        .expect_err("a symlinked final node must be refused by O_NOFOLLOW");
    assert!(
        is_symlink_refusal(err),
        "expected a symlink refusal (ELOOP/ENOTDIR), got {err:?}"
    );
}

/// LINUX-ONLY: `openat2(RESOLVE_NO_SYMLINKS)` is reachable under the enabled
/// feature set and refuses a symlinked intermediate component in a single
/// syscall; a clean path is accepted.
///
/// Gated to Linux because `rustix::fs::openat2` is Linux-only (the
/// `linux_raw`/`linux_kernel` backend); on macOS this arm is cfg'd out and the
/// portable walk above is the proof. Fixture construction is also Linux-gated
/// per the contract (APFS edge-fixture caveat, Phase 4 P3); the production check
/// is platform-independent.
#[cfg(target_os = "linux")]
#[test]
fn linux_openat2_no_symlinks_refuses_symlinked_intermediate_component() {
    use rustix::fs::{ResolveFlags, openat2};

    let root = tempdir().expect("tempdir");
    let base = root.path().canonicalize().expect("canonicalize base");
    let anchor = open_anchor(&base);

    let real_dir = base.join("real_dir");
    fs::create_dir(&real_dir).expect("mkdir real_dir");
    fs::write(real_dir.join("leaf.txt"), b"spike").expect("write leaf");

    let oflags = OFlags::RDONLY | OFlags::CLOEXEC;

    // Clean anchor-relative path is ACCEPTED: openat2 resolves it with no
    // symlink in any component below the trusted anchor.
    openat2(
        &anchor,
        "real_dir/leaf.txt",
        oflags,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS,
    )
    .expect("clean path must be accepted by openat2(NO_SYMLINKS)");

    // Symlinked intermediate component → REFUSED in-kernel.
    symlink(&real_dir, base.join("link_dir")).expect("symlink link_dir -> real_dir");

    let err = openat2(
        &anchor,
        "link_dir/leaf.txt",
        oflags,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS,
    )
    .expect_err("openat2(NO_SYMLINKS) must refuse a symlinked component");
    assert!(
        is_symlink_refusal(err),
        "expected a symlink refusal from RESOLVE_NO_SYMLINKS, got {err:?}"
    );
}
