//! The command-line surface: the parsed argument struct and the help text.

use clap::Parser;

#[derive(Parser)]
#[command(
    about = "Generate git commit messages from staged diffs using Claude",
    // Let `-h`/`--help` fall through to git_args so we can print our own help,
    // which documents the git-commit flags we intercept (see `HELP`).
    disable_help_flag = true
)]
pub(crate) struct Args {
    /// Claude model to use (passed directly to `claude --model`).
    /// Must come before any git flags.
    #[arg(long, default_value = "haiku")]
    pub(crate) model: String,

    /// Standard `git commit` flags. Recognized flags steer how the message is
    /// generated; everything else is forwarded verbatim to `git commit`.
    /// Run `git aicommit --help` for the full list.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub(crate) git_args: Vec<String>,
}

/// The multi-line `--version` report: the semver line, the build metadata baked
/// in by `build.rs`, and the path of the running binary (resolved at runtime).
pub(crate) fn version() -> String {
    let binary = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    format!(
        "\
git-aicommit {version}
  commit:   {commit}
  built:    {built}
  profile:  {profile}
  binary:   {binary}
",
        version = env!("CARGO_PKG_VERSION"),
        commit = env!("GIT_AICOMMIT_COMMIT_HASH"),
        built = env!("GIT_AICOMMIT_BUILD_DATE"),
        profile = env!("GIT_AICOMMIT_BUILD_PROFILE"),
    )
}

pub(crate) const HELP: &str = "\
git-aicommit — draft a commit message from your changes with Claude, then open `git commit`.

USAGE:
    git aicommit [--model <name>] [git commit flags] [-- <pathspec>...]

HANDLED BY git-aicommit:
    --model <name>      Claude model to use (default: haiku). Must come first.
    -V, --version       Print version, build metadata, and binary path, then exit.
    -m, --message <s>   Steer the AI with an instruction (NOT a literal message). Repeatable.
    -t, --template <f>  Make the AI follow the format/structure in file <f>.
    -a, --all           Include all tracked changes (diff vs HEAD); also forwarded to git.
    -p, --patch         Interactively stage hunks first, then summarize what you staged.
        --interactive   Interactively stage via `git add -i` first.
        --amend         Regenerate the message for an amended commit (prev message + combined diff).
        --dry-run       Print the diff and generated message, then exit without committing.
        --push          Run `git push` after the commit succeeds.
    <pathspec>...       Limit the commit (and the AI's context) to these paths.

FORWARDED to `git commit` verbatim:
    -e/--edit (on by default), -n/--no-verify, -s/--signoff, -S/--gpg-sign,
    -v/--verbose (note: -V is our version flag), --author, --date,
    --allow-empty, and any other flags you pass.

BYPASS (no AI; runs plain `git commit`):
    --fixup, --squash, -C/--reuse-message, -c/--reedit-message, -F/--file
    (the message comes from another commit or file, so there is nothing to generate).

Authentication is delegated entirely to the `claude` CLI.
";
