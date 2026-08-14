//! The command-line surface: the parsed argument struct and the help text.

use clap::{Parser, ValueEnum};

/// The local agent CLI used to generate the commit message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum AgentChoice {
    Codex,
    Claude,
}

#[derive(Parser)]
#[command(
    about = "Generate git commit messages from staged diffs using a local AI agent",
    // Let `-h`/`--help` fall through to git_args so we can print our own help,
    // which documents the git-commit flags we intercept (see `HELP`).
    disable_help_flag = true
)]
pub(crate) struct Args {
    /// Local agent CLI to use. When omitted, prefer Codex, then Claude.
    #[arg(long, value_enum)]
    pub(crate) agent: Option<AgentChoice>,

    /// Model to use with the selected agent.
    /// Must come before any git flags. When omitted, the model is chosen
    /// automatically from the diff size.
    #[arg(long)]
    pub(crate) model: Option<String>,

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
git-aicommit — draft a commit message with Codex or Claude, then open `git commit`.

USAGE:
    git aicommit [--agent <codex|claude>] [--model <name>] [git commit flags] [-- <pathspec>...]

HANDLED BY git-aicommit:
    --agent <name>      Agent CLI to use: codex or claude. Must come first. When
                        omitted, uses codex if installed, then claude.
    --model <name>      Model for the selected agent. Must come first. When
                        omitted, chooses by diff size: Luna/Terra for Codex or
                        Haiku/Sonnet for Claude.
    -V, --version       Print version, build metadata, and binary path, then exit.
    -m, --message <s>   Steer the AI with an instruction (NOT a literal message). Repeatable.
    -t, --template <f>  Make the AI follow the format/structure in file <f>.
    -a, --all           Include all tracked changes (diff vs HEAD); also forwarded to git.
    -p, --patch         Interactively stage hunks first, then summarize what you staged.
        --interactive   Interactively stage via `git add -i` first.
        --amend         Regenerate the message for an amended commit (prev message + combined diff).
        --dry-run       Print everything that would be sent — git commands, model,
                        system prompt, every context item, and a per-file table of
                        what the summarizer kept and why — then the generated
                        message, and exit without committing.
        --no-compact    Send the diff verbatim instead of summarizing it to fit the
                        context budget. Large changes may then be truncated.
    -y, --yes           Commit the generated message directly, without opening the
                        editor; the message is printed after the commit succeeds.
        --push          Run `git push` after the commit succeeds.
    <pathspec>...       Limit the commit (and the AI's context) to these paths.

FORWARDED to `git commit` verbatim:
    -e/--edit (on by default), -n/--no-verify, -s/--signoff, -S/--gpg-sign,
    -v/--verbose (note: -V is our version flag), --author, --date,
    --allow-empty, and any other flags you pass.

BYPASS (no AI; runs plain `git commit`):
    --fixup, --squash, -C/--reuse-message, -c/--reedit-message, -F/--file
    (the message comes from another commit or file, so there is nothing to generate).

Authentication is delegated entirely to the selected `codex` or `claude` CLI.
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_agent_and_model_before_git_arguments() {
        let args = Args::try_parse_from([
            "git-aicommit",
            "--agent",
            "claude",
            "--model",
            "sonnet",
            "--amend",
        ])
        .unwrap();

        assert_eq!(args.agent, Some(AgentChoice::Claude));
        assert_eq!(args.model.as_deref(), Some("sonnet"));
        assert_eq!(args.git_args, ["--amend"]);
    }

    #[test]
    fn rejects_unknown_agent() {
        assert!(Args::try_parse_from(["git-aicommit", "--agent", "other"]).is_err());
    }
}
