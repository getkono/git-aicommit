mod agent;
mod cli;
mod error;
mod flags;
mod git;
mod metrics;
mod spinner;

use aicommit_core::{CommitRequest, ModelChoice, auto_select_with_models};
use clap::Parser;

use cli::{AgentChoice, Args};
use error::{Error, Result};
use metrics::{fmt_size, metrics_line};
use spinner::Spinner;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args = Args::parse();
    if let Err(e) = run(args.agent, args.model.as_deref(), &args.git_args).await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

/// Orchestrate the whole flow: classify flags, read the diff, ask
/// `aicommit_core` for a message, then hand off to `git commit`. Everything git
/// lives here; everything about the message lives in the library. `user_model`
/// is the explicit `--model`, or `None` to auto-pick from the diff size.
async fn run(
    requested_agent: Option<AgentChoice>,
    user_model: Option<&str>,
    git_args: &[String],
) -> Result<()> {
    // 0. Short-circuits that do no AI work.
    if flags::wants_help(git_args) {
        print!("{}", cli::HELP);
        return Ok(());
    }
    if flags::wants_version(git_args) {
        print!("{}", cli::version());
        return Ok(());
    }
    if flags::is_bypass(git_args) {
        return git::run_git_commit_passthrough(git_args);
    }

    let p = flags::classify_args(git_args)?;
    // Fail before staging or hooks when no usable local agent is installed.
    let selected_agent = agent::select(requested_agent)?;

    // 1. Ensure we're in a git repo.
    {
        let sp = Spinner::new("checking git repository…");
        git::ensure_in_repo()?;
        sp.finish("git repository confirmed");
    }

    // 2. Interactive staging — hand the terminal to git, then read the index.
    git::interactive_stage(&p)?;

    // 3. Read the diff for the AI, matching what the commit will record.
    let base = git::resolve_base(&p)?;
    let diff_args = git::build_diff_args(&p, &base);
    let (diff, file_count) = {
        let sp = Spinner::new("reading changes…");
        let d = git::read_diff(&p, &diff_args)?;
        sp.finish(format!("changes ready  ({} file(s))", d.file_count));
        (d.text, d.file_count)
    };

    // 3b. Read the changed-file inventory (`git diff --stat`) as a complete
    //     checklist, so a small change buried in — or truncated out of — a large
    //     diff is still surfaced to the model. Best-effort: empty on failure.
    let diff_stat = git::read_diff_stat(&p, &base);

    // 4. Resolve the model. Honor an explicit `--model`, otherwise use the
    //    selected provider's size-based tiers.
    let (small_model, large_model) = selected_agent.model_tiers();
    let choice = match user_model {
        Some(m) => ModelChoice::new(m),
        None => {
            let c = auto_select_with_models(diff.len(), file_count, small_model, large_model);
            if c.model != small_model {
                let effort_note = c
                    .effort
                    .map(|e| format!(" (effort {e})"))
                    .unwrap_or_default();
                eprintln!(
                    "auto-selected model: {}{effort_note} — large diff ({}, {file_count} file(s))",
                    c.model,
                    fmt_size(diff.len()),
                );
            }
            c
        }
    };

    // 5. Early pre-commit hook check — only when the index equals what we'll
    //    commit (plain/amend/interactive). For `-a` and pathspec(`--only`) modes
    //    the committed content differs, so we let `git commit` run hooks for real.
    //    (Known limitation, unchanged: the `--no-verify` we add below to avoid a
    //    double pre-commit run also skips any commit-msg hook.)
    let early_check = !p.no_verify && !p.dry_run && p.commits_index();
    if early_check {
        git::run_pre_commit_hook()?;
    }

    // 6. Gather what the library needs. Reading the template file and the
    //    previous commit message is our I/O, not the library's.
    let template = match &p.template {
        Some(path) => Some(
            std::fs::read_to_string(path).map_err(|source| Error::Template {
                path: path.clone(),
                source,
            })?,
        ),
        None => None,
    };
    let request = CommitRequest {
        diff: diff.clone(),
        stat: diff_stat,
        file_count,
        prev_message: if p.amend {
            Some(git::previous_commit_message()?)
        } else {
            None
        },
        template,
        instructions: p.instructions.clone(),
        amend: p.amend,
    };

    // 7. Generate. The library builds the prompt, truncates the diff, runs the
    //    agent, and cleans the answer.
    let agent = selected_agent.build(&choice);
    let message = {
        let sp = Spinner::new(&format!(
            "generating commit message with {} {}…",
            selected_agent.display_name(),
            choice.model
        ));
        let generated = aicommit_core::generate_commit_message(&request, agent.as_ref()).await?;
        sp.finish(format!(
            "commit message generated  ({})",
            metrics_line(generated.usage.as_ref())
        ));
        generated.message
    };

    // 8. Dry run: show the diff and message, then stop before committing.
    if p.dry_run {
        println!("{diff}");
        println!("\n----- generated commit message -----\n");
        println!("{message}");
        return Ok(());
    }

    // 9. Hand off to `git commit` so the user can review/edit (unless the editor
    //    is skipped via `-y`/`--yes` or `--no-edit`).
    if p.skip_editor() {
        eprintln!();
    } else {
        eprintln!("\nopening editor to review commit message…");
    }
    let commit_args = git::build_commit_args(&message, &p, early_check);
    git::final_commit(&commit_args)?;

    // 10. Echo what was recorded. Only when the editor was skipped: there the
    //     user never saw the message, and git's own summary line shows the
    //     subject alone, so any body would be invisible.
    if p.skip_editor() {
        eprintln!("{}", committed_message_block(&message));
    }

    // 11. Push, if asked. Only reached when the commit succeeded (an aborted or
    //     failed `git commit` returns above), so we never push without a commit.
    if p.push {
        git::push()?;
    }
    Ok(())
}

/// The block printed after a successful editor-less commit: a blank line, a
/// header, then the message the commit recorded. Mirrors the `--dry-run`
/// preview so both paths present the message the same way.
fn committed_message_block(message: &str) -> String {
    format!("\n----- committed message -----\n\n{message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_block_shows_whole_message() {
        assert_eq!(
            committed_message_block("feat: add thing"),
            "\n----- committed message -----\n\nfeat: add thing"
        );
        // The body is the part git's summary line drops, so it must survive.
        let multi = "feat: add thing\n\nWhy it matters.\n- one\n- two";
        assert!(committed_message_block(multi).ends_with(multi));
    }
}
