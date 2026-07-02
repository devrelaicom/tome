//! `tome completions <shell>` integration tests (issue #322).
//!
//! Drives the compiled `tome` binary. The command is intercepted pre-dispatch
//! in `main.rs` (before `Paths::resolve()`), so it must generate a valid
//! completion script with NO valid HOME / index / config — that property is the
//! whole point of the feature (a user runs it during shell setup). These are
//! cheap spawns: no models, no DB, no network.

mod common;

use common::ToolEnv;

/// Spawn `tome completions <shell>` under the isolated env, returning
/// `(exit_code, stdout, stderr)`.
fn completions(env: &ToolEnv, shell: &str) -> (Option<i32>, String, String) {
    let out = env
        .cmd()
        .args(["completions", shell])
        .output()
        .expect("spawn tome completions");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// zsh script carries a `#compdef tome` header and a `_tome` completion fn.
#[test]
fn completions_zsh_emits_zsh_script() {
    let env = ToolEnv::new();
    let (code, stdout, stderr) = completions(&env, "zsh");
    assert_eq!(code, Some(0), "expected exit 0; stderr={stderr}");
    assert!(
        !stdout.is_empty(),
        "zsh completion script must be non-empty"
    );
    assert!(
        stdout.contains("#compdef tome"),
        "zsh script must contain the `#compdef tome` marker; got:\n{stdout}"
    );
    assert!(
        stdout.contains("_tome"),
        "zsh script must reference the `_tome` completion function"
    );
}

/// bash script registers completion via `complete -F _tome`.
#[test]
fn completions_bash_emits_bash_script() {
    let env = ToolEnv::new();
    let (code, stdout, stderr) = completions(&env, "bash");
    assert_eq!(code, Some(0), "expected exit 0; stderr={stderr}");
    assert!(
        !stdout.is_empty(),
        "bash completion script must be non-empty"
    );
    assert!(
        stdout.contains("_tome"),
        "bash script must reference the `_tome` completion function"
    );
    assert!(
        stdout.contains("complete -F"),
        "bash script must register completion with `complete -F`; got:\n{stdout}"
    );
}

/// An unknown shell is a parse-time usage error (exit 2) that lists the valid
/// values. This exercises the `clap_complete::Shell` ValueEnum surface.
#[test]
fn completions_unknown_shell_is_usage_error() {
    let env = ToolEnv::new();
    let (code, stdout, stderr) = completions(&env, "bogus");
    assert_eq!(
        code,
        Some(2),
        "unknown shell must be a usage error (exit 2)"
    );
    assert!(
        stdout.is_empty(),
        "no completion script on a usage error; got stdout:\n{stdout}"
    );
    // clap lists the valid values; assert every supported shell appears so the
    // message is actually the "possible values" list and not some other error.
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        assert!(
            stderr.contains(shell),
            "usage error must list valid shell `{shell}`; got:\n{stderr}"
        );
    }
}

/// Every supported shell parses and produces a non-empty script at exit 0.
#[test]
fn completions_all_shells_parse() {
    let env = ToolEnv::new();
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let (code, stdout, stderr) = completions(&env, shell);
        assert_eq!(code, Some(0), "`{shell}` should exit 0; stderr={stderr}");
        assert!(
            !stdout.is_empty(),
            "`{shell}` completion script must be non-empty"
        );
    }
}

/// The load-bearing property: completions work with NO valid HOME and a
/// deliberately BROKEN config. The command is intercepted BEFORE `Paths::resolve`
/// and scope resolution, so neither a missing HOME nor a malformed
/// `~/.tome/config.toml` (which would brick every other command with exit 5) can
/// stop it. A user runs this during shell setup, before Tome is configured.
#[test]
fn completions_work_without_home_or_config() {
    // Case 1: empty HOME pointing at a fresh temp dir (no `.tome/` at all).
    let env = ToolEnv::new();
    let (code, stdout, _stderr) = completions(&env, "zsh");
    assert_eq!(code, Some(0), "unconfigured HOME must still succeed");
    assert!(stdout.contains("#compdef tome"), "must emit the zsh script");

    // Case 2: a HOME whose `~/.tome/config.toml` is malformed. Every OTHER
    // command fails loudly at the pre-dispatch strict config gate (exit 5);
    // completions is intercepted before that gate and must still succeed.
    let broken = ToolEnv::new();
    let root = broken.tome_root();
    std::fs::create_dir_all(&root).expect("mk .tome");
    std::fs::write(root.join("config.toml"), "this is not = valid toml [[[")
        .expect("write broken config");
    let (code, stdout, stderr) = completions(&broken, "bash");
    assert_eq!(
        code,
        Some(0),
        "a malformed config must NOT block completions; stderr={stderr}"
    );
    assert!(
        stdout.contains("complete -F"),
        "must emit the bash script even with a broken config"
    );

    // Case 3: HOME unset entirely (the env var removed from the child).
    let mut cmd = ToolEnv::new().cmd();
    cmd.env_remove("HOME");
    cmd.args(["completions", "fish"]);
    let out = cmd.output().expect("spawn with HOME unset");
    assert_eq!(
        out.status.code(),
        Some(0),
        "HOME unset must still succeed; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !out.stdout.is_empty(),
        "fish completion script must be non-empty with HOME unset"
    );
}
