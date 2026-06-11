//! Best-effort detection of how the running binary was installed (FR-033).
//!
//! The signal feeds `tome.install.install_method`. It is a coarse, closed enum
//! ([`InstallMethod`]) — never a path or a free-form string — so it cannot
//! fingerprint. Detection is heuristic over the canonicalized
//! `std::env::current_exe()` path; ANY failure (no `current_exe`, non-UTF-8
//! path) collapses to [`InstallMethod::Unknown`] rather than guessing. The
//! precedence and the exact path patterns are documented in `TELEMETRY.md`.

use std::path::Path;

use crate::telemetry::event::InstallMethod;

/// Homebrew install markers. A `Cellar` path segment is the canonical signal
/// (every brewed binary lives under `…/Cellar/<formula>/…`), and the platform
/// `opt` prefixes catch the symlinked `bin` front-doors before canonicalization
/// would have resolved them into the Cellar.
const BREW_MARKERS: &[&str] = &[
    "/opt/homebrew/",     // Apple-silicon default prefix.
    "/usr/local/Cellar/", // Intel-mac default Cellar.
    "/home/linuxbrew/",   // Linuxbrew default prefix.
];

/// Detect the install method of the *running* binary (best-effort).
///
/// Resolves `current_exe`, canonicalizes it (so a symlinked front-door resolves
/// to its real location), and classifies via [`classify_install_path`] against
/// the user's home directory. Any failure ⇒ [`InstallMethod::Unknown`].
pub fn detect_install_method() -> InstallMethod {
    // No `current_exe` (an exotic platform / deleted binary) ⇒ Unknown.
    let Ok(exe) = std::env::current_exe() else {
        return InstallMethod::Unknown;
    };
    // Canonicalize so a Homebrew `bin` symlink resolves into the real Cellar
    // path and a relative invocation becomes absolute. A canonicalize failure
    // (e.g. the binary was unlinked) falls back to the raw path — still useful
    // for the cargo/dist prefix checks.
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);

    // Home is only needed for the `~/.cargo/bin` and `~/.local/bin` fallbacks;
    // a missing HOME just means those two checks can't fire (⇒ they're skipped
    // inside `classify_install_path`, which tolerates an empty home).
    let home = crate::paths::home_root().unwrap_or_default();

    classify_install_path(&exe, &home)
}

/// Classify a (canonicalized) executable path into an [`InstallMethod`] given
/// the user's home directory. Injectable so the precedence logic is unit-tested
/// without depending on where the test binary actually lives.
///
/// PRECEDENCE (FR-033, first match wins):
/// 1. Homebrew — a `Cellar` path segment OR a known opt/prefix tree ⇒ `Brew`.
/// 2. Cargo — under `$CARGO_HOME/bin` or `~/.cargo/bin` ⇒ `Cargo`.
/// 3. cargo-dist — under the installer's default `~/.local/bin` ⇒ `Curl`.
/// 4. Otherwise ⇒ `Unknown` (we never guess).
///
/// A non-UTF-8 path can't be matched against our `&str` markers, so it falls
/// straight through to `Unknown`.
pub fn classify_install_path(exe: &Path, home: &Path) -> InstallMethod {
    // Non-UTF-8 path ⇒ can't classify against string markers ⇒ Unknown.
    let Some(exe_str) = exe.to_str() else {
        return InstallMethod::Unknown;
    };

    // 1. Homebrew. A `Cellar` directory component is the strongest signal; the
    //    opt/prefix trees catch the pre-canonicalization symlink front-doors.
    if exe
        .components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case("Cellar"))
        || BREW_MARKERS.iter().any(|m| exe_str.contains(m))
    {
        return InstallMethod::Brew;
    }

    // 2. Cargo. `$CARGO_HOME/bin` (explicit override) wins over the default
    //    `~/.cargo/bin`; both are checked as path *prefixes* so a binary merely
    //    *named* `.cargo` elsewhere doesn't false-match.
    if let Some(cargo_home) = std::env::var_os("CARGO_HOME") {
        let cargo_bin = Path::new(&cargo_home).join("bin");
        if exe.starts_with(&cargo_bin) {
            return InstallMethod::Cargo;
        }
    }
    if !home.as_os_str().is_empty() && exe.starts_with(home.join(".cargo").join("bin")) {
        return InstallMethod::Cargo;
    }

    // 3. cargo-dist default install location (`~/.local/bin`). The shell
    //    installer is the `curl | sh` front door, so we report it as `Curl`.
    if !home.as_os_str().is_empty() && exe.starts_with(home.join(".local").join("bin")) {
        return InstallMethod::Curl;
    }

    // 4. No marker matched ⇒ Unknown (never guess).
    InstallMethod::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn home() -> PathBuf {
        PathBuf::from("/home/user")
    }

    #[test]
    fn cellar_segment_is_brew() {
        // Apple-silicon canonical Cellar path.
        let exe = PathBuf::from("/opt/homebrew/Cellar/tome/0.6.0/bin/tome");
        assert_eq!(classify_install_path(&exe, &home()), InstallMethod::Brew);
    }

    #[test]
    fn intel_cellar_is_brew() {
        let exe = PathBuf::from("/usr/local/Cellar/tome/0.6.0/bin/tome");
        assert_eq!(classify_install_path(&exe, &home()), InstallMethod::Brew);
    }

    #[test]
    fn linuxbrew_prefix_is_brew() {
        // The opt-tree front-door (before canonicalization resolves the Cellar).
        let exe = PathBuf::from("/home/linuxbrew/.linuxbrew/bin/tome");
        assert_eq!(classify_install_path(&exe, &home()), InstallMethod::Brew);
    }

    #[test]
    fn homebrew_opt_prefix_is_brew() {
        let exe = PathBuf::from("/opt/homebrew/bin/tome");
        assert_eq!(classify_install_path(&exe, &home()), InstallMethod::Brew);
    }

    #[test]
    fn cargo_bin_under_home_is_cargo() {
        let exe = home().join(".cargo").join("bin").join("tome");
        assert_eq!(classify_install_path(&exe, &home()), InstallMethod::Cargo);
    }

    #[test]
    fn local_bin_under_home_is_curl() {
        let exe = home().join(".local").join("bin").join("tome");
        assert_eq!(classify_install_path(&exe, &home()), InstallMethod::Curl);
    }

    #[test]
    fn unrelated_path_is_unknown() {
        let exe = PathBuf::from("/usr/bin/tome");
        assert_eq!(classify_install_path(&exe, &home()), InstallMethod::Unknown);
    }

    #[test]
    fn cellar_precedes_cargo_when_both_present() {
        // A pathological path that contains BOTH a Cellar segment and a
        // `.cargo/bin` prefix: Homebrew must win (rule 1 before rule 2).
        let exe = home()
            .join(".cargo")
            .join("bin")
            .join("Cellar")
            .join("tome");
        assert_eq!(classify_install_path(&exe, &home()), InstallMethod::Brew);
    }

    #[test]
    fn empty_home_skips_home_relative_rules() {
        // With no home, the `~/.cargo/bin` and `~/.local/bin` checks can't fire;
        // a plain path falls through to Unknown rather than panicking on a join.
        let exe = PathBuf::from("/srv/app/tome");
        assert_eq!(
            classify_install_path(&exe, Path::new("")),
            InstallMethod::Unknown
        );
    }

    #[test]
    fn detect_install_method_never_panics() {
        // Smoke: the real-binary path classifies to *some* closed variant
        // (whatever it is for the test runner location) without panicking.
        let m = detect_install_method();
        assert!(matches!(
            m,
            InstallMethod::Brew
                | InstallMethod::Cargo
                | InstallMethod::Curl
                | InstallMethod::Unknown
        ));
    }
}
