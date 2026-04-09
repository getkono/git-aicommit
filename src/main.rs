use std::io::Write;
use std::process::{Command, Stdio};

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    // 1. Ensure we're in a git repo.
    let status = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("failed to run git: {e}"))?;
    if !status.success() {
        return Err("not inside a git repository".into());
    }

    // 2. Grab the staged diff.
    let diff_out = Command::new("git")
        .args(["diff", "--cached", "--no-color"])
        .output()
        .map_err(|e| format!("failed to run `git diff --cached`: {e}"))?;
    if !diff_out.status.success() {
        return Err(format!(
            "`git diff --cached` failed: {}",
            String::from_utf8_lossy(&diff_out.stderr)
        ));
    }
    let diff = String::from_utf8_lossy(&diff_out.stdout).to_string();
    if diff.trim().is_empty() {
        return Err("no staged changes (did you forget `git add`?)".into());
    }

    // 3. Also grab a short name-status summary for extra context.
    let stat_out = Command::new("git")
        .args(["diff", "--cached", "--stat"])
        .output()
        .map_err(|e| format!("failed to run `git diff --cached --stat`: {e}"))?;
    let stat = String::from_utf8_lossy(&stat_out.stdout).to_string();

    // 4. Truncate the diff if it's huge, so we don't blow up the prompt.
    const MAX_DIFF_BYTES: usize = 60_000;
    let diff_for_prompt = if diff.len() > MAX_DIFF_BYTES {
        let mut s = diff[..MAX_DIFF_BYTES].to_string();
        s.push_str("\n\n[diff truncated]\n");
        s
    } else {
        diff
    };

    let prompt = format!(
        "You are generating a git commit message for the following staged changes.\n\
         \n\
         Rules:\n\
         - Follow Conventional Commits style (e.g. feat:, fix:, refactor:, docs:, chore:, test:).\n\
         - First line: imperative mood, <= 72 chars, no trailing period.\n\
         - Then a blank line.\n\
         - Then an optional short body (wrapped at ~72 chars) explaining the WHY, not the what.\n\
         - Output ONLY the commit message. No code fences, no preamble, no explanation.\n\
         \n\
         --- diffstat ---\n{stat}\n\
         --- diff ---\n{diff}\n",
        stat = stat,
        diff = diff_for_prompt,
    );

    // 5. Run claude in non-interactive print mode with Haiku.
    eprintln!("generating commit message with claude haiku…");
    let mut child = Command::new("claude")
        .args(["-p", "--model", "haiku"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn `claude` (is it on PATH?): {e}"))?;

    child
        .stdin
        .as_mut()
        .ok_or("failed to open claude stdin")?
        .write_all(prompt.as_bytes())
        .map_err(|e| format!("failed to write prompt to claude: {e}"))?;

    let claude_out = child
        .wait_with_output()
        .map_err(|e| format!("failed to wait on claude: {e}"))?;
    if !claude_out.status.success() {
        return Err(format!(
            "claude exited with {}: {}",
            claude_out.status,
            String::from_utf8_lossy(&claude_out.stderr)
        ));
    }

    let message = clean_message(&String::from_utf8_lossy(&claude_out.stdout));
    if message.is_empty() {
        return Err("claude returned an empty commit message".into());
    }

    // 6. Hand off to `git commit -e -m <msg>` so the user can review/edit.
    //    Inherit stdio so the editor gets the terminal.
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
