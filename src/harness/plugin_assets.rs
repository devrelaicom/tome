//! Embedded harness-plugin (TypeScript shim) registry.
//!
//! Some harnesses (Cline, Pi, OpenCode) cannot run a native Tome session-start
//! hook, so Tome ships a small TypeScript plugin shim for each (authored under
//! `assets/harness-plugins/<harness>/tome.ts`, pulled into the binary by the
//! `build.rs` manifest generator — the same `include_bytes!` pipeline the
//! Phase-9 meta-skill manifest uses). The shim is executed by the **harness's
//! own runtime**, never by Tome, so the sync boundary (`src/mcp/` is the only
//! async island) holds.
//!
//! This submodule defines the `include!` target's struct shapes and exposes the
//! generated `HARNESS_PLUGINS` slice plus a small lookup. It lives under
//! `harness/` (NOT a new top-level module) per the Phase-11 constitution gate.
//!
//! Sync-only — `tests/sync_boundary.rs` guards this tree.

/// One file embedded as part of a harness shim.
///
/// Distinct from [`crate::authoring::meta::EmbeddedFile`]: same shape, but a
/// separate type in a separate module so neither `include!`d manifest depends
/// on the other. The build-time validation proves every `rel_path` is
/// `Normal`-only.
pub struct EmbeddedFile {
    /// POSIX-relative path inside the shim folder (`tome.ts`, …).
    pub rel_path: &'static str,
    pub bytes: &'static [u8],
}

/// One embedded harness shim — a record in the `build.rs`-generated manifest.
pub struct EmbeddedHarnessPlugin {
    /// The harness id (the subdir name); a safe path segment, validated at
    /// build time.
    pub harness: &'static str,
    /// The rel path of the required entrypoint (`tome.ts`), validated to exist
    /// exactly once at the folder root (FR-022).
    pub entrypoint: &'static str,
    pub files: &'static [EmbeddedFile],
}

// The generated `HARNESS_PLUGINS: &[EmbeddedHarnessPlugin]` slice (see
// build.rs). The struct names above are in scope at this site.
include!(concat!(env!("OUT_DIR"), "/harness_plugins_manifest.rs"));

/// Look up an embedded harness shim by harness id. Linear over a tiny
/// compile-time slice.
pub fn find(harness: &str) -> Option<&'static EmbeddedHarnessPlugin> {
    HARNESS_PLUGINS.iter().find(|p| p.harness == harness)
}

/// All embedded harness shims (registry order = build.rs sorted-by-id order).
pub fn all() -> &'static [EmbeddedHarnessPlugin] {
    HARNESS_PLUGINS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeds_the_three_expected_harness_shims() {
        for harness in ["cline", "pi", "opencode"] {
            let plugin =
                find(harness).unwrap_or_else(|| panic!("expected embedded shim for `{harness}`"));
            assert_eq!(plugin.harness, harness);
            assert_eq!(
                plugin.entrypoint, "tome.ts",
                "{harness} shim entrypoint must be tome.ts",
            );

            let entry = plugin
                .files
                .iter()
                .find(|f| f.rel_path == "tome.ts")
                .unwrap_or_else(|| panic!("{harness} shim must contain tome.ts"));
            assert!(
                !entry.bytes.is_empty(),
                "{harness} tome.ts must have non-empty bytes",
            );
        }
    }

    #[test]
    fn registry_has_at_least_the_three_known_shims() {
        let names: Vec<&str> = all().iter().map(|p| p.harness).collect();
        for expected in ["cline", "opencode", "pi"] {
            assert!(names.contains(&expected), "missing shim `{expected}`");
        }
    }
}
