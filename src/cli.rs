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

pub(crate) const HELP: &str = "\
git-aicommit — draft a commit message from your changes with Claude, then open `git commit`.

USAGE:
    git aicommit [--model <name>] [git commit flags] [-- <pathspec>...]

HANDLED BY git-aicommit:
    --model <name>      Claude model to use (default: haiku). Must come first.
    -m, --message <s>   Steer the AI with an instruction (NOT a literal message). Repeatable.
    -t, --template <f>  Make the AI follow the format/structure in file <f>.
    -a, --all           Include all tracked changes (diff vs HEAD); also forwarded to git.
    -p, --patch         Interactively stage hunks first, then summarize what you staged.
        --interactive   Interactively stage via `git add -i` first.
        --amend         Regenerate the message for an amended commit (prev message + combined diff).
        --dry-run       Print the diff and generated message, then exit without committing.
    <pathspec>...       Limit the commit (and the AI's context) to these paths.

FORWARDED to `git commit` verbatim:
    -e/--edit (on by default), -n/--no-verify, -s/--signoff, -S/--gpg-sign,
    --author, --date, --allow-empty, and any other flags you pass.

BYPASS (no AI; runs plain `git commit`):
    --fixup, --squash, -C/--reuse-message, -c/--reedit-message, -F/--file
    (the message comes from another commit or file, so there is nothing to generate).

Authentication is delegated entirely to the `claude` CLI.
";
