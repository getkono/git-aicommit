//! Every git subprocess the tool runs lives here, so the full set of commands
//! issued against the user's repository is auditable in one place. Pure helpers
//! (`build_diff_args`, `build_commit_args`, `resolve_base`) assemble argument
//! vectors; the `Result`-returning functions actually invoke git.

use std::process::{Command, Stdio};

use crate::error::{Error, Result};
use crate::flags::{Interactive, ParsedArgs};

/// The well-known SHA of git's empty tree, used as a diff base when there is no
/// suitable commit to compare against (root commit or amending a root commit).
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// Run a git command with all output discarded and report whether it succeeded.
fn git_ok(args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Ensure the current directory is inside a git repository.
pub(crate) fn ensure_in_repo() -> Result<()> {
    let status = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| Error::Git(format!("failed to run git: {e}")))?;
    if !status.success() {
        return Err(Error::NotARepo);
    }
    Ok(())
}

/// Hand the terminal to `git add --patch`/`--interactive` so the user can stage,
/// then return. A no-op when no interactive mode was requested.
pub(crate) fn interactive_stage(p: &ParsedArgs) -> Result<()> {
    let Some(kind) = p.interactive else {
        return Ok(());
    };
    let flag = match kind {
        Interactive::Patch => "--patch",
        Interactive::Interactive => "--interactive",
    };
    eprintln!("staging changes interactively (git add {flag})…");
    let mut cmd = Command::new("git");
    cmd.arg("add").arg(flag);
    if !p.pathspecs.is_empty() {
        cmd.arg("--");
        cmd.args(&p.pathspecs);
    }
    let st = cmd
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| Error::Git(format!("failed to run `git add {flag}`: {e}")))?;
    if !st.success() {
        return Err(Error::Git(format!(
            "interactive staging aborted (`git add {flag}` exited with {st})"
        )));
    }
    Ok(())
}

/// The commit/tree the diff is taken against. For amend it's HEAD's parent
/// (or the empty tree for a root commit); otherwise HEAD (or the empty tree in a
/// repo with no commits yet).
pub(crate) fn resolve_base(p: &ParsedArgs) -> Result<String> {
    let has_head = git_ok(&["rev-parse", "--verify", "--quiet", "HEAD"]);
    if p.amend {
        if !has_head {
            return Err(Error::Git("nothing to amend: no commits yet".to_string()));
        }
        if git_ok(&["rev-parse", "--verify", "--quiet", "HEAD^"]) {
            Ok("HEAD^".to_string())
        } else {
            Ok(EMPTY_TREE.to_string())
        }
    } else if has_head {
        Ok("HEAD".to_string())
    } else {
        Ok(EMPTY_TREE.to_string())
    }
}

/// Build the `git diff …` argument vector for the AI's context, matching what the
/// final commit will record.
pub(crate) fn build_diff_args(p: &ParsedArgs, base: &str) -> Vec<String> {
    // Working-tree diff for `-a` and pathspec(`--only`) modes; index diff otherwise
    // (including interactive, where staging just happened).
    let working_tree = p.interactive.is_none() && (p.all || !p.pathspecs.is_empty());
    let mut args = vec!["diff".to_string()];
    if !working_tree {
        args.push("--cached".to_string());
    }
    args.push("--no-color".to_string());
    args.push(base.to_string());
    if p.scoped() {
        args.push("--".to_string());
        args.extend(p.pathspecs.iter().cloned());
    }
    args
}

/// The diff text plus the number of files it touches.
pub(crate) struct Diff {
    pub(crate) text: String,
    pub(crate) file_count: usize,
}

/// Run the prepared `git diff` and return its output. Errors if git fails or if
/// there are no changes to commit (and we're not amending / allowing empty).
pub(crate) fn read_diff(p: &ParsedArgs, diff_args: &[String]) -> Result<Diff> {
    let diff_out = Command::new("git")
        .args(diff_args)
        .output()
        .map_err(|e| Error::Git(format!("failed to run `git {}`: {e}", diff_args.join(" "))))?;
    if !diff_out.status.success() {
        return Err(Error::Git(format!(
            "`git {}` failed: {}",
            diff_args.join(" "),
            String::from_utf8_lossy(&diff_out.stderr)
        )));
    }
    let text = String::from_utf8_lossy(&diff_out.stdout).to_string();
    if text.trim().is_empty() && !p.amend && !p.allow_empty {
        let msg = if p.scoped() {
            "no changes in the given path(s)"
        } else if p.all {
            "no changes to commit (working tree matches HEAD)"
        } else {
            "no staged changes (did you forget `git add`?)"
        };
        return Err(Error::NoChanges(msg.to_string()));
    }
    let file_count = text.lines().filter(|l| l.starts_with("diff --git")).count();
    Ok(Diff { text, file_count })
}

/// `git log -1 --pretty=%B`, trimmed.
pub(crate) fn previous_commit_message() -> Result<String> {
    let out = Command::new("git")
        .args(["log", "-1", "--pretty=%B"])
        .output()
        .map_err(|e| Error::Git(format!("failed to read previous commit message: {e}")))?;
    if !out.status.success() {
        return Err(Error::Git(format!(
            "failed to read previous commit message: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Run the pre-commit hook for an early check (before spending Claude tokens).
pub(crate) fn run_pre_commit_hook() -> Result<()> {
    eprintln!("running pre-commit hooks…");
    let hook_status = Command::new("git")
        .args(["hook", "run", "--ignore-missing", "pre-commit"])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| Error::Git(format!("failed to run pre-commit hooks: {e}")))?;
    if !hook_status.success() {
        return Err(Error::Git(format!(
            "pre-commit hooks failed (exit {hook_status}); fix the issues and try again"
        )));
    }
    eprintln!("pre-commit hooks passed");
    Ok(())
}

/// Assemble the final `git commit …` argument vector.
pub(crate) fn build_commit_args(message: &str, p: &ParsedArgs, add_no_verify: bool) -> Vec<String> {
    let mut args = vec!["commit".to_string()];
    if !p.no_edit {
        args.push("-e".to_string());
    }
    args.push("-m".to_string());
    args.push(message.to_string());
    args.extend(p.passthrough.iter().cloned());
    // Hooks already ran in the early check — skip them at commit time. Guard
    // against double `--no-verify` if it somehow slipped into passthrough.
    if add_no_verify
        && !p
            .passthrough
            .iter()
            .any(|a| a == "--no-verify" || a == "-n")
    {
        args.push("--no-verify".to_string());
    }
    if p.scoped() {
        args.push("--".to_string());
        args.extend(p.pathspecs.iter().cloned());
    }
    args
}

/// Run the final `git commit` (with the prepared args), inheriting stdio so the
/// editor can open.
pub(crate) fn final_commit(commit_args: &[String]) -> Result<()> {
    let status = Command::new("git")
        .args(commit_args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| Error::Git(format!("failed to run `git commit`: {e}")))?;
    if !status.success() {
        return Err(Error::Git(format!("`git commit` exited with {status}")));
    }
    Ok(())
}

/// Run `git commit` with the user's raw args, inheriting stdio. Used for bypass
/// modes (`--fixup`, `-C`, …) where there's nothing for the AI to generate.
pub(crate) fn run_git_commit_passthrough(git_args: &[String]) -> Result<()> {
    let status = Command::new("git")
        .arg("commit")
        .args(git_args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| Error::Git(format!("failed to run `git commit`: {e}")))?;
    if !status.success() {
        return Err(Error::Git(format!("`git commit` exited with {status}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vec<String> from &str literals, for ergonomic assert_eq! comparisons.
    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn commit_args_basic() {
        let p = ParsedArgs::default();
        assert_eq!(
            build_commit_args("hi", &p, false),
            v(&["commit", "-e", "-m", "hi"])
        );
        assert_eq!(
            build_commit_args("hi", &p, true),
            v(&["commit", "-e", "-m", "hi", "--no-verify"])
        );
    }

    #[test]
    fn commit_args_no_edit() {
        let p = ParsedArgs {
            no_edit: true,
            passthrough: v(&["--no-edit"]),
            ..Default::default()
        };
        let a = build_commit_args("hi", &p, false);
        assert_eq!(a, v(&["commit", "-m", "hi", "--no-edit"]));
        assert!(!a.contains(&"-e".to_string()));
    }

    #[test]
    fn commit_args_passthrough_and_paths() {
        let p = ParsedArgs {
            all: true,
            passthrough: v(&["-a", "-s"]),
            pathspecs: v(&["f.rs"]),
            ..Default::default()
        };
        assert_eq!(
            build_commit_args("hi", &p, false),
            v(&["commit", "-e", "-m", "hi", "-a", "-s", "--", "f.rs"])
        );
    }

    #[test]
    fn commit_args_interactive_omits_paths() {
        let p = ParsedArgs {
            interactive: Some(Interactive::Patch),
            pathspecs: v(&["f.rs"]),
            ..Default::default()
        };
        // Interactive staging already happened; commit the index, not `--only` paths.
        assert_eq!(
            build_commit_args("hi", &p, true),
            v(&["commit", "-e", "-m", "hi", "--no-verify"])
        );
    }

    #[test]
    fn commit_args_no_double_no_verify() {
        let p = ParsedArgs {
            no_verify: true,
            passthrough: v(&["--no-verify"]),
            ..Default::default()
        };
        let a = build_commit_args("hi", &p, false);
        assert_eq!(a.iter().filter(|x| *x == "--no-verify").count(), 1);
    }

    #[test]
    fn diff_args_modes() {
        let plain = ParsedArgs::default();
        assert_eq!(
            build_diff_args(&plain, "HEAD"),
            v(&["diff", "--cached", "--no-color", "HEAD"])
        );

        let amend = ParsedArgs {
            amend: true,
            ..Default::default()
        };
        assert_eq!(
            build_diff_args(&amend, "HEAD^"),
            v(&["diff", "--cached", "--no-color", "HEAD^"])
        );

        let all = ParsedArgs {
            all: true,
            ..Default::default()
        };
        assert_eq!(
            build_diff_args(&all, "HEAD"),
            v(&["diff", "--no-color", "HEAD"])
        );

        let path = ParsedArgs {
            pathspecs: v(&["f"]),
            ..Default::default()
        };
        assert_eq!(
            build_diff_args(&path, "HEAD"),
            v(&["diff", "--no-color", "HEAD", "--", "f"])
        );

        let all_amend = ParsedArgs {
            all: true,
            amend: true,
            ..Default::default()
        };
        assert_eq!(
            build_diff_args(&all_amend, "HEAD^"),
            v(&["diff", "--no-color", "HEAD^"])
        );

        let inter = ParsedArgs {
            interactive: Some(Interactive::Patch),
            pathspecs: v(&["f"]),
            ..Default::default()
        };
        // Interactive → index diff, no path scope.
        assert_eq!(
            build_diff_args(&inter, "HEAD"),
            v(&["diff", "--cached", "--no-color", "HEAD"])
        );
    }
}
