//! `tome catalog add` integration tests. Each test builds a local file://
//! fixture catalog, invokes the binary against an isolated XDG layout, and
//! asserts on exit code, stdout shape, registry state, and cache layout.

use crate::common::{Fixture, ToolEnv, global_enrolment_url, has_global_enrolment, paths_for};
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
    // #281: onboarding `next:` hint points at the next command in the flow.
    assert!(
        stdout.contains("next:") && stdout.contains("tome plugin list"),
        "expected onboarding `next:` hint in human stdout: {}",
        stdout
    );

    let paths = paths_for(&env);
    assert!(
        has_global_enrolment(&paths, "sample-experts"),
        "expected sample-experts in workspace_catalogs for global",
    );
    assert_eq!(
        global_enrolment_url(&paths, "sample-experts").as_deref(),
        Some(fix.url.as_str()),
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
    // #281: the onboarding `next:` hint is human-mode only — JSON stdout must
    // stay byte-stable (parses cleanly above and carries no `next:` marker).
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("next:"),
        "`next:` hint must not appear in --json stdout: {}",
        stdout
    );
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
    let paths = paths_for(&env);
    assert!(has_global_enrolment(&paths, "renamed"));
    assert!(!has_global_enrolment(&paths, "sample-experts"));
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
    let paths = paths_for(&env);
    let url = global_enrolment_url(&paths, "sample-experts")
        .expect("expected sample-experts in workspace_catalogs");
    assert!(
        !url.contains("supersecret") && !url.contains("alice:"),
        "credentials leaked into workspace_catalogs.url: {url}",
    );
}

// Removed in F11c-1: `config_toml_is_chmod_0600_on_unix`.
// Phase 4 / F11b dropped `config.toml` as the registry for catalog
// enrolment — the central DB's `workspace_catalogs` table owns that
// state now. File-permission tightening is SQLite-managed for
// `index.db` (the file mode is set by SQLite at create time) and
// outside Tome's direct control, so the original chmod-0600 invariant
// no longer has a meaningful test target.

#[test]
fn add_echoes_resolved_commit_sha() {
    // #329 Part A: `catalog add` resolves the HEAD commit of the cache dir and
    // surfaces it. Human output carries a short (7-char) sha in the
    // parenthetical; `--json` carries the full 40-hex sha under `added.commit`.
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();

    // Human mode.
    let out = env
        .cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .expect("spawn");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("commit: "),
        "expected a `commit: ` fragment in human stdout: {}",
        stdout
    );
    // The short sha is a 7-char lowercase hex token immediately after
    // `commit: `. Extract and validate it.
    let short = stdout
        .split("commit: ")
        .nth(1)
        .and_then(|tail| tail.split([',', ')', ' ']).next())
        .expect("a token after `commit: `");
    assert_eq!(short.len(), 7, "short sha must be 7 chars, got {:?}", short);
    assert!(
        short.chars().all(|c| c.is_ascii_hexdigit()),
        "short sha must be hex, got {:?}",
        short
    );

    // JSON mode (fresh env so the add isn't a duplicate).
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
    let commit = v["added"]["commit"]
        .as_str()
        .expect("`added.commit` must be a string");
    assert_eq!(
        commit.len(),
        40,
        "the JSON `commit` must be the full 40-hex sha, got {:?}",
        commit
    );
    assert!(
        commit.chars().all(|c| c.is_ascii_hexdigit()),
        "the JSON `commit` must be hex, got {:?}",
        commit
    );
    // The human short sha must be the 7-char prefix of the full JSON sha.
    assert_eq!(
        &commit[..7],
        short,
        "human short sha must be the prefix of the full JSON sha",
    );
}

#[test]
fn ref_aliases_and_short_name_parse() {
    // #329 Part A: `--branch`/`--tag` are visible aliases of `--ref`, and `-n`
    // is a short for `--name`. Assert at the clap parse level.
    use clap::Parser;
    use tome::cli::{CatalogCommand, Cli, Command};

    let extract = |argv: &[&str]| -> (Option<String>, Option<String>) {
        let cli = Cli::try_parse_from(argv).expect("parse");
        let Command::Catalog(CatalogCommand::Add(add)) = cli.command else {
            panic!("expected `catalog add`");
        };
        (add.ref_, add.name)
    };

    let (ref_, _) = extract(&["tome", "catalog", "add", "owner/repo", "--branch", "dev"]);
    assert_eq!(ref_.as_deref(), Some("dev"), "`--branch` sets ref_");

    let (ref_, _) = extract(&["tome", "catalog", "add", "owner/repo", "--tag", "v1.2.0"]);
    assert_eq!(ref_.as_deref(), Some("v1.2.0"), "`--tag` sets ref_");

    let (_, name) = extract(&["tome", "catalog", "add", "owner/repo", "-n", "alias"]);
    assert_eq!(name.as_deref(), Some("alias"), "`-n` sets name");
}

#[test]
fn update_force_flag_is_rejected() {
    // #329 Part C: the inert `catalog update --force` flag was removed; parsing
    // it is now a clap usage error.
    use clap::Parser;
    use tome::cli::Cli;

    let res = Cli::try_parse_from(["tome", "catalog", "update", "--force"]);
    assert!(
        res.is_err(),
        "`catalog update --force` must be a usage error now that the flag is gone",
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
