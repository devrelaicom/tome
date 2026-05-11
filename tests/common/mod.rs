//! Shared test harness for the catalog command integration suites. Each test
//! builds a fresh fixture catalog inside a `tempfile::TempDir`, runs
//! `git init && git add -A && git commit -m init` against it (so it has a
//! HEAD), and constructs `Command` invocations of the `tome` binary with
//! isolated `HOME`/`XDG_*` so the host's real config is never touched.
//!
//! All paths are absolute. No mocking of git or the filesystem.

#![allow(dead_code)] // each test file uses a subset of these helpers

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

/// Build a self-contained Git fixture catalog from the on-disk
/// `tests/fixtures/sample-catalog/` skeleton. Returns the temp dir handle
/// (must stay alive for the lifetime of the test) and a `file://` URL the
/// `tome` binary can clone from.
pub struct Fixture {
    pub tempdir: TempDir,
    pub repo_path: PathBuf,
    pub url: String,
}

impl Fixture {
    pub fn build_sample() -> Self {
        Self::build_from(fixture_path("sample-catalog"))
    }

    pub fn build_from(skeleton: PathBuf) -> Self {
        let tempdir = TempDir::new().expect("tempdir");
        let repo_path = tempdir.path().join("catalog");
        copy_dir(&skeleton, &repo_path).expect("copy skeleton");
        // We need real plugin directories — git won't track empty ones, and
        // `.keep` files are inside them so they materialise as soon as
        // they're copied.
        git_init_and_commit(&repo_path);
        let url = format!("file://{}", repo_path.display());
        Self {
            tempdir,
            repo_path,
            url,
        }
    }
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let target = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn git_init_and_commit(repo: &Path) {
    let run = |args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(repo)
            // Suppress identity prompts in CI.
            .env("GIT_AUTHOR_NAME", "Tome Test")
            .env("GIT_AUTHOR_EMAIL", "tests@tome.invalid")
            .env("GIT_COMMITTER_NAME", "Tome Test")
            .env("GIT_COMMITTER_EMAIL", "tests@tome.invalid")
            .status()
            .unwrap_or_else(|e| panic!("git {:?}: {}", args, e));
        assert!(status.success(), "git {:?} exited {}", args, status);
    };
    run(&["init", "-q", "-b", "main"]);
    run(&["add", "-A"]);
    run(&["commit", "-q", "-m", "init"]);
}

/// Isolated environment for invoking the `tome` binary. Each test gets a
/// fresh XDG layout so the host config is never touched.
pub struct ToolEnv {
    pub home: TempDir,
}

impl ToolEnv {
    pub fn new() -> Self {
        Self {
            home: TempDir::new().expect("tool env home"),
        }
    }

    pub fn home_path(&self) -> &Path {
        self.home.path()
    }

    pub fn config_dir(&self) -> PathBuf {
        // `directories` on macOS would route through Application Support if
        // `qualifier` were non-empty; here we use the XDG vars so the layout
        // is the same on both supported platforms.
        self.home.path().join(".config/tome")
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir().join("config.toml")
    }

    pub fn data_dir(&self) -> PathBuf {
        self.home.path().join(".local/share/tome")
    }

    pub fn catalogs_dir(&self) -> PathBuf {
        self.data_dir().join("catalogs")
    }

    /// Build a `Command` for the compiled `tome` binary, pre-populated with
    /// the isolated env.
    pub fn cmd(&self) -> Command {
        let mut cmd = Command::new(tome_bin());
        cmd.env("HOME", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join(".config"))
            .env("XDG_DATA_HOME", self.home.path().join(".local/share"))
            // `directories` honours these on macOS too when set.
            .env_remove("TOME_LOG")
            .env_remove("RUST_LOG");
        cmd
    }
}

fn tome_bin() -> PathBuf {
    // Cargo points `CARGO_BIN_EXE_<name>` at the freshly-built binary for
    // the package; integration tests get this for free.
    PathBuf::from(env!("CARGO_BIN_EXE_tome"))
}
