//! Synthetic fixture homes. Nothing here comes from a real machine.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use tempfile::TempDir;

/// Planted secrets: must never appear in any output.
pub const SECRETS: &[&str] = &[
    "sk-PLANTED0secret0value0123",
    "ghp_PLANTED0token0value0456",
    "PLANTED0header0value0789xyz",
    "PLANTED0query0value0abc0123",
];

pub struct Fixture {
    _tmp: TempDir,
    pub home: PathBuf,
    pub source: PathBuf,
}

pub fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[cfg(unix)]
pub fn symlink(target: &Path, link: &Path) {
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(target, link).unwrap();
}

impl Fixture {
    pub fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        // canonical, so paths match what onesrc resolves (macOS: /var -> /private/var)
        let root = tmp.path().canonicalize().unwrap();
        let home = root.join("home");
        let source = root.join("source");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&source).unwrap();
        Self {
            _tmp: tmp,
            home,
            source,
        }
    }

    /// `onesrc` with a clean environment pointing at this fixture.
    pub fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::cargo_bin("onesrc").unwrap();
        c.env_clear()
            .env("HOME", &self.home)
            .env("ONESRC_SOURCE", &self.source)
            .env("PATH", "")
            .args(args);
        c
    }

    /// Run and return (stdout, stderr, exit code), with every planted
    /// secret asserted absent.
    pub fn run(&self, args: &[&str]) -> (String, String, i32) {
        let out = self.cmd(args).output().unwrap();
        let stdout = String::from_utf8(out.stdout).unwrap();
        let stderr = String::from_utf8(out.stderr).unwrap();
        for s in SECRETS {
            assert!(
                !stdout.contains(s),
                "secret {s} leaked to stdout of {args:?}:\n{stdout}"
            );
            assert!(
                !stderr.contains(s),
                "secret {s} leaked to stderr of {args:?}:\n{stderr}"
            );
        }
        (stdout, stderr, out.status.code().unwrap_or(-1))
    }
}
