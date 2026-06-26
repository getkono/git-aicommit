mod claude;
mod cli;
mod error;
mod flags;
mod git;
mod prompt;
mod spinner;

use clap::Parser;

use cli::Args;
use error::{Error, Result};
use spinner::Spinner;

fn main() {
    let args = Args::parse();
    if let Err(e) = run(&args.model, &args.git_args) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

/// Orchestrate the whole flow: classify flags, read the diff, build the prompt,
/// ask Claude for a message, then hand off to `git commit`. Each phase delegates
/// to a focused module; this function just sequences them.
fn run(model: &str, git_args: &[String]) -> Result<()> {
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
    let diff = {
        let sp = Spinner::new("reading changes…");
        let d = git::read_diff(&p, &diff_args)?;
        sp.finish(format!("changes ready  ({} file(s))", d.file_count));
        d.text
    };

    // 4. Truncate the diff if it's huge, on a char boundary so we never split UTF-8.
    let diff_for_prompt = prompt::truncate_diff(&diff);

    // 5. Early pre-commit hook check — only when the index equals what we'll
    //    commit (plain/amend/interactive). For `-a` and pathspec(`--only`) modes
    //    the committed content differs, so we let `git commit` run hooks for real.
    //    (Known limitation, unchanged: the `--no-verify` we add below to avoid a
    //    double pre-commit run also skips any commit-msg hook.)
    let early_check = !p.no_verify && !p.dry_run && p.commits_index();
    if early_check {
        git::run_pre_commit_hook()?;
    }

    // 6. Build the prompt. Template + instructions shape the system prompt; amend
    //    prefixes the previous commit message to the diff.
    let template_contents = match &p.template {
        Some(path) => Some(
            std::fs::read_to_string(path).map_err(|source| Error::Template {
                path: path.clone(),
                source,
            })?,
        ),
        None => None,
    };
    let system_prompt = prompt::build_system_prompt(&p, template_contents.as_deref());
    let prev_msg = if p.amend {
        Some(git::previous_commit_message()?)
    } else {
        None
    };
    let payload = prompt::build_stdin_payload(&diff_for_prompt, prev_msg.as_deref());

    // 7. Run claude in non-interactive print mode with minimal context.
    let message = {
        let sp = Spinner::new(&format!("generating commit message with claude {model}…"));
        let generated = claude::generate(model, &system_prompt, &payload)?;
        sp.finish(format!(
            "commit message generated  ({})",
            generated.metrics_line()
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

    // 10. Push, if asked. Only reached when the commit succeeded (an aborted or
    //     failed `git commit` returns above), so we never push without a commit.
    if p.push {
        git::push()?;
    }
    Ok(())
}
