use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};

#[derive(Parser)]
#[command(about = "Generate git commit messages from staged diffs using Claude")]
struct Args {
    /// Claude model to use (passed directly to `claude --model`)
    #[arg(long, default_value = "haiku")]
    model: String,
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

fn main() {
    let args = Args::parse();
    if let Err(e) = run(&args.model) {
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

fn run(model: &str) -> Result<(), String> {
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
        return Err("not inside a git repository".into());
    }
    pb.finish_with_message("git repository confirmed");

    // 2. Grab the staged diff.
    let pb = spinner("reading staged changes…");
    let diff_out = Command::new("git")
        .args(["diff", "--cached", "--no-color"])
        .output()
        .map_err(|e| {
            pb.finish_and_clear();
            format!("failed to run `git diff --cached`: {e}")
        })?;
    if !diff_out.status.success() {
        pb.finish_and_clear();
        return Err(format!(
            "`git diff --cached` failed: {}",
            String::from_utf8_lossy(&diff_out.stderr)
        ));
    }
    let diff = String::from_utf8_lossy(&diff_out.stdout).to_string();
    if diff.trim().is_empty() {
        pb.finish_and_clear();
        return Err("no staged changes (did you forget `git add`?)".into());
    }
    let file_count = diff
        .lines()
        .filter(|l| l.starts_with("diff --git"))
        .count();
    pb.finish_with_message(format!("staged changes ready  ({file_count} file(s))"));

    // 3. Truncate the diff if it's huge, so we don't blow up the prompt.
    const MAX_DIFF_BYTES: usize = 60_000;
    let diff_for_prompt = if diff.len() > MAX_DIFF_BYTES {
        let mut s = diff[..MAX_DIFF_BYTES].to_string();
        s.push_str("\n\n[diff truncated]\n");
        s
    } else {
        diff
    };

    // 4. Run claude in non-interactive print mode with minimal context:
    //    --tools ""              – disables all built-in tools (none needed)
    //    --system-prompt         – replaces the default system prompt
    //    --no-session-persistence – don't write session to disk
    //    --disable-slash-commands – skip skill resolution
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
            SYSTEM_PROMPT,
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
        .write_all(diff_for_prompt.as_bytes())
        .map_err(|e| {
            pb.finish_and_clear();
            format!("failed to write prompt to claude: {e}")
        })?;

    let claude_out = child
        .wait_with_output()
        .map_err(|e| {
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
        return Err(format!("claude exited with {}: {}", claude_out.status, output));
    }

    let stdout = String::from_utf8_lossy(&claude_out.stdout);
    let parsed: ClaudeResponse = serde_json::from_str(&stdout).map_err(|e| {
        pb.finish_and_clear();
        format!("failed to parse claude JSON response: {e}\nraw: {stdout}")
    })?;

    if parsed.is_error {
        pb.finish_and_clear();
        return Err(format!("claude reported an error in its response: {stdout}"));
    }

    let raw_result = parsed.result.ok_or_else(|| {
        pb.finish_and_clear();
        "claude response missing `result` field".to_string()
    })?;

    let message = clean_message(&raw_result);
    if message.is_empty() {
        pb.finish_and_clear();
        return Err("claude returned an empty commit message".into());
    }

    let input_total = parsed.usage.input_tokens + parsed.usage.cache_creation_input_tokens;
    let output_total = parsed.usage.output_tokens;
    pb.finish_with_message(format!(
        "commit message generated  ({} in / {} out, {})",
        fmt_tokens(input_total),
        fmt_tokens(output_total),
        fmt_cost(parsed.usage.cache_read_input_tokens, parsed.total_cost_usd),
    ));

    // 6. Hand off to `git commit -e -m <msg>` so the user can review/edit.
    //    Inherit stdio so the editor gets the terminal.
    eprintln!("\nopening editor to review commit message…");
    let status = Command::new("git")
        .arg("commit")
        .arg("-e")
        .arg("-m")
        .arg(&message)
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
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

/// Format cost as "$0.0034".
fn fmt_cost(_cache_read: u64, usd: f64) -> String {
    format!("${:.4}", usd)
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
