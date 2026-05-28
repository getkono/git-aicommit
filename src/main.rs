use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};

#[derive(Parser)]
#[command(
    about = "Generate git commit messages from staged diffs using Claude",
    // Let `-h`/`--help` fall through to git_args so we can print our own help,
    // which documents the git-commit flags we intercept (see `HELP`).
    disable_help_flag = true
)]
struct Args {
    /// Claude model to use (passed directly to `claude --model`).
    /// Must come before any git flags.
    #[arg(long, default_value = "haiku")]
    model: String,

    /// Standard `git commit` flags. Recognized flags steer how the message is
    /// generated; everything else is forwarded verbatim to `git commit`.
    /// Run `git aicommit --help` for the full list.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    git_args: Vec<String>,
}

#[derive(serde::Deserialize)]
struct ClaudeResponse {
    is_error: bool,
    result: Option<String>,
    #[serde(default)]
    total_cost_usd: f64,
    usage: ClaudeUsage,
}

#[derive(serde::Deserialize, Default)]
struct ClaudeUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

const SYSTEM_PROMPT: &str = "\
You are generating a git commit message for staged changes provided as a unified diff.\n\
\n\
Rules:\n\
- Follow Conventional Commits style (e.g. feat:, fix:, refactor:, docs:, chore:, test:).\n\
- First line: imperative mood, <= 72 chars, no trailing period.\n\
- Then a blank line.\n\
- Then an optional short body (wrapped at ~72 chars) explaining the WHY, not the what.\n\
- Output ONLY the commit message. No code fences, no preamble, no explanation.";

/// The well-known SHA of git's empty tree, used as a diff base when there is no
/// suitable commit to compare against (root commit or amending a root commit).
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

const HELP: &str = "\
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Interactive {
    Patch,
    Interactive,
}

/// The result of classifying the git-commit flags the user passed.
#[derive(Debug, Default, PartialEq, Eq)]
struct ParsedArgs {
    all: bool,
    amend: bool,
    interactive: Option<Interactive>,
    dry_run: bool,
    no_edit: bool,
    no_verify: bool,
    allow_empty: bool,
    /// Steering instructions for the AI (from `-m`/`--message`), empties dropped.
    instructions: Vec<String>,
    /// Path to a template file (from `-t`/`--template`).
    template: Option<String>,
    /// Paths after `--` or bare positional tokens.
    pathspecs: Vec<String>,
    /// Flags forwarded verbatim to `git commit` (`-a`, `--amend`, `-s`, `-S…`, unknowns…).
    passthrough: Vec<String>,
}

impl ParsedArgs {
    /// True when pathspecs scope the commit (git's `--only` mode). Interactive
    /// staging consumes pathspecs itself, so they don't scope the commit there.
    fn scoped(&self) -> bool {
        self.interactive.is_none() && !self.pathspecs.is_empty()
    }

    /// True when the final commit records exactly the index (so an early
    /// pre-commit hook check is meaningful). `-a` and pathspec(`--only`) modes
    /// commit working-tree content that may differ from the index.
    fn commits_index(&self) -> bool {
        !self.all && !self.scoped()
    }
}

fn main() {
    let args = Args::parse();
    if let Err(e) = run(&args.model, &args.git_args) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

/// `true` if the user asked for help (so we print ours instead of spending tokens).
fn wants_help(git_args: &[String]) -> bool {
    git_args.iter().any(|a| a == "-h" || a == "--help")
}

/// `true` if the user supplied a message from another commit/file (`--fixup`,
/// `--squash`, `-C`/`-c`/`-F` and their long forms). There is nothing for the AI
/// to generate, so we hand the original args straight to `git commit`.
fn is_bypass(git_args: &[String]) -> bool {
    git_args.iter().any(|a| is_message_source(a))
}

fn is_message_source(a: &str) -> bool {
    const LONG: [&str; 5] = [
        "--fixup",
        "--squash",
        "--file",
        "--reuse-message",
        "--reedit-message",
    ];
    if LONG
        .iter()
        .any(|f| a == *f || a.strip_prefix(f).is_some_and(|r| r.starts_with('=')))
    {
        return true;
    }
    // Short forms `-F`/`-C`/`-c`, possibly bundled after boolean shorts (`-aC HEAD`).
    if let Some(body) = a.strip_prefix('-')
        && !a.starts_with("--")
        && !body.is_empty()
    {
        return short_token_has_msg_source(body);
    }
    false
}

/// Scan a short-flag bundle for a message-source flag, stopping at the first
/// value-taking flag (whose remainder is its value, not more flags).
fn short_token_has_msg_source(body: &str) -> bool {
    for c in body.chars() {
        match c {
            'C' | 'c' | 'F' => return true,
            'm' | 't' | 'S' | 'u' => return false,
            _ => {}
        }
    }
    false
}

/// Parse the git-commit flags into a [`ParsedArgs`]. Pure (no I/O). Returns `Err`
/// only for malformed *intercepted* flags (e.g. a `-t` with no file); unknown
/// flags are forwarded untouched.
fn classify_args(git_args: &[String]) -> Result<ParsedArgs, String> {
    let mut p = ParsedArgs::default();
    let mut i = 0;
    while i < git_args.len() {
        let tok = &git_args[i];
        if tok == "--" {
            p.pathspecs.extend(git_args[i + 1..].iter().cloned());
            break;
        } else if let Some(long) = tok.strip_prefix("--") {
            let (name, attached) = match long.split_once('=') {
                Some((n, v)) => (n, Some(v)),
                None => (long, None),
            };
            match name {
                "all" => {
                    p.all = true;
                    p.passthrough.push("--all".to_string());
                }
                "amend" => {
                    p.amend = true;
                    p.passthrough.push("--amend".to_string());
                }
                "patch" => p.interactive = Some(Interactive::Patch),
                "interactive" => p.interactive = Some(Interactive::Interactive),
                "dry-run" => p.dry_run = true,
                "edit" => {} // we always add `-e` ourselves
                "no-edit" => {
                    p.no_edit = true;
                    p.passthrough.push("--no-edit".to_string());
                }
                "no-verify" => {
                    p.no_verify = true;
                    p.passthrough.push("--no-verify".to_string());
                }
                "allow-empty" | "allow-empty-message" => {
                    p.allow_empty = true;
                    p.passthrough.push(tok.clone());
                }
                "signoff" => p.passthrough.push("--signoff".to_string()),
                "gpg-sign" => p.passthrough.push(tok.clone()),
                "message" => {
                    let v = take_value(attached, git_args, &mut i, "`--message` requires a value")?;
                    if !v.is_empty() {
                        p.instructions.push(v);
                    }
                }
                "template" => {
                    let v = take_value(attached, git_args, &mut i, "`--template` requires a file")?;
                    if v.is_empty() {
                        return Err("`--template` requires a file".to_string());
                    }
                    p.template = Some(v);
                }
                // Known value-taking flags we forward: consume the value too so it
                // isn't mistaken for a pathspec.
                "author" | "date" | "cleanup" | "trailer" | "pathspec-from-file" => {
                    p.passthrough.push(tok.clone());
                    if attached.is_none() {
                        i += 1;
                        match git_args.get(i) {
                            Some(v) => p.passthrough.push(v.clone()),
                            None => return Err(format!("`--{name}` requires a value")),
                        }
                    }
                }
                // Unknown long flag: forward verbatim (don't assume it takes a value).
                _ => p.passthrough.push(tok.clone()),
            }
        } else if tok.starts_with('-') && tok.len() > 1 {
            let body = &tok[1..];
            for (idx, c) in body.char_indices() {
                match c {
                    'a' => {
                        p.all = true;
                        p.passthrough.push("-a".to_string());
                    }
                    's' => p.passthrough.push("-s".to_string()),
                    'e' => {} // we always add `-e` ourselves
                    'n' => {
                        p.no_verify = true;
                        p.passthrough.push("-n".to_string());
                    }
                    'p' => p.interactive = Some(Interactive::Patch),
                    'm' | 't' => {
                        let rest = &body[idx + c.len_utf8()..];
                        let v = if rest.is_empty() {
                            i += 1;
                            git_args
                                .get(i)
                                .ok_or_else(|| format!("`-{c}` requires a value"))?
                                .clone()
                        } else {
                            rest.to_string()
                        };
                        if c == 'm' {
                            if !v.is_empty() {
                                p.instructions.push(v);
                            }
                        } else if v.is_empty() {
                            return Err("`-t` requires a file".to_string());
                        } else {
                            p.template = Some(v);
                        }
                        break;
                    }
                    // `-S` takes an OPTIONAL attached key id and never consumes the
                    // next token (git rejects `-S keyid`); `-u` likewise (`-uno`).
                    'S' | 'u' => {
                        let rest = &body[idx + c.len_utf8()..];
                        p.passthrough.push(format!("-{c}{rest}"));
                        break;
                    }
                    // Unknown short flag: forward as its own boolean flag.
                    other => p.passthrough.push(format!("-{other}")),
                }
            }
        } else {
            p.pathspecs.push(tok.clone());
        }
        i += 1;
    }
    Ok(p)
}

/// Resolve a value-taking flag's argument: the attached `=value`, else the next
/// token (advancing the cursor).
fn take_value(
    attached: Option<&str>,
    git_args: &[String],
    i: &mut usize,
    err: &str,
) -> Result<String, String> {
    match attached {
        Some(v) => Ok(v.to_string()),
        None => {
            *i += 1;
            git_args.get(*i).ok_or_else(|| err.to_string()).cloned()
        }
    }
}

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

/// The commit/tree the diff is taken against. For amend it's HEAD's parent
/// (or the empty tree for a root commit); otherwise HEAD (or the empty tree in a
/// repo with no commits yet).
fn resolve_base(p: &ParsedArgs) -> Result<String, String> {
    let has_head = git_ok(&["rev-parse", "--verify", "--quiet", "HEAD"]);
    if p.amend {
        if !has_head {
            return Err("nothing to amend: no commits yet".to_string());
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
fn build_diff_args(p: &ParsedArgs, base: &str) -> Vec<String> {
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

/// Assemble the system prompt: base rules + optional template + optional
/// steering instructions + an amend note. `template_contents` is the (already
/// read) template file, if any.
fn build_system_prompt(p: &ParsedArgs, template_contents: Option<&str>) -> String {
    let mut prompt = String::from(SYSTEM_PROMPT);
    if let Some(tmpl) = template_contents {
        prompt.push_str(
            "\n\nThe commit message MUST follow this template exactly. \
             Preserve its structure and headings; fill in the content:\n",
        );
        prompt.push_str(tmpl.trim_end());
    }
    if !p.instructions.is_empty() {
        prompt.push_str("\n\nAdditional instructions from the user (prioritize these):\n");
        prompt.push_str(&p.instructions.join("\n"));
    }
    if p.amend {
        prompt.push_str(
            "\n\nThis revises an existing commit (--amend). You are given the previous \
             commit message and the combined diff of the amended commit; produce an \
             improved message describing the full change.",
        );
    }
    prompt
}

/// The stdin payload for Claude: the diff, prefixed with the previous commit
/// message when amending.
fn build_stdin_payload(diff: &str, prev_msg: Option<&str>) -> String {
    match prev_msg {
        Some(m) => format!("Previous commit message:\n{}\n\n---\n\n{diff}", m.trim()),
        None => diff.to_string(),
    }
}

/// `git log -1 --pretty=%B`, trimmed.
fn previous_commit_message() -> Result<String, String> {
    let out = Command::new("git")
        .args(["log", "-1", "--pretty=%B"])
        .output()
        .map_err(|e| format!("failed to read previous commit message: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "failed to read previous commit message: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Assemble the final `git commit …` argument vector.
fn build_commit_args(message: &str, p: &ParsedArgs, add_no_verify: bool) -> Vec<String> {
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

/// Run `git commit` with the user's raw args, inheriting stdio.
fn run_git_commit_passthrough(git_args: &[String]) -> Result<(), String> {
    let status = Command::new("git")
        .arg("commit")
        .args(git_args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("failed to run `git commit`: {e}"))?;
    if !status.success() {
        return Err(format!("`git commit` exited with {status}"));
    }
    Ok(())
}

fn run(model: &str, git_args: &[String]) -> Result<(), String> {
    // 0. Short-circuits that do no AI work.
    if wants_help(git_args) {
        print!("{HELP}");
        return Ok(());
    }
    if is_bypass(git_args) {
        return run_git_commit_passthrough(git_args);
    }

    let p = classify_args(git_args)?;

    // 1. Ensure we're in a git repo.
    let pb = spinner("checking git repository…");
    let status = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| {
            pb.finish_and_clear();
            format!("failed to run git: {e}")
        })?;
    if !status.success() {
        pb.finish_and_clear();
        return Err("not inside a git repository".to_string());
    }
    pb.finish_with_message("git repository confirmed");

    // 2. Interactive staging — hand the terminal to git, then read the index.
    if let Some(kind) = p.interactive {
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
            .map_err(|e| format!("failed to run `git add {flag}`: {e}"))?;
        if !st.success() {
            return Err(format!(
                "interactive staging aborted (`git add {flag}` exited with {st})"
            ));
        }
    }

    // 3. Read the diff for the AI, matching what the commit will record.
    let base = resolve_base(&p)?;
    let diff_args = build_diff_args(&p, &base);
    let pb = spinner("reading changes…");
    let diff_out = Command::new("git").args(&diff_args).output().map_err(|e| {
        pb.finish_and_clear();
        format!("failed to run `git {}`: {e}", diff_args.join(" "))
    })?;
    if !diff_out.status.success() {
        pb.finish_and_clear();
        return Err(format!(
            "`git {}` failed: {}",
            diff_args.join(" "),
            String::from_utf8_lossy(&diff_out.stderr)
        ));
    }
    let diff = String::from_utf8_lossy(&diff_out.stdout).to_string();
    if diff.trim().is_empty() && !p.amend && !p.allow_empty {
        pb.finish_and_clear();
        let msg = if p.scoped() {
            "no changes in the given path(s)"
        } else if p.all {
            "no changes to commit (working tree matches HEAD)"
        } else {
            "no staged changes (did you forget `git add`?)"
        };
        return Err(msg.to_string());
    }
    let file_count = diff.lines().filter(|l| l.starts_with("diff --git")).count();
    pb.finish_with_message(format!("changes ready  ({file_count} file(s))"));

    // 4. Truncate the diff if it's huge, on a char boundary so we never split UTF-8.
    const MAX_DIFF_BYTES: usize = 60_000;
    let diff_for_prompt = if diff.len() > MAX_DIFF_BYTES {
        let mut end = MAX_DIFF_BYTES;
        while !diff.is_char_boundary(end) {
            end -= 1;
        }
        let mut s = diff[..end].to_string();
        s.push_str("\n\n[diff truncated]\n");
        s
    } else {
        diff.clone()
    };

    // 5. Early pre-commit hook check — only when the index equals what we'll
    //    commit (plain/amend/interactive). For `-a` and pathspec(`--only`) modes
    //    the committed content differs, so we let `git commit` run hooks for real.
    //    (Known limitation, unchanged: the `--no-verify` we add below to avoid a
    //    double pre-commit run also skips any commit-msg hook.)
    let early_check = !p.no_verify && !p.dry_run && p.commits_index();
    if early_check {
        eprintln!("running pre-commit hooks…");
        let hook_status = Command::new("git")
            .args(["hook", "run", "--ignore-missing", "pre-commit"])
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|e| format!("failed to run pre-commit hooks: {e}"))?;
        if !hook_status.success() {
            return Err(format!(
                "pre-commit hooks failed (exit {hook_status}); fix the issues and try again"
            ));
        }
        eprintln!("pre-commit hooks passed");
    }

    // 6. Build the prompt. Template + instructions shape the system prompt; amend
    //    prefixes the previous commit message to the diff.
    let template_contents = match &p.template {
        Some(path) => Some(
            std::fs::read_to_string(path)
                .map_err(|e| format!("failed to read template `{path}`: {e}"))?,
        ),
        None => None,
    };
    let system_prompt = build_system_prompt(&p, template_contents.as_deref());
    let prev_msg = if p.amend {
        Some(previous_commit_message()?)
    } else {
        None
    };
    let payload = build_stdin_payload(&diff_for_prompt, prev_msg.as_deref());

    // 7. Run claude in non-interactive print mode with minimal context.
    let pb = spinner(&format!("generating commit message with claude {model}…"));
    let mut child = Command::new("claude")
        .args([
            "-p",
            "--model",
            model,
            "--output-format",
            "json",
            "--tools",
            "",
            "--no-session-persistence",
            "--disable-slash-commands",
            "--system-prompt",
            &system_prompt,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            pb.finish_and_clear();
            format!("failed to spawn `claude` (is it on PATH?): {e}")
        })?;

    child
        .stdin
        .as_mut()
        .ok_or("failed to open claude stdin")?
        .write_all(payload.as_bytes())
        .map_err(|e| {
            pb.finish_and_clear();
            format!("failed to write prompt to claude: {e}")
        })?;

    let claude_out = child.wait_with_output().map_err(|e| {
        pb.finish_and_clear();
        format!("failed to wait on claude: {e}")
    })?;
    if !claude_out.status.success() {
        pb.finish_and_clear();
        let stderr = String::from_utf8_lossy(&claude_out.stderr);
        let stdout = String::from_utf8_lossy(&claude_out.stdout);
        let output = if !stderr.trim().is_empty() {
            stderr
        } else {
            stdout
        };
        return Err(format!(
            "claude exited with {}: {}",
            claude_out.status, output
        ));
    }

    let stdout = String::from_utf8_lossy(&claude_out.stdout);
    let parsed: ClaudeResponse = serde_json::from_str(&stdout).map_err(|e| {
        pb.finish_and_clear();
        format!("failed to parse claude JSON response: {e}\nraw: {stdout}")
    })?;

    if parsed.is_error {
        pb.finish_and_clear();
        return Err(format!(
            "claude reported an error in its response: {stdout}"
        ));
    }

    let raw_result = parsed.result.ok_or_else(|| {
        pb.finish_and_clear();
        "claude response missing `result` field".to_string()
    })?;

    let message = clean_message(&raw_result);
    if message.is_empty() {
        pb.finish_and_clear();
        return Err("claude returned an empty commit message".to_string());
    }

    let input_total = parsed.usage.input_tokens + parsed.usage.cache_creation_input_tokens;
    let output_total = parsed.usage.output_tokens;
    pb.finish_with_message(format!(
        "commit message generated  ({} in / {} out, {})",
        fmt_tokens(input_total),
        fmt_tokens(output_total),
        fmt_cost(parsed.usage.cache_read_input_tokens, parsed.total_cost_usd),
    ));

    // 8. Dry run: show the diff and message, then stop before committing.
    if p.dry_run {
        println!("{diff}");
        println!("\n----- generated commit message -----\n");
        println!("{message}");
        return Ok(());
    }

    // 9. Hand off to `git commit` so the user can review/edit (unless --no-edit).
    if p.no_edit {
        eprintln!();
    } else {
        eprintln!("\nopening editor to review commit message…");
    }
    let commit_args = build_commit_args(&message, &p, early_check);
    let status = Command::new("git")
        .args(&commit_args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("failed to run `git commit`: {e}"))?;

    if !status.success() {
        return Err(format!("`git commit` exited with {status}"));
    }
    Ok(())
}

/// Format a token count with thousands separators (e.g. 12345 -> "12,345").
fn fmt_tokens(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

/// Format cost as "$0.0034".
fn fmt_cost(_cache_read: u64, usd: f64) -> String {
    format!("${usd:.4}")
}

/// Strip stray code fences / surrounding whitespace that models sometimes add.
fn clean_message(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    if s.starts_with("```") {
        // Drop first line (``` or ```text) and trailing ```.
        if let Some(nl) = s.find('\n') {
            s = s[nl + 1..].to_string();
        }
        if let Some(idx) = s.rfind("```") {
            s.truncate(idx);
        }
    }
    s.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> ParsedArgs {
        classify_args(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
    }
    fn parse_err(args: &[&str]) -> String {
        classify_args(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap_err()
    }
    fn bypass(args: &[&str]) -> bool {
        is_bypass(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }
    /// Vec<String> from &str literals, for ergonomic assert_eq! comparisons.
    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }
    #[test]
    fn empty_input() {
        assert_eq!(parse(&[]), ParsedArgs::default());
    }

    #[test]
    fn all_flag() {
        let p = parse(&["-a"]);
        assert!(p.all);
        assert_eq!(p.passthrough, v(&["-a"]));
        let p = parse(&["--all"]);
        assert!(p.all);
        assert_eq!(p.passthrough, v(&["--all"]));
    }

    #[test]
    fn amend_flag() {
        let p = parse(&["--amend"]);
        assert!(p.amend);
        assert_eq!(p.passthrough, v(&["--amend"]));
    }

    #[test]
    fn no_verify_aliases() {
        assert!(parse(&["-n"]).no_verify);
        assert!(parse(&["--no-verify"]).no_verify);
    }

    #[test]
    fn edit_is_deduped() {
        assert!(parse(&["-e"]).passthrough.is_empty());
        assert!(parse(&["--edit"]).passthrough.is_empty());
    }

    #[test]
    fn no_edit_recorded_and_forwarded() {
        let p = parse(&["--no-edit"]);
        assert!(p.no_edit);
        assert_eq!(p.passthrough, v(&["--no-edit"]));
    }

    #[test]
    fn message_forms() {
        assert_eq!(parse(&["-m", "x"]).instructions, v(&["x"]));
        assert_eq!(parse(&["-mx"]).instructions, v(&["x"]));
        assert_eq!(parse(&["--message", "x"]).instructions, v(&["x"]));
        assert_eq!(parse(&["--message=x"]).instructions, v(&["x"]));
        assert!(parse(&["--message="]).instructions.is_empty());
        assert_eq!(parse(&["-m", "a", "-m", "b"]).instructions, v(&["a", "b"]));
        // `-m` is not forwarded to git.
        assert!(parse(&["-m", "x"]).passthrough.is_empty());
    }

    #[test]
    fn template_forms() {
        assert_eq!(parse(&["-t", "f.txt"]).template.as_deref(), Some("f.txt"));
        assert_eq!(parse(&["-tf.txt"]).template.as_deref(), Some("f.txt"));
        assert_eq!(
            parse(&["--template=f.txt"]).template.as_deref(),
            Some("f.txt")
        );
        assert_eq!(
            parse(&["--template", "f.txt"]).template.as_deref(),
            Some("f.txt")
        );
        assert!(parse_err(&["--template"]).contains("template"));
        assert!(parse_err(&["--template="]).contains("template"));
        assert!(parse_err(&["-t"]).contains("requires"));
    }

    #[test]
    fn gpg_sign_optional_value() {
        assert_eq!(parse(&["-S"]).passthrough, v(&["-S"]));
        assert_eq!(parse(&["-Skey"]).passthrough, v(&["-Skey"]));
        assert_eq!(parse(&["--gpg-sign"]).passthrough, v(&["--gpg-sign"]));
        assert_eq!(
            parse(&["--gpg-sign=key"]).passthrough,
            v(&["--gpg-sign=key"])
        );
        // `-S` must NOT consume the next token (git rejects `-S keyid`).
        let p = parse(&["-S", "key"]);
        assert_eq!(p.passthrough, v(&["-S"]));
        assert_eq!(p.pathspecs, v(&["key"]));
    }

    #[test]
    fn untracked_files_optional_value() {
        assert_eq!(parse(&["-u"]).passthrough, v(&["-u"]));
        assert_eq!(parse(&["-uno"]).passthrough, v(&["-uno"]));
    }

    #[test]
    fn signoff_and_dry_run() {
        assert_eq!(parse(&["-s"]).passthrough, v(&["-s"]));
        assert_eq!(parse(&["--signoff"]).passthrough, v(&["--signoff"]));
        assert!(parse(&["--dry-run"]).dry_run);
    }

    #[test]
    fn interactive_flags() {
        assert_eq!(parse(&["-p"]).interactive, Some(Interactive::Patch));
        assert_eq!(parse(&["--patch"]).interactive, Some(Interactive::Patch));
        assert_eq!(
            parse(&["--interactive"]).interactive,
            Some(Interactive::Interactive)
        );
    }

    #[test]
    fn short_bundles() {
        let p = parse(&["-am", "msg"]);
        assert!(p.all);
        assert_eq!(p.instructions, v(&["msg"]));
        assert_eq!(p.passthrough, v(&["-a"]));

        assert_eq!(parse(&["-ammsg"]).instructions, v(&["msg"]));

        let p = parse(&["-aem", "msg"]);
        assert!(p.all);
        assert_eq!(p.instructions, v(&["msg"]));
        assert_eq!(p.passthrough, v(&["-a"])); // `e` dropped

        let p = parse(&["-asn"]);
        assert!(p.all && p.no_verify);
        assert_eq!(p.passthrough, v(&["-a", "-s", "-n"]));

        assert_eq!(parse(&["-asSkey"]).passthrough, v(&["-a", "-s", "-Skey"]));

        let p = parse(&["-sm", "msg"]);
        assert_eq!(p.passthrough, v(&["-s"]));
        assert_eq!(p.instructions, v(&["msg"]));
    }

    #[test]
    fn pathspecs_and_dashdash() {
        assert_eq!(parse(&["file.js"]).pathspecs, v(&["file.js"]));
        assert_eq!(parse(&["a", "b", "c"]).pathspecs, v(&["a", "b", "c"]));
        assert_eq!(
            parse(&["--", "--weird-name"]).pathspecs,
            v(&["--weird-name"])
        );
        let p = parse(&["--"]);
        assert!(p.pathspecs.is_empty());
        assert!(!p.scoped());
    }

    #[test]
    fn forwarded_value_flags_consume_value() {
        let p = parse(&["--author", "Bob", "file.js"]);
        assert_eq!(p.passthrough, v(&["--author", "Bob"]));
        assert_eq!(p.pathspecs, v(&["file.js"]));

        let p = parse(&["--date=2020", "x"]);
        assert_eq!(p.passthrough, v(&["--date=2020"]));
        assert_eq!(p.pathspecs, v(&["x"]));
    }

    #[test]
    fn unknown_flags_forwarded() {
        let p = parse(&["--allow-empty"]);
        assert!(p.allow_empty);
        assert_eq!(p.passthrough, v(&["--allow-empty"]));
        assert_eq!(parse(&["-q"]).passthrough, v(&["-q"]));
        assert_eq!(parse(&["--quiet"]).passthrough, v(&["--quiet"]));
    }

    #[test]
    fn mixed_flags() {
        let p = parse(&["-a", "--amend", "-m", "tweak", "-s", "file.rs"]);
        assert!(p.all && p.amend);
        assert_eq!(p.instructions, v(&["tweak"]));
        assert!(p.passthrough.contains(&"-a".to_string()));
        assert!(p.passthrough.contains(&"--amend".to_string()));
        assert!(p.passthrough.contains(&"-s".to_string()));
        assert_eq!(p.pathspecs, v(&["file.rs"]));
    }

    #[test]
    fn bypass_positives() {
        assert!(bypass(&["--fixup", "HEAD"]));
        assert!(bypass(&["--fixup=HEAD"]));
        assert!(bypass(&["--fixup=amend:HEAD~2"]));
        assert!(bypass(&["--squash"]));
        assert!(bypass(&["--squash=abc"]));
        assert!(bypass(&["-C", "HEAD"]));
        assert!(bypass(&["-CHEAD"]));
        assert!(bypass(&["-c", "HEAD"]));
        assert!(bypass(&["-F", "file"]));
        assert!(bypass(&["--file=x"]));
        assert!(bypass(&["--reuse-message=x"]));
        assert!(bypass(&["--reedit-message"]));
        assert!(bypass(&["-aC", "HEAD"])); // bundled after a boolean short
    }

    #[test]
    fn bypass_negatives() {
        assert!(!bypass(&["-a"]));
        assert!(!bypass(&["-m", "x"]));
        assert!(!bypass(&["--amend"]));
        assert!(!bypass(&["-am", "x"])); // `m` consumes the rest; no C/c/F flag
        assert!(!bypass(&["file.js"]));
        assert!(!bypass(&[]));
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

    #[test]
    fn stdin_payload_amend_prefix() {
        assert_eq!(build_stdin_payload("DIFF", None), "DIFF");
        let amend = build_stdin_payload("DIFF", Some("old msg\n"));
        assert!(amend.starts_with("Previous commit message:\nold msg\n\n---\n\n"));
        assert!(amend.ends_with("DIFF"));
    }

    #[test]
    fn system_prompt_blocks() {
        let plain = ParsedArgs::default();
        let s = build_system_prompt(&plain, None);
        assert!(s.starts_with(SYSTEM_PROMPT));
        assert!(!s.contains("template"));
        assert!(!s.contains("revises an existing commit"));

        let p = ParsedArgs {
            instructions: v(&["focus on perf"]),
            ..Default::default()
        };
        assert!(build_system_prompt(&p, None).contains("focus on perf"));

        assert!(build_system_prompt(&plain, Some("TEMPLATE BODY")).contains("TEMPLATE BODY"));

        let p = ParsedArgs {
            amend: true,
            ..Default::default()
        };
        assert!(build_system_prompt(&p, None).contains("revises an existing commit"));
    }

    #[test]
    fn clean_message_strips_fences() {
        assert_eq!(clean_message("hello"), "hello");
        assert_eq!(clean_message("```\nhello\n```"), "hello");
        assert_eq!(clean_message("```text\nhello\nworld\n```"), "hello\nworld");
        assert_eq!(clean_message("  hello  "), "hello");
    }

    #[test]
    fn fmt_tokens_separators() {
        assert_eq!(fmt_tokens(0), "0");
        assert_eq!(fmt_tokens(999), "999");
        assert_eq!(fmt_tokens(1000), "1,000");
        assert_eq!(fmt_tokens(12345), "12,345");
        assert_eq!(fmt_tokens(1_000_000), "1,000,000");
    }
}
