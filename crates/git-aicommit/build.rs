//! Build script: bake the commit hash, build date, and build profile into the
//! binary so `git aicommit --version` can report them.
//!
//! Every value degrades to `"unknown"` rather than failing the build, because
//! the source isn't always present: a `cargo install` from crates.io has no
//! `.git`, and the cross container that builds the Linux release artifacts has
//! no `git` at all. To give those Linux binaries a real hash and date anyway,
//! the release workflow exports `GIT_AICOMMIT_COMMIT`/`GIT_AICOMMIT_BUILD_DATE`
//! (where `git` *is* available) and forwards them into the container via
//! `Cross.toml`; the env override below takes precedence over the local probes.

use std::path::Path;
use std::process::Command;

fn main() {
    emit_commit_hash();
    emit_build_date();
    emit_build_profile();
}

/// Short commit hash: a CI-provided `GIT_AICOMMIT_COMMIT`, else `git`, else
/// `"unknown"`. Watching `HEAD` keeps incremental local builds fresh after a
/// commit or checkout.
fn emit_commit_hash() {
    println!("cargo:rerun-if-env-changed=GIT_AICOMMIT_COMMIT");
    watch_git_head();

    let hash = env_override("GIT_AICOMMIT_COMMIT")
        .or_else(|| run("git", &["rev-parse", "--short", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_AICOMMIT_COMMIT_HASH={hash}");
}

/// Build timestamp (UTC): a CI-provided `GIT_AICOMMIT_BUILD_DATE`, else `date`,
/// else `"unknown"`.
fn emit_build_date() {
    println!("cargo:rerun-if-env-changed=GIT_AICOMMIT_BUILD_DATE");
    let date = env_override("GIT_AICOMMIT_BUILD_DATE")
        .or_else(|| run("date", &["-u", "+%Y-%m-%d %H:%M:%S UTC"]))
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_AICOMMIT_BUILD_DATE={date}");
}

/// Cargo's build profile (`"debug"`/`"release"`); always set during a build.
fn emit_build_profile() {
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=GIT_AICOMMIT_BUILD_PROFILE={profile}");
}

/// A trimmed, non-empty environment variable, or `None` if unset or blank.
fn env_override(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Run a command, returning its trimmed stdout on success and `None` otherwise
/// (binary missing, non-zero exit, or empty output).
fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Re-run when the checked-out commit changes, so an incremental build after a
/// commit/checkout re-reads the hash. Best effort: skipped for linked worktrees
/// (`.git` is a file) and packaged sources (no `.git`), where the env override
/// or `git` probe still supplies the value.
fn watch_git_head() {
    let head = Path::new(".git/HEAD");
    if !head.is_file() {
        return;
    }
    println!("cargo:rerun-if-changed=.git/HEAD");
    if let Ok(content) = std::fs::read_to_string(head)
        && let Some(reference) = content.strip_prefix("ref: ")
    {
        let ref_path = format!(".git/{}", reference.trim());
        if Path::new(&ref_path).is_file() {
            println!("cargo:rerun-if-changed={ref_path}");
        }
    }
}
