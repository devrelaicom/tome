//! The Open Plugins `tome-op` portable-plugin emitter (Phase 11 / US4).
//!
//! Emitted for the `generic-op` and `goose` harnesses (contract
//! open-plugins-tome-op.md). A self-contained, portable Open Plugins plugin a
//! conformant host can install:
//!
//! ```text
//! <plugin-root>/
//! ├── .plugin/plugin.json
//! ├── hooks/hooks.json
//! ├── .mcp.json
//! └── AGENTS.md
//! ```
//!
//! The whole bundle is built in a sibling `.tome.tmp.*` staging dir and then
//! POSIX-atomic-renamed into `<plugin-root>` via
//! [`crate::util::land_directory_with_replace`] — the same all-or-nothing
//! discipline as [`crate::authoring::meta::install_skill`] /
//! [`crate::authoring::emit`]. A crash mid-populate leaves no debris; a partial
//! bundle is never observable.
//!
//! ## Containment / safety (mirrors the meta-skill + agent sinks)
//!
//! - [`crate::util::refuse_symlinked_component`] runs against the bundle root
//!   BEFORE the landing (`land_directory_with_replace` re-runs it internally,
//!   but the explicit pre-check gives the dedicated error shape, not the inner
//!   `Io`).
//! - Every emitted file name is re-asserted `Normal`-only at the write sink (the
//!   write-side `ensure_in_bounds` analogue) — the bundle's relative paths are
//!   `&'static` constants, but the assertion makes "validated by construction"
//!   load-bearing so a future edit can't silently introduce a `..`.
//! - The plugin name `"tome-op"` is validated against the Open Plugins name rule
//!   (lowercase alphanumeric / hyphen / period, start+end alphanumeric) — reusing
//!   the [`crate::plugin::identity`] validators.
//!
//! ## Launcher resolution (#290)
//!
//! The bundle's `.mcp.json` server command and SessionStart hook command are run
//! by the *host* (a CI runner / sandboxed non-IDE agent), whose `PATH` need not
//! contain `tome`. So both are emitted as an absolute launcher resolved by
//! [`tome_command`] (`$TOME_BIN` → `current_exe` → bare `"tome"` fallback), never
//! a bare-PATH name. Bundle ownership is by the `.plugin/plugin.json` `name`
//! field (see [`is_tome_op_bundle`]), NOT the command string, so the launcher is
//! free to vary per machine without affecting recognition / removal.
//!
//! ## JSON byte convention
//!
//! All JSON is rendered with `serde_json::to_vec_pretty` + a trailing newline —
//! the project's existing pretty+`\n` convention (matches `mcp_config::write_entry`
//! and `authoring::emit`). `serde_json`'s `preserve_order` feature keeps the key
//! order the constructors emit, so the bytes are byte-stable.
//!
//! Sync-only — `tests/sync_boundary.rs` guards this tree.

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::error::TomeError;
use crate::harness::rules_file;
use crate::paths::Paths;
use crate::plugin::identity::open_plugins_name_ok;

/// The Open Plugins plugin name Tome emits. Validated against the Open Plugins
/// name rule at every emit; the directory name + the manifest `name` field.
pub const TOME_OP_NAME: &str = "tome-op";

/// The four bundle files, as POSIX-relative paths inside `<plugin-root>`. The
/// write sink re-asserts each is a `Normal`-only relative path before writing.
const MANIFEST_REL: &str = ".plugin/plugin.json";
const HOOKS_REL: &str = "hooks/hooks.json";
const MCP_REL: &str = ".mcp.json";
const AGENTS_REL: &str = "AGENTS.md";

/// Env var overriding the launcher this bundle invokes (`TOME_BIN`). When set
/// and non-empty it wins the [`tome_command`] resolution; otherwise the running
/// binary's absolute path (`current_exe`) is used, falling back to the bare
/// `"tome"` name. The override is also the test seam for the byte pins.
const TOME_BIN_ENV: &str = "TOME_BIN";

/// Emit the whole `tome-op` Open Plugins bundle into `plugin_root`, atomically.
///
/// `project_root` locates the directive source (`<project>/.tome/RULES.md`) so
/// the bundle's `AGENTS.md` carries the SAME inline rules region the `generic`
/// AGENTS.md sink writes. `workspace` + `harness_name` are stamped into the
/// hooks/MCP commands (`--workspace <ws> --harness <name>`).
///
/// Builds in a `.tome.tmp.*` sibling staging dir then renames into place
/// (replacing any prior bundle). Idempotent: re-emitting produces byte-identical
/// content, so the landed bundle converges.
///
/// # Errors
///
/// - [`TomeError::HarnessNotSupported`] (exit 18) — the (constant) plugin name
///   fails the Open Plugins name rule. Defence-in-depth; never fires in practice.
/// - [`TomeError::Io`] (exit 7) — a symlinked component on the bundle path, a
///   bundle-relative path that is not `Normal`-only, or a generic write/rename
///   failure. The symlink refusal fails CLOSED with no write outside `plugin_root`.
pub fn emit_tome_op(
    plugin_root: &Path,
    project_root: &Path,
    workspace: &str,
    harness_name: &str,
) -> Result<(), TomeError> {
    validate_name()?;

    // Symlink-safe pre-write guard on the bundle root — fail closed BEFORE any
    // staging, mirroring `authoring::meta::install_skill`. `land_directory_with_replace`
    // re-runs this internally; the explicit pre-check keeps the contract's
    // "no write outside plugin_root" honest at this seam too.
    crate::util::refuse_symlinked_component(plugin_root).map_err(TomeError::Io)?;

    // The directive body identical to the `generic` AGENTS.md sink: the verbatim
    // `<project>/.tome/RULES.md` (self-heal preamble + tiered routing already
    // baked in by `routing::write_workspace_rules`). Absent → empty body (a
    // freshly-bound empty workspace), other IO errors propagate.
    let directive_body = read_inline_rules_body(project_root)?;

    // Resolve the launcher ONCE (#290), then thread the SAME value into both
    // sinks (the `.mcp.json` server command AND the SessionStart hook) so the
    // bundle never invokes a bare-PATH `tome` that a sandboxed / CI host can't
    // find — and so the two sinks can never diverge.
    let command = tome_command();

    let manifest = manifest_bytes();
    let hooks = hooks_bytes(&command, workspace, harness_name);
    let mcp = mcp_bytes(&command, workspace, harness_name);

    land_directory_with_replace_bundle(plugin_root, |staged| {
        write_bundle_file(staged, MANIFEST_REL, &manifest)?;
        write_bundle_file(staged, HOOKS_REL, &hooks)?;
        write_bundle_file(staged, MCP_REL, &mcp)?;
        // AGENTS.md carries the `<!-- tome:begin -->…<!-- tome:end -->` block
        // (Inline body) — byte-identical to the `generic` rules sink. Write it
        // into the staged file via the same block writer so the bytes match.
        let agents_path = bundle_target(staged, AGENTS_REL)?;
        rules_file::write_block(
            &agents_path,
            &directive_body,
            crate::harness::BlockBodyStyle::Inline,
        )?;
        Ok(())
    })
}

/// Remove the Tome-owned `tome-op` bundle at `plugin_root` (structural-match).
///
/// Mass-delete safeguard: the directory is removed ONLY when it is recognisably
/// the `tome-op` bundle — its `.plugin/plugin.json` exists and names `tome-op`.
/// A sibling directory, or a same-named dir that is NOT a tome-op plugin, is
/// left untouched (returns [`RemoveOutcome::NotTomeOp`]). Absent → no-op.
///
/// # Errors
///
/// - [`TomeError::Io`] (exit 7) — a symlinked component on the bundle path, or a
///   generic removal failure. Symlink refusal fails closed (nothing removed).
pub fn remove_tome_op(plugin_root: &Path) -> Result<RemoveOutcome, TomeError> {
    crate::util::refuse_symlinked_component(plugin_root).map_err(TomeError::Io)?;

    if !plugin_root.exists() {
        return Ok(RemoveOutcome::NotPresent);
    }
    if !is_tome_op_bundle(plugin_root) {
        // Not our bundle — never mass-delete a directory we don't own.
        return Ok(RemoveOutcome::NotTomeOp);
    }
    std::fs::remove_dir_all(plugin_root).map_err(TomeError::Io)?;
    Ok(RemoveOutcome::Removed)
}

/// Outcome of [`remove_tome_op`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveOutcome {
    /// The `tome-op` bundle existed and was deleted.
    Removed,
    /// Nothing at `plugin_root` (idempotent no-op).
    NotPresent,
    /// `plugin_root` exists but is NOT a recognisable `tome-op` bundle — left
    /// untouched by the mass-delete safeguard.
    NotTomeOp,
}

// =====================================================================
// Launcher resolution (#290)
// =====================================================================

/// Resolve the absolute launcher this bundle should invoke (`tome` issue #290).
///
/// The bundle's `.mcp.json` server command and its SessionStart hook command are
/// executed by the *host* (a CI runner or a sandboxed non-IDE agent), whose
/// `PATH` need not contain `tome`. A bare `"tome"` therefore silently fails to
/// start the MCP server and the agent gets zero skills. Resolution order:
///
/// 1. `$TOME_BIN`, if set and non-empty — an explicit operator override (and the
///    deterministic test seam, since `current_exe` is machine-specific).
/// 2. [`std::env::current_exe`] — the absolute path of the running binary, so the
///    emitted command points at the exact `tome` that ran the sync.
/// 3. The bare name `"tome"` — the old behaviour, used only when both above
///    fail (an exotic platform / a deleted binary). Never panics, never errors
///    the sync: this resolver is infallible by design.
///
/// A `current_exe` path that is not valid UTF-8 is treated as a resolution
/// failure (we cannot embed it in JSON / a shell command cleanly) and falls
/// through to the bare-name fallback.
fn tome_command() -> String {
    // (1) Explicit override wins.
    if let Some(value) = std::env::var_os(TOME_BIN_ENV)
        && !value.is_empty()
        && let Some(s) = value.to_str()
    {
        return s.to_string();
    }

    // (2) The running binary's absolute path. UTF-8-fail and `current_exe`-fail
    //     both fall through to the bare name.
    if let Ok(exe) = std::env::current_exe()
        && let Some(s) = exe.to_str()
    {
        return s.to_string();
    }

    // (3) Last-resort fallback: the old bare-PATH behaviour.
    "tome".to_string()
}

/// Quote a launcher path for safe interpolation into the SessionStart hook's
/// single shell-command string (`"<cmd> harness session-start …"`). An absolute
/// `current_exe` path can contain spaces (e.g. macOS `Application Support`),
/// which would otherwise split into multiple shell words. POSIX single-quoting
/// wraps the path and escapes any embedded single quote via the `'\''` idiom.
/// The bare name `"tome"` (no shell-special chars) is returned unquoted so the
/// fallback string stays identical to the historical bytes.
fn shell_quote(cmd: &str) -> String {
    if !cmd.is_empty() && cmd.bytes().all(is_shell_safe_byte) {
        return cmd.to_string();
    }
    format!("'{}'", cmd.replace('\'', "'\\''"))
}

/// Bytes that need no shell quoting (a conservative POSIX-portable set).
fn is_shell_safe_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/')
}

// =====================================================================
// Byte builders (byte-stable; serde_json preserve_order + trailing `\n`)
// =====================================================================

/// `.plugin/plugin.json` — the Open Plugins manifest.
fn manifest_bytes() -> Vec<u8> {
    let doc = json!({
        "name": TOME_OP_NAME,
        "version": env!("CARGO_PKG_VERSION"),
        "description": "Tome — cross-harness skill routing and MCP tools",
    });
    pretty_bytes(&doc)
}

/// `hooks/hooks.json` — the SessionStart command hook delivering the directive.
///
/// `launcher` is the resolved absolute path to the running `tome` (see
/// [`tome_command`]); it is shell-quoted because the hook `command` is a single
/// shell string (#290).
fn hooks_bytes(launcher: &str, workspace: &str, harness_name: &str) -> Vec<u8> {
    let command = format!(
        "{} harness session-start --workspace {workspace} --harness {harness_name}",
        shell_quote(launcher),
    );
    let doc = json!({
        "hooks": {
            "SessionStart": [
                { "hooks": [ { "type": "command", "command": command } ] }
            ]
        }
    });
    pretty_bytes(&doc)
}

/// `.mcp.json` — the Tome MCP server entry (`mcpServers` + `CommandArgs` + `env:{}`).
///
/// `launcher` is the resolved absolute path to the running `tome` (see
/// [`tome_command`]); it is the execve-style `command` (no shell, so no quoting)
/// alongside the `args` array (#290).
fn mcp_bytes(launcher: &str, workspace: &str, harness_name: &str) -> Vec<u8> {
    let doc = json!({
        "mcpServers": {
            "tome": {
                "command": launcher,
                "args": ["mcp", "--workspace", workspace, "--harness", harness_name],
                "env": {}
            }
        }
    });
    pretty_bytes(&doc)
}

/// `serde_json::to_vec_pretty` + a trailing newline (the project's JSON
/// convention; matches `mcp_config::write_entry`). `to_vec_pretty` cannot fail
/// for these `json!`-constructed values; the `.expect` makes the impossible
/// failure loud rather than silently writing `{}` debris into a bundle file.
fn pretty_bytes(value: &serde_json::Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(value).expect("json! value always serializes");
    bytes.push(b'\n');
    bytes
}

// =====================================================================
// Staging / write sink
// =====================================================================

/// Land the whole bundle atomically: build in a `.tome.tmp.*` sibling, then
/// rename into `plugin_root` (replacing any prior bundle). 0o755 dir mode so the
/// plugin dir is host-readable.
fn land_directory_with_replace_bundle<F>(plugin_root: &Path, populate: F) -> Result<(), TomeError>
where
    F: FnOnce(&Path) -> Result<(), TomeError>,
{
    crate::util::land_directory_with_replace(plugin_root, 0o755, populate)?;
    Ok(())
}

/// Resolve `<staged>/<rel>` after re-asserting every component of `rel` is a
/// `Normal`-only relative path (the write-side containment guard — the
/// `ensure_in_bounds` analogue). Then create the parent dir under the staged
/// root.
fn bundle_target(staged: &Path, rel: &str) -> Result<PathBuf, TomeError> {
    assert_normal_relative(rel)?;
    let target = staged.join(rel);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(TomeError::Io)?;
    }
    Ok(target)
}

/// Write one bundle file's bytes into the staged dir, after the `Normal`-only
/// assertion + parent creation.
fn write_bundle_file(staged: &Path, rel: &str, bytes: &[u8]) -> Result<(), TomeError> {
    let target = bundle_target(staged, rel)?;
    std::fs::write(&target, bytes).map_err(TomeError::Io)?;
    Ok(())
}

/// Assert every component of `rel` is a plain `Normal` path component — no
/// `..`, no absolute prefix, no root/cur-dir. The bundle's relative paths are
/// `&'static` constants, so this is defence-in-depth: an edit that introduced a
/// traversal would fail closed here, never escape the staged dir.
fn assert_normal_relative(rel: &str) -> Result<(), TomeError> {
    use std::path::Component;
    let path = Path::new(rel);
    let all_normal = path.components().all(|c| matches!(c, Component::Normal(_)));
    if !all_normal || rel.is_empty() {
        return Err(TomeError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("tome-op bundle path `{rel}` is not a Normal-only relative path"),
        )));
    }
    Ok(())
}

/// Validate the (constant) plugin name against the Open Plugins name rule.
fn validate_name() -> Result<(), TomeError> {
    if open_plugins_name_ok(TOME_OP_NAME) {
        Ok(())
    } else {
        // Unreachable for the constant; map to HarnessNotSupported (18) as the
        // closest closed-set variant for "this target's identity is invalid".
        Err(TomeError::HarnessNotSupported {
            name: TOME_OP_NAME.to_string(),
        })
    }
}

/// Read `<project>/.tome/RULES.md` verbatim (the inline directive body). Absent
/// → empty string; other IO errors propagate.
fn read_inline_rules_body(project_root: &Path) -> Result<String, TomeError> {
    let project_rules = Paths::project_marker_rules(project_root);
    match crate::util::bounded_read_to_string(&project_rules, crate::util::HARNESS_RULES_MAX) {
        Ok(s) => Ok(s),
        Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e),
    }
}

/// Structural-match recogniser: `plugin_root` is a `tome-op` bundle iff its
/// `.plugin/plugin.json` exists and names `tome-op`. A lenient read — a
/// malformed/oversize manifest is treated as "not ours" (fail closed; never
/// mass-delete what we cannot positively identify).
///
/// Read/write containment parity (m1, P8/P9 precedent): the manifest read is
/// routed through [`crate::util::refuse_symlinked_component`] BEFORE the read,
/// degrading a symlinked-component refusal to `false` ("not ours") — the same
/// guard the write sink runs. A bundle reachable only through a symlinked
/// component is not positively identified as ours, so removal is refused.
fn is_tome_op_bundle(plugin_root: &Path) -> bool {
    let manifest = plugin_root.join(MANIFEST_REL);
    // A symlinked component on the manifest path → treat as "not ours" (fail
    // closed; never read or mass-delete through a symlink).
    if crate::util::refuse_symlinked_component(&manifest).is_err() {
        return false;
    }
    let Ok(body) = crate::util::bounded_read_to_string(&manifest, crate::util::PLUGIN_MANIFEST_MAX)
    else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) else {
        return false;
    };
    value.get("name").and_then(|n| n.as_str()) == Some(TOME_OP_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::TempDir;

    /// Serialises every test that mutates `TOME_BIN` (process-global; `cargo
    /// test` runs a module's tests on multiple threads). Mirrors the `ENV_MUTEX`
    /// idiom used across the codebase (see `provider::config`, `telemetry`).
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    /// RAII guard: snapshot the named env vars, clear them, restore on drop.
    /// Holds `ENV_MUTEX` for its lifetime so a restore can't interleave with
    /// another test's set.
    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        saved: Vec<(String, Option<std::ffi::OsString>)>,
    }

    impl EnvGuard {
        fn new(vars: &[&str]) -> Self {
            let lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            let saved = vars
                .iter()
                .map(|&k| (k.to_string(), std::env::var_os(k)))
                .collect::<Vec<_>>();
            // SAFETY: ENV_MUTEX held for the guard's lifetime; no other test in
            // this module mutates these vars concurrently.
            for &k in vars {
                unsafe { std::env::remove_var(k) };
            }
            EnvGuard { _lock: lock, saved }
        }

        fn set(&self, key: &str, val: &str) {
            // SAFETY: guarded by ENV_MUTEX (held via `_lock`).
            unsafe { std::env::set_var(key, val) };
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: still holding ENV_MUTEX (dropped after this).
            for (k, v) in &self.saved {
                match v {
                    Some(val) => unsafe { std::env::set_var(k, val) },
                    None => unsafe { std::env::remove_var(k) },
                }
            }
        }
    }

    fn project_with_rules(body: &str) -> (TempDir, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("project");
        let rules = Paths::project_marker_rules(&project);
        std::fs::create_dir_all(rules.parent().unwrap()).unwrap();
        std::fs::write(&rules, body).unwrap();
        (tmp, project)
    }

    // ---- byte pins -------------------------------------------------------

    #[test]
    fn manifest_bytes_are_byte_stable() {
        let expected = format!(
            "{{\n  \"name\": \"tome-op\",\n  \"version\": \"{}\",\n  \"description\": \"Tome — cross-harness skill routing and MCP tools\"\n}}\n",
            env!("CARGO_PKG_VERSION"),
        );
        assert_eq!(String::from_utf8(manifest_bytes()).unwrap(), expected);
    }

    #[test]
    fn hooks_bytes_are_byte_stable() {
        // The launcher is the (resolved) command; pin against an explicit
        // absolute path so the bytes stay deterministic across machines.
        let expected = "{\n  \"hooks\": {\n    \"SessionStart\": [\n      {\n        \"hooks\": [\n          {\n            \"type\": \"command\",\n            \"command\": \"/usr/local/bin/tome harness session-start --workspace ws --harness goose\"\n          }\n        ]\n      }\n    ]\n  }\n}\n";
        assert_eq!(
            String::from_utf8(hooks_bytes("/usr/local/bin/tome", "ws", "goose")).unwrap(),
            expected
        );
    }

    #[test]
    fn mcp_bytes_are_byte_stable() {
        // The `.mcp.json` `command` is the resolved absolute launcher, NOT the
        // bare `tome` (#290).
        let expected = "{\n  \"mcpServers\": {\n    \"tome\": {\n      \"command\": \"/usr/local/bin/tome\",\n      \"args\": [\n        \"mcp\",\n        \"--workspace\",\n        \"ws\",\n        \"--harness\",\n        \"generic-op\"\n      ],\n      \"env\": {}\n    }\n  }\n}\n";
        assert_eq!(
            String::from_utf8(mcp_bytes("/usr/local/bin/tome", "ws", "generic-op")).unwrap(),
            expected
        );
    }

    // ---- emit → land → 4 files ------------------------------------------

    #[test]
    fn emit_lands_four_files_and_is_idempotent() {
        // Pin the launcher via TOME_BIN so the emitted commands are deterministic
        // (and the `command` fields are an exact absolute path, not bare `tome`).
        let guard = EnvGuard::new(&[TOME_BIN_ENV]);
        guard.set(TOME_BIN_ENV, "/opt/tome/bin/tome");

        let (_tmp, project) = project_with_rules("# rules body\n");
        let root = project.join(".config/goose/plugins/tome-op");

        emit_tome_op(&root, &project, "ws", "goose").expect("emit");

        assert!(root.join(".plugin/plugin.json").is_file());
        assert!(root.join("hooks/hooks.json").is_file());
        assert!(root.join(".mcp.json").is_file());
        assert!(root.join("AGENTS.md").is_file());

        // Both sinks carry the resolved absolute launcher, NOT bare `tome` (#290).
        let mcp = std::fs::read_to_string(root.join(".mcp.json")).unwrap();
        assert!(
            mcp.contains("\"command\": \"/opt/tome/bin/tome\""),
            "mcp.json command must be the absolute launcher; got:\n{mcp}",
        );
        let hooks = std::fs::read_to_string(root.join("hooks/hooks.json")).unwrap();
        assert!(
            hooks.contains("/opt/tome/bin/tome harness session-start"),
            "hook command must be the absolute launcher; got:\n{hooks}",
        );

        // AGENTS.md carries the tome block wrapping the verbatim rules body.
        let agents = std::fs::read_to_string(root.join("AGENTS.md")).unwrap();
        assert_eq!(
            agents,
            "<!-- tome:begin -->\n# rules body\n\n<!-- tome:end -->\n"
        );

        // Re-emit is byte-identical (idempotent landing).
        let manifest_a = std::fs::read(root.join(".plugin/plugin.json")).unwrap();
        emit_tome_op(&root, &project, "ws", "goose").expect("re-emit");
        let manifest_b = std::fs::read(root.join(".plugin/plugin.json")).unwrap();
        assert_eq!(manifest_a, manifest_b);
    }

    #[test]
    fn emit_with_absent_rules_writes_empty_block() {
        let _guard = EnvGuard::new(&[TOME_BIN_ENV]);
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let root = project.join("tome-op");
        emit_tome_op(&root, &project, "ws", "generic-op").expect("emit");
        let agents = std::fs::read_to_string(root.join("AGENTS.md")).unwrap();
        assert_eq!(agents, "<!-- tome:begin -->\n\n<!-- tome:end -->\n");
    }

    // ---- remove takes ONLY the tome-op bundle ----------------------------

    #[test]
    fn remove_takes_only_the_tome_op_bundle() {
        let (_tmp, project) = project_with_rules("# r\n");
        let plugins = project.join(".config/goose/plugins");
        let root = plugins.join("tome-op");
        emit_tome_op(&root, &project, "ws", "goose").unwrap();

        // A developer's sibling plugin in the SAME plugins dir.
        let sibling = plugins.join("their-plugin");
        std::fs::create_dir_all(&sibling).unwrap();
        std::fs::write(sibling.join("keep.txt"), b"mine").unwrap();

        assert_eq!(remove_tome_op(&root).unwrap(), RemoveOutcome::Removed);
        assert!(!root.exists(), "tome-op bundle removed");
        assert!(sibling.join("keep.txt").is_file(), "sibling survives");

        // Idempotent.
        assert_eq!(remove_tome_op(&root).unwrap(), RemoveOutcome::NotPresent);
    }

    #[test]
    fn remove_refuses_a_non_tome_op_directory() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("tome-op");
        std::fs::create_dir_all(&root).unwrap();
        // A directory named tome-op that is NOT a tome-op bundle (no manifest).
        std::fs::write(root.join("something.txt"), b"not ours").unwrap();
        assert_eq!(remove_tome_op(&root).unwrap(), RemoveOutcome::NotTomeOp);
        assert!(root.join("something.txt").is_file(), "left untouched");

        // A manifest naming a DIFFERENT plugin is also refused.
        std::fs::create_dir_all(root.join(".plugin")).unwrap();
        std::fs::write(
            root.join(".plugin/plugin.json"),
            br#"{"name":"not-tome-op","version":"1.0.0"}"#,
        )
        .unwrap();
        assert_eq!(remove_tome_op(&root).unwrap(), RemoveOutcome::NotTomeOp);
        assert!(root.exists(), "foreign plugin dir not deleted");
    }

    // ---- is_tome_op_bundle fail-closed branches (m3) ---------------------

    #[test]
    fn is_tome_op_bundle_recognises_a_real_bundle() {
        let (_tmp, project) = project_with_rules("# r\n");
        let root = project.join("tome-op");
        emit_tome_op(&root, &project, "ws", "goose").unwrap();
        assert!(is_tome_op_bundle(&root), "an emitted bundle is recognised");
    }

    #[test]
    fn is_tome_op_bundle_false_when_manifest_absent() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("tome-op");
        std::fs::create_dir_all(&root).unwrap();
        // No `.plugin/plugin.json` at all → not ours.
        assert!(!is_tome_op_bundle(&root));
    }

    #[test]
    fn is_tome_op_bundle_false_on_malformed_manifest() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("tome-op");
        std::fs::create_dir_all(root.join(".plugin")).unwrap();
        // Not valid JSON → fail closed (return false), never mass-delete.
        std::fs::write(root.join(MANIFEST_REL), b"{ this is not json").unwrap();
        assert!(!is_tome_op_bundle(&root));
    }

    #[test]
    fn is_tome_op_bundle_false_on_wrong_name() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("tome-op");
        std::fs::create_dir_all(root.join(".plugin")).unwrap();
        // Valid JSON naming a DIFFERENT plugin → not ours.
        std::fs::write(
            root.join(MANIFEST_REL),
            br#"{"name":"not-tome-op","version":"1.0.0"}"#,
        )
        .unwrap();
        assert!(!is_tome_op_bundle(&root));
    }

    #[test]
    fn is_tome_op_bundle_false_on_oversize_manifest() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("tome-op");
        std::fs::create_dir_all(root.join(".plugin")).unwrap();
        // A manifest over PLUGIN_MANIFEST_MAX → the bounded read errors → false
        // (fail closed; never positively identify an oversize file as ours).
        let oversize = vec![b' '; (crate::util::PLUGIN_MANIFEST_MAX as usize) + 1];
        std::fs::write(root.join(MANIFEST_REL), &oversize).unwrap();
        assert!(!is_tome_op_bundle(&root));
    }

    #[cfg(unix)]
    #[test]
    fn is_tome_op_bundle_false_when_manifest_is_a_symlink() {
        // Read/write containment parity (m1): the `refuse_symlinked_component`
        // guard runs against the manifest path before the read. The guard
        // refuses a symlinked component that lands in its walked tail — here the
        // manifest's own `.plugin` directory is a symlink to a sibling holding a
        // real tome-op manifest. The guard refuses the symlinked component, so
        // identification degrades to `false` ("not ours") and removal is refused.
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        // A sibling directory holding a real tome-op manifest.
        let real_plugin_dir = base.join("real_plugin");
        std::fs::create_dir_all(&real_plugin_dir).unwrap();
        std::fs::write(
            real_plugin_dir.join("plugin.json"),
            format!(
                r#"{{"name":"tome-op","version":"{}"}}"#,
                env!("CARGO_PKG_VERSION")
            ),
        )
        .unwrap();

        // The bundle root: a REAL directory whose `.plugin` child is a SYMLINK to
        // the sibling. `.plugin` is the symlinked component on the manifest path
        // and has no real-directory descendant of its own under the root, so it
        // lands in the guard's walked tail and is refused.
        let root = base.join("tome-op");
        std::fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(&real_plugin_dir, root.join(".plugin")).unwrap();

        assert!(
            !is_tome_op_bundle(&root),
            "a symlinked manifest-path component must fail closed to 'not ours'",
        );
    }

    // ---- name validation -------------------------------------------------

    #[test]
    fn tome_op_name_passes_open_plugins_rule() {
        assert!(open_plugins_name_ok(TOME_OP_NAME));
        // The emitter's name guard accepts the constant.
        assert!(validate_name().is_ok());
    }

    // ---- Normal-only assertion at the write sink -------------------------

    #[test]
    fn bundle_target_rejects_traversal() {
        let tmp = TempDir::new().unwrap();
        assert!(bundle_target(tmp.path(), "../escape").is_err());
        assert!(bundle_target(tmp.path(), "/abs").is_err());
        assert!(bundle_target(tmp.path(), "").is_err());
        // The real bundle rel-paths all pass.
        for rel in [MANIFEST_REL, HOOKS_REL, MCP_REL, AGENTS_REL] {
            assert!(bundle_target(tmp.path(), rel).is_ok(), "{rel}");
        }
    }

    // ---- symlink refusal on the bundle landing ---------------------------

    #[cfg(unix)]
    #[test]
    fn emit_refuses_symlinked_bundle_component() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        let project = base.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let real = base.join("real");
        std::fs::create_dir_all(&real).unwrap();
        // `plugins` is a symlink to `real` — a symlinked component of the root.
        std::os::unix::fs::symlink(&real, base.join("plugins")).unwrap();

        let root = base.join("plugins").join("tome-op");
        let err = emit_tome_op(&root, &project, "ws", "goose")
            .expect_err("symlinked component must be refused");
        assert_eq!(err.exit_code(), 7, "got {err:?}");
        assert!(
            !real.join("tome-op").exists(),
            "no bundle landed through the symlink"
        );
    }

    // ---- launcher resolution (#290) -------------------------------------

    #[test]
    fn tome_command_honors_tome_bin_override() {
        let guard = EnvGuard::new(&[TOME_BIN_ENV]);
        guard.set(TOME_BIN_ENV, "/custom/path/to/tome");
        assert_eq!(tome_command(), "/custom/path/to/tome");
    }

    #[test]
    fn tome_command_falls_back_to_current_exe_when_override_unset() {
        // With TOME_BIN unset, the resolver returns the running binary's path —
        // which is absolute (the test binary) and, crucially, NOT the bare name.
        let _guard = EnvGuard::new(&[TOME_BIN_ENV]);
        let cmd = tome_command();
        let exe = std::env::current_exe().expect("current_exe");
        assert_eq!(
            cmd,
            exe.to_str().expect("test binary path is UTF-8"),
            "with TOME_BIN unset, the launcher is current_exe",
        );
        assert_ne!(cmd, "tome", "the launcher must not be the bare name");
        assert!(
            Path::new(&cmd).is_absolute(),
            "the resolved launcher must be an absolute path; got {cmd}",
        );
    }

    #[test]
    fn tome_command_ignores_empty_override() {
        // An empty TOME_BIN is treated as unset → falls through to current_exe,
        // never emitting an empty command.
        let guard = EnvGuard::new(&[TOME_BIN_ENV]);
        guard.set(TOME_BIN_ENV, "");
        let cmd = tome_command();
        assert!(!cmd.is_empty());
        assert_ne!(cmd, "");
    }

    #[test]
    fn shell_quote_leaves_simple_paths_unquoted() {
        assert_eq!(shell_quote("tome"), "tome");
        assert_eq!(shell_quote("/usr/local/bin/tome"), "/usr/local/bin/tome");
        assert_eq!(
            shell_quote("/opt/tome-1.2/bin/tome"),
            "/opt/tome-1.2/bin/tome"
        );
    }

    #[test]
    fn shell_quote_wraps_paths_with_spaces() {
        assert_eq!(
            shell_quote("/Applications/My Tome.app/tome"),
            "'/Applications/My Tome.app/tome'",
        );
    }

    #[test]
    fn shell_quote_escapes_embedded_single_quote() {
        // The POSIX `'\''` idiom closes the quote, escapes a literal quote, reopens.
        assert_eq!(shell_quote("/o'dd/tome"), "'/o'\\''dd/tome'");
    }

    #[test]
    fn hook_command_quotes_a_spaced_launcher() {
        let bytes = hooks_bytes("/Applications/My Tome.app/tome", "ws", "goose");
        let s = String::from_utf8(bytes).unwrap();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        let command = v["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert_eq!(
            command,
            "'/Applications/My Tome.app/tome' harness session-start --workspace ws --harness goose",
        );
    }
}
