//! `tome catalog add` integration tests. Each test builds a local file://
//! fixture catalog, invokes the binary against an isolated XDG layout, and
//! asserts on exit code, stdout shape, registry state, and cache layout.

mod common;

use common::{Fixture, ToolEnv};
use serde_json::Value;

#[test]
fn happy_path_human_mode() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();

    let out = env
        .cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .expect("spawn");

    assert!(
        out.status.success(),
        "exit={:?} stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Added catalog `sample-experts`"),
        "unexpected stdout: {}",
        stdout
    );
    assert!(stdout.contains("plugins: 2"), "stdout: {}", stdout);

    let config_text = std::fs::read_to_string(env.config_file()).expect("config written");
    assert!(
        config_text.contains("[catalogs.sample-experts]"),
        "{}",
        config_text
    );
    assert!(
        config_text.contains(&format!("url = \"{}\"", fix.url)),
        "{}",
        config_text
    );
}

#[test]
fn happy_path_json_mode() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();

    let out = env
        .cmd()
        .args(["catalog", "add", &fix.url, "--json"])
        .output()
        .expect("spawn");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let v: Value = serde_json::from_slice(&out.stdout).expect("json parse");
    assert_eq!(v["added"]["name"], "sample-experts");
    assert_eq!(v["added"]["plugin_count"], 2);
    assert_eq!(v["added"]["url"], fix.url);
    assert!(v["added"]["last_synced"].is_string());
}

#[test]
fn name_override_replaces_manifest_name() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();

    let out = env
        .cmd()
        .args(["catalog", "add", &fix.url, "--name", "renamed"])
        .output()
        .expect("spawn");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let config_text = std::fs::read_to_string(env.config_file()).unwrap();
    assert!(config_text.contains("[catalogs.renamed]"));
    assert!(!config_text.contains("[catalogs.sample-experts]"));
}

#[test]
fn duplicate_registration_exits_4() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();

    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .expect("first add");
    let out = env
        .cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .expect("second add");
    assert_eq!(
        out.status.code(),
        Some(4),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn duplicate_display_name_exits_4() {
    let fix1 = Fixture::build_sample();
    let fix2 = Fixture::build_sample();
    let env = ToolEnv::new();

    env.cmd()
        .args(["catalog", "add", &fix1.url, "--name", "same"])
        .output()
        .expect("first");
    let out = env
        .cmd()
        .args(["catalog", "add", &fix2.url, "--name", "same"])
        .output()
        .expect("second");
    assert_eq!(out.status.code(), Some(4));
}

#[test]
fn missing_manifest_exits_5() {
    // Build a fixture that's a git repo with no tome-catalog.toml.
    let tempdir = tempfile::TempDir::new().unwrap();
    let repo = tempdir.path().join("bad");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("README"), "no manifest here").unwrap();
    use std::process::Command;
    let g = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(&repo)
            .env("GIT_AUTHOR_NAME", "x")
            .env("GIT_AUTHOR_EMAIL", "x@x.invalid")
            .env("GIT_COMMITTER_NAME", "x")
            .env("GIT_COMMITTER_EMAIL", "x@x.invalid")
            .status()
            .unwrap()
    };
    g(&["init", "-q", "-b", "main"]);
    g(&["add", "-A"]);
    g(&["commit", "-q", "-m", "init"]);
    let url = format!("file://{}", repo.display());

    let env = ToolEnv::new();
    let out = env.cmd().args(["catalog", "add", &url]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(5),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn successful_add_persists_scrubbed_url_in_config() {
    // file:// URLs with embedded userinfo clone fine — git silently ignores
    // the userinfo for local transports. The scrub must still strip the
    // credentials before they land in config.toml or on stdout.
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    let url_with_creds = fix.url.replacen("file://", "file://alice:supersecret@", 1);

    let out = env
        .cmd()
        .args(["catalog", "add", &url_with_creds])
        .output()
        .expect("spawn");
    assert!(
        out.status.success(),
        "expected exit 0, got {:?}, stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("supersecret") && !stdout.contains("alice:"),
        "credentials leaked on stdout: {stdout}",
    );
    let config_text = std::fs::read_to_string(env.config_file()).expect("config written");
    assert!(
        !config_text.contains("supersecret") && !config_text.contains("alice:"),
        "credentials leaked into config.toml:\n{config_text}",
    );
    // The catalog itself is still registered.
    assert!(
        config_text.contains("[catalogs.sample-experts]"),
        "expected sample-experts to be registered, got:\n{config_text}",
    );
}

#[cfg(unix)]
#[test]
fn config_toml_is_chmod_0600_on_unix() {
    use std::os::unix::fs::PermissionsExt;
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    let out = env
        .cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let perms = std::fs::metadata(env.config_file()).unwrap().permissions();
    // Only the low 9 mode bits are meaningful here.
    assert_eq!(
        perms.mode() & 0o777,
        0o600,
        "config.toml should be 0600, got {:o}",
        perms.mode() & 0o777,
    );
}

#[test]
fn git_failure_with_credential_url_is_scrubbed() {
    let env = ToolEnv::new();
    // URL with embedded credentials pointing at nothing. Git will fail; we
    // assert that the bytes "supersecret" never appear in stderr.
    let bad_url = "https://alice:supersecret@127.0.0.1:1/nope.git";
    let out = env
        .cmd()
        .args(["catalog", "add", bad_url])
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(6));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("supersecret"),
        "credential leaked in stderr: {}",
        stderr
    );
    assert!(
        !stderr.contains("alice:"),
        "userinfo leaked in stderr: {}",
        stderr
    );
}
