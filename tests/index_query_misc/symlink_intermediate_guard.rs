//! FR-007: symlink-safe write guard across ALL sinks (intermediate-component).
//!
//! Two layers of proof live here:
//!
//!   1. **Primitive backends** (the original F2 spike, kept verbatim): direct
//!      proof that the two `rustix` mechanisms the SSOT primitive
//!      (`src/util/symlink_safe.rs`) is built on are reachable and refuse a
//!      symlinked *component* — Linux `openat2(RESOLVE_NO_SYMLINKS)` in one
//!      syscall, and the portable per-component `openat` + `OFlags::NOFOLLOW`
//!      walk (the macOS path). These exercise the raw syscalls so a future
//!      `rustix`/platform change that breaks the mechanism fails loudly here,
//!      independently of the wiring.
//!
//!   2. **Per-sink wiring** (the R2 obligation): for every Tome-managed write
//!      sink, plant a symlink as an **intermediate** directory component on
//!      that sink's real write path, drive the sink's public writer, and assert
//!      the write is REFUSED with that sink's DEDICATED exit code — never a
//!      regression to generic `Io` (7) on a dedicated sink. The final-node
//!      refusal (the guarantee the project already had) is asserted too, now
//!      flowing through the same SSOT primitive.
//!
//! Sink → dedicated exit code coverage:
//!
//! | Sink                         | exit | proven in                       |
//! |------------------------------|------|---------------------------------|
//! | Hooks `settings.local.json`  | 44   | `sinks::hooks_*` (here)         |
//! | Guardrails region + sibling  | 46   | `sinks::guardrails_*` (here)    |
//! | Rules file                   |  7   | `sinks::rules_file_*` (here)    |
//! | MCP config                   |  7   | `sinks::mcp_config_*` (here)    |
//! | Atomic dir landing           |  7   | `sinks::atomic_dir_*` (here)    |
//! | Catalog registry             |  7   | `sinks::catalog_store_*` (here) |
//! | Agent files                  | 45   | unit test in `reconcile/agents.rs` (private write path) |
//!
//! The agents sink's symlink→exit-45 mapping lives on the private
//! `write_agent_file` (the public `reconcile_agents` entry needs the full
//! DB/registry plumbing); it is proven by a focused unit test in
//! `src/harness/reconcile/agents.rs` rather than re-plumbed here. SC-006:
//! refused 100% across supported platforms — macOS runs the portable walk;
//! Linux runs `openat2`; both via the one primitive every sink delegates to.
//!
//! Fixtures use a real `tempfile::tempdir` + `std::os::unix::fs::symlink`; no
//! mocks. Unix-only by construction (the symlink syscall + `rustix::fs` relative
//! opens); the suite is `#![cfg(unix)]`.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

use rustix::fd::OwnedFd;
use rustix::fs::{CWD, Mode, OFlags, openat};
use rustix::io::Errno;
use tempfile::tempdir;

// =====================================================================
// Layer 1 — primitive backends (the F2 spike, kept verbatim)
// =====================================================================

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
/// arm the FR-007 SSOT primitive adopts; here it is local to the spike.
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

// =====================================================================
// Layer 2 — per-sink wiring (the R2 obligation)
//
// Every test plants a symlinked INTERMEDIATE directory component on the
// sink's real write path, drives the sink's PUBLIC writer, and asserts the
// dedicated exit code; a sibling test asserts the FINAL-node refusal still
// holds. A canonicalized tempdir base keeps the trusted anchor free of system
// symlinks (macOS `/tmp -> /private/tmp`), so only the planted symlink trips
// the guard.
// =====================================================================
mod sinks {
    use super::{fs, symlink};
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    use tome::error::TomeError;
    use tome::harness::guardrails;
    use tome::harness::hooks::{self, RewrittenHooks};
    use tome::harness::mcp_config::{self, TomeEntry};
    use tome::harness::rules_file;
    use tome::harness::{BlockBodyStyle, MCP_CONFIG_KEY, McpConfigFormat};

    /// A canonicalized tempdir base (trusted anchor, no system symlinks) plus a
    /// real directory and a symlink aimed at it. Returns `(base, real_dir)`
    /// where `<base>/link -> <base>/real`, so any path under `<base>/link/...`
    /// traverses a symlinked INTERMEDIATE component.
    fn intermediate_symlink_fixture() -> (tempfile::TempDir, PathBuf) {
        let root = tempdir().expect("tempdir");
        let base = root.path().canonicalize().expect("canonicalize base");
        let real = base.join("real");
        fs::create_dir(&real).expect("mkdir real");
        symlink(&real, base.join("link")).expect("symlink link -> real");
        (root, real)
    }

    /// `<base>/link/<name>` — a write target whose parent (`link`) is a symlink.
    fn through_symlinked_dir(base: &Path, name: &str) -> PathBuf {
        base.join("link").join(name)
    }

    // --- Hooks sink → exit 44 (HookSettingsWriteFailed) -----------------

    fn one_hook() -> RewrittenHooks {
        // Minimal non-empty rewritten hook: one PreToolUse entry.
        RewrittenHooks {
            events: vec![(
                "PreToolUse".to_string(),
                vec![serde_json::json!({ "command": "/abs/tool.sh" })],
            )],
        }
    }

    #[test]
    fn hooks_refuses_symlinked_intermediate_with_exit_44() {
        let (root, _real) = intermediate_symlink_fixture();
        let base = root.path().canonicalize().unwrap();
        let target = through_symlinked_dir(&base, "settings.local.json");

        let err = hooks::merge_into_settings(&target, &one_hook())
            .expect_err("hooks write through a symlinked intermediate must be refused");
        assert_eq!(
            err.exit_code(),
            44,
            "hooks sink must map a symlink refusal to HookSettingsWriteFailed (44), not Io (7); got {err:?}"
        );
        assert!(
            matches!(err, TomeError::HookSettingsWriteFailed { .. }),
            "expected HookSettingsWriteFailed, got {err:?}"
        );
    }

    #[test]
    fn hooks_refuses_symlinked_final_node_with_exit_44() {
        let root = tempdir().expect("tempdir");
        let base = root.path().canonicalize().expect("canonicalize");
        fs::write(base.join("decoy.json"), b"{}").expect("write decoy");
        let target = base.join("settings.local.json");
        symlink(base.join("decoy.json"), &target).expect("symlink final node");

        let err = hooks::merge_into_settings(&target, &one_hook())
            .expect_err("hooks write through a symlinked final node must be refused");
        assert_eq!(err.exit_code(), 44, "got {err:?}");
    }

    // --- Guardrails sink → exit 46 (GuardrailsWriteFailed) --------------

    fn one_region() -> std::collections::BTreeMap<String, String> {
        let mut m = std::collections::BTreeMap::new();
        m.insert("cat:plug".to_string(), "be careful\n".to_string());
        m
    }

    #[test]
    fn guardrails_in_file_refuses_symlinked_intermediate_with_exit_46() {
        let (root, _real) = intermediate_symlink_fixture();
        let base = root.path().canonicalize().unwrap();
        let target = through_symlinked_dir(&base, "CLAUDE.md");

        let err = guardrails::reconcile_in_file_region(&target, &one_region())
            .expect_err("guardrails write through a symlinked intermediate must be refused");
        assert_eq!(
            err.exit_code(),
            46,
            "guardrails sink must map a symlink refusal to GuardrailsWriteFailed (46), not Io (7); got {err:?}"
        );
        assert!(
            matches!(err, TomeError::GuardrailsWriteFailed { .. }),
            "expected GuardrailsWriteFailed, got {err:?}"
        );
    }

    #[test]
    fn guardrails_sibling_refuses_symlinked_intermediate_with_exit_46() {
        let (root, _real) = intermediate_symlink_fixture();
        let base = root.path().canonicalize().unwrap();
        let target = through_symlinked_dir(&base, "TOME_GUARDRAILS.md");

        let err = guardrails::reconcile_standalone_sibling(&target, &one_region()).expect_err(
            "guardrails sibling write through a symlinked intermediate must be refused",
        );
        assert_eq!(err.exit_code(), 46, "got {err:?}");
    }

    #[test]
    fn guardrails_in_file_refuses_symlinked_final_node_with_exit_46() {
        let root = tempdir().expect("tempdir");
        let base = root.path().canonicalize().expect("canonicalize");
        fs::write(base.join("decoy.md"), b"x").expect("write decoy");
        let target = base.join("CLAUDE.md");
        symlink(base.join("decoy.md"), &target).expect("symlink final node");

        let err = guardrails::reconcile_in_file_region(&target, &one_region())
            .expect_err("guardrails write through a symlinked final node must be refused");
        assert_eq!(err.exit_code(), 46, "got {err:?}");
    }

    // --- Rules-file sink → exit 7 (Io) ---------------------------------
    // A dedicated sink must never regress to generic Io; the rules file's
    // dedicated code IS 7 (it has no narrower variant), so 7 is correct here —
    // and crucially still REFUSED, not silently followed.

    #[test]
    fn rules_file_refuses_symlinked_intermediate_with_exit_7() {
        let (root, _real) = intermediate_symlink_fixture();
        let base = root.path().canonicalize().unwrap();
        let target = through_symlinked_dir(&base, "RULES.md");

        let err = rules_file::write_standalone(&target, "rules\n")
            .expect_err("rules-file write through a symlinked intermediate must be refused");
        assert_eq!(
            err.exit_code(),
            7,
            "rules-file refusal stays on its dedicated Io code (7); got {err:?}"
        );
        assert!(matches!(err, TomeError::Io(_)), "expected Io, got {err:?}");
    }

    #[test]
    fn rules_file_block_refuses_symlinked_intermediate_with_exit_7() {
        let (root, _real) = intermediate_symlink_fixture();
        let base = root.path().canonicalize().unwrap();
        let target = through_symlinked_dir(&base, "AGENTS.md");

        let err = rules_file::write_block(&target, "body\n", BlockBodyStyle::Inline)
            .expect_err("rules-file block write through a symlinked intermediate must be refused");
        assert_eq!(err.exit_code(), 7, "got {err:?}");
    }

    #[test]
    fn rules_file_refuses_symlinked_final_node_with_exit_7() {
        let root = tempdir().expect("tempdir");
        let base = root.path().canonicalize().expect("canonicalize");
        fs::write(base.join("decoy.md"), b"x").expect("write decoy");
        let target = base.join("RULES.md");
        symlink(base.join("decoy.md"), &target).expect("symlink final node");

        let err = rules_file::write_standalone(&target, "rules\n")
            .expect_err("rules-file write through a symlinked final node must be refused");
        assert_eq!(err.exit_code(), 7, "got {err:?}");
    }

    // --- MCP-config sink → exit 7 (Io) ---------------------------------

    #[test]
    fn mcp_config_refuses_symlinked_intermediate_with_exit_7() {
        let (root, _real) = intermediate_symlink_fixture();
        let base = root.path().canonicalize().unwrap();
        let target = through_symlinked_dir(&base, ".mcp.json");

        let entry = TomeEntry::new("tome".to_string(), vec!["mcp".to_string()]);
        let err = mcp_config::write_entry(&target, McpConfigFormat::Json, "mcpServers", &entry)
            .expect_err("mcp-config write through a symlinked intermediate must be refused");
        assert_eq!(
            err.exit_code(),
            7,
            "mcp-config refusal stays on its dedicated Io code (7); got {err:?}"
        );
        assert!(matches!(err, TomeError::Io(_)), "expected Io, got {err:?}");
    }

    #[test]
    fn mcp_config_refuses_symlinked_final_node_with_exit_7() {
        let root = tempdir().expect("tempdir");
        let base = root.path().canonicalize().expect("canonicalize");
        fs::write(base.join("decoy.json"), b"{}").expect("write decoy");
        let target = base.join(".mcp.json");
        symlink(base.join("decoy.json"), &target).expect("symlink final node");

        let entry = TomeEntry::new("tome".to_string(), vec!["mcp".to_string()]);
        let err = mcp_config::write_entry(&target, McpConfigFormat::Json, "mcpServers", &entry)
            .expect_err("mcp-config write through a symlinked final node must be refused");
        assert_eq!(err.exit_code(), 7, "got {err:?}");
        let _ = MCP_CONFIG_KEY; // keep the import meaningful across refactors
    }

    // --- Atomic-dir landing sink → exit 7 (Io) -------------------------

    #[test]
    fn atomic_dir_refuses_symlinked_intermediate_with_exit_7() {
        let (root, _real) = intermediate_symlink_fixture();
        let base = root.path().canonicalize().unwrap();
        // Land a directory at <base>/link/ws — `link` is a symlinked
        // intermediate component on the landing path.
        let target = through_symlinked_dir(&base, "ws");

        let err = tome::util::land_directory(&target, 0o700, |staged| {
            fs::write(staged.join("marker"), b"x").map_err(TomeError::Io)
        })
        .expect_err("directory landing through a symlinked intermediate must be refused");
        assert_eq!(
            err.exit_code(),
            7,
            "atomic-dir refusal stays on its dedicated Io code (7); got {err:?}"
        );
        assert!(matches!(err, TomeError::Io(_)), "expected Io, got {err:?}");
    }

    #[test]
    fn atomic_dir_refuses_symlinked_final_node_with_exit_7() {
        let root = tempdir().expect("tempdir");
        let base = root.path().canonicalize().expect("canonicalize");
        // The landing target itself is a symlink (final-node refusal).
        fs::create_dir(base.join("real_ws")).expect("mkdir real_ws");
        let target = base.join("ws");
        symlink(base.join("real_ws"), &target).expect("symlink final node");

        let err = tome::util::land_directory(&target, 0o700, |staged| {
            fs::write(staged.join("marker"), b"x").map_err(TomeError::Io)
        })
        .expect_err("directory landing onto a symlinked target must be refused");
        assert_eq!(err.exit_code(), 7, "got {err:?}");
    }

    // --- Catalog-store sink → exit 7 (Io) ------------------------------

    #[test]
    fn catalog_store_refuses_symlinked_intermediate_with_exit_7() {
        let (root, _real) = intermediate_symlink_fixture();
        let base = root.path().canonicalize().unwrap();
        let target = through_symlinked_dir(&base, "config.toml");

        let err = tome::catalog::store::write_atomic(&target, b"x = 1\n")
            .expect_err("registry write through a symlinked intermediate must be refused");
        assert_eq!(
            err.exit_code(),
            7,
            "catalog-store refusal stays on its dedicated Io code (7); got {err:?}"
        );
        assert!(matches!(err, TomeError::Io(_)), "expected Io, got {err:?}");
    }

    #[test]
    fn catalog_store_refuses_symlinked_final_node_with_exit_7() {
        let root = tempdir().expect("tempdir");
        let base = root.path().canonicalize().expect("canonicalize");
        fs::write(base.join("decoy.toml"), b"x = 1\n").expect("write decoy");
        let target = base.join("config.toml");
        symlink(base.join("decoy.toml"), &target).expect("symlink final node");

        let err = tome::catalog::store::write_atomic(&target, b"y = 2\n")
            .expect_err("registry write through a symlinked final node must be refused");
        assert_eq!(err.exit_code(), 7, "got {err:?}");
    }

    // --- Cross-sink sanity: normal (non-symlink) writes still succeed ---
    // Defence-in-depth must not break the happy path; a clean path through
    // real directories writes successfully through each public writer. (The
    // SyncOutcome JSON pins in the harness suites cover the orchestrated
    // happy path; this is a fast direct smoke test.)

    #[test]
    fn clean_paths_through_real_dirs_still_write() {
        let root = tempdir().expect("tempdir");
        let base = root.path().canonicalize().expect("canonicalize");
        let dir = base.join("real");
        fs::create_dir(&dir).expect("mkdir real");

        rules_file::write_standalone(&dir.join("RULES.md"), "rules\n").expect("rules write");
        assert!(dir.join("RULES.md").is_file());

        guardrails::reconcile_in_file_region(&dir.join("CLAUDE.md"), &one_region())
            .expect("guardrails write");
        assert!(dir.join("CLAUDE.md").is_file());

        let entry = TomeEntry::new("tome".to_string(), vec!["mcp".to_string()]);
        mcp_config::write_entry(
            &dir.join(".mcp.json"),
            McpConfigFormat::Json,
            "mcpServers",
            &entry,
        )
        .expect("mcp write");
        assert!(dir.join(".mcp.json").is_file());

        tome::catalog::store::write_atomic(&dir.join("config.toml"), b"x = 1\n")
            .expect("registry write");
        assert!(dir.join("config.toml").is_file());

        let _ = hooks::merge_into_settings(&dir.join("settings.local.json"), &one_hook())
            .expect("hooks write");
        assert!(dir.join("settings.local.json").is_file());
    }
}
