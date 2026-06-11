//! Typed, emit-only telemetry event records (Phase 10).
//!
//! Everything here is a closed, `Serialize`-only record — the anonymous stream
//! never lets a free-form string off the box. There is no `deny_unknown_fields`
//! (that is reserved for *inputs*; these are outputs).

use serde::Serialize;

/// The host operating system, as a closed enum (NFR-012). `Windows` is a
/// RESERVED value: no runtime target on our build matrix maps to it today, but
/// it stays in the enum so a future Windows port serialises a known token
/// rather than a junk string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Macos,
    Linux,
    Windows,
}

/// The host CPU architecture, as a closed enum. The per-variant renames pin the
/// wire tokens exactly (`x86_64`/`aarch64`) — `rename_all = "lowercase"` would
/// not reproduce the underscores/digits faithfully.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Arch {
    #[serde(rename = "x86_64")]
    X86_64,
    #[serde(rename = "aarch64")]
    Aarch64,
}

// The enums are TOTAL by construction (FR-023a, research §R-3): a source build
// for a target outside the supported matrix fails HERE rather than shipping a
// value the `CURRENT_*` resolvers below cannot map. Supported matrix:
// (macos | linux) × (x86_64 | aarch64).
#[cfg(not(any(
    all(target_os = "macos", target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "aarch64"),
)))]
compile_error!("telemetry os/arch enum: unsupported target — extend src/telemetry/event.rs");

/// This binary's OS, resolved at compile time from `cfg!(target_os)`. The
/// compile-error guard above guarantees exactly one arm matches.
pub const CURRENT_OS: Os = if cfg!(target_os = "macos") {
    Os::Macos
} else {
    // Only `linux` remains after the compile-error guard rules out everything
    // else; `Windows` is reserved and never reached at runtime on our matrix.
    Os::Linux
};

/// This binary's architecture, resolved at compile time from `cfg!(target_arch)`.
pub const CURRENT_ARCH: Arch = if cfg!(target_arch = "x86_64") {
    Arch::X86_64
} else {
    // Only `aarch64` remains after the compile-error guard.
    Arch::Aarch64
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_serialises_lowercase() {
        assert_eq!(serde_json::to_string(&Os::Macos).unwrap(), "\"macos\"");
        assert_eq!(serde_json::to_string(&Os::Linux).unwrap(), "\"linux\"");
        assert_eq!(serde_json::to_string(&Os::Windows).unwrap(), "\"windows\"");
    }

    #[test]
    fn arch_serialises_with_pinned_tokens() {
        assert_eq!(serde_json::to_string(&Arch::X86_64).unwrap(), "\"x86_64\"");
        assert_eq!(
            serde_json::to_string(&Arch::Aarch64).unwrap(),
            "\"aarch64\""
        );
    }

    #[test]
    fn current_target_resolves_to_a_known_value() {
        // Compiles + runs only on the supported matrix (the compile-error guard
        // enforces that), so these are always among the mapped variants.
        assert!(matches!(CURRENT_OS, Os::Macos | Os::Linux));
        assert!(matches!(CURRENT_ARCH, Arch::X86_64 | Arch::Aarch64));
    }
}
