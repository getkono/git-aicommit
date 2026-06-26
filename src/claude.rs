//! Talking to the `claude` CLI: spawning it in non-interactive print mode,
//! feeding it the payload on stdin, and parsing/validating its JSON response
//! into a commit message plus usage metrics.

use std::io::Write;
use std::process::{Command, Stdio};

use crate::error::{Error, Result};

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
    output_tokens: u64,
}

/// A successfully generated commit message plus the usage stats to report.
pub(crate) struct Generated {
    pub(crate) message: String,
    usage: ClaudeUsage,
    total_cost_usd: f64,
}

impl Generated {
    /// The "12,345 in / 678 out, $0.0034" summary shown when generation finishes.
    pub(crate) fn metrics_line(&self) -> String {
        let input_total = self.usage.input_tokens + self.usage.cache_creation_input_tokens;
        format!(
            "{} in / {} out, {}",
            fmt_tokens(input_total),
            fmt_tokens(self.usage.output_tokens),
            fmt_cost(self.total_cost_usd),
        )
    }
}

/// Diffs at or above this many bytes escalate from haiku to a stronger model.
const ESCALATE_DIFF_BYTES: usize = 16_000;
/// Diffs touching at least this many files escalate too — many small, unrelated
/// changes are exactly where a single-pass summary tends to drop detail.
const ESCALATE_FILE_COUNT: usize = 8;

/// Choose the model (and thinking effort) for a diff of the given size, used when
/// the user didn't pin one with `--model`. Small diffs stay on fast, cheap haiku
/// at the default effort; large or many-file diffs escalate to sonnet with
/// `medium` effort so secondary changes aren't lost in the summary.
pub(crate) fn auto_select(
    diff_len: usize,
    file_count: usize,
) -> (&'static str, Option<&'static str>) {
    if diff_len >= ESCALATE_DIFF_BYTES || file_count >= ESCALATE_FILE_COUNT {
        ("sonnet", Some("medium"))
    } else {
        ("haiku", None)
    }
}

/// Run `claude` in non-interactive print mode with minimal context, feeding the
/// payload on stdin, and return the cleaned message plus usage stats. When
/// `effort` is `Some`, it is passed through as `--effort <level>`.
pub(crate) fn generate(
    model: &str,
    effort: Option<&str>,
    system_prompt: &str,
    payload: &str,
) -> Result<Generated> {
    let mut args = vec![
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
        system_prompt,
    ];
    if let Some(level) = effort {
        args.push("--effort");
        args.push(level);
    }
    let mut child = Command::new("claude")
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::Claude(format!("failed to spawn `claude` (is it on PATH?): {e}")))?;

    child
        .stdin
        .as_mut()
        .ok_or_else(|| Error::Claude("failed to open claude stdin".to_string()))?
        .write_all(payload.as_bytes())
        .map_err(|e| Error::Claude(format!("failed to write prompt to claude: {e}")))?;

    let claude_out = child
        .wait_with_output()
        .map_err(|e| Error::Claude(format!("failed to wait on claude: {e}")))?;
    if !claude_out.status.success() {
        let stderr = String::from_utf8_lossy(&claude_out.stderr);
        let stdout = String::from_utf8_lossy(&claude_out.stdout);
        let output = if !stderr.trim().is_empty() {
            stderr
        } else {
            stdout
        };
        return Err(Error::Claude(format!(
            "claude exited with {}: {}",
            claude_out.status, output
        )));
    }

    let stdout = String::from_utf8_lossy(&claude_out.stdout);
    let parsed: ClaudeResponse = serde_json::from_str(&stdout).map_err(|e| {
        Error::Claude(format!(
            "failed to parse claude JSON response: {e}\nraw: {stdout}"
        ))
    })?;

    if parsed.is_error {
        return Err(Error::Claude(format!(
            "claude reported an error in its response: {stdout}"
        )));
    }

    let raw_result = parsed
        .result
        .ok_or_else(|| Error::Claude("claude response missing `result` field".to_string()))?;

    let message = clean_message(&raw_result);
    if message.is_empty() {
        return Err(Error::Claude(
            "claude returned an empty commit message".to_string(),
        ));
    }

    Ok(Generated {
        message,
        usage: parsed.usage,
        total_cost_usd: parsed.total_cost_usd,
    })
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
fn fmt_cost(usd: f64) -> String {
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

    #[test]
    fn clean_message_strips_fences() {
        assert_eq!(clean_message("hello"), "hello");
        assert_eq!(clean_message("```\nhello\n```"), "hello");
        assert_eq!(clean_message("```text\nhello\nworld\n```"), "hello\nworld");
        assert_eq!(clean_message("  hello  "), "hello");
    }

    #[test]
    fn auto_select_tiers() {
        // Small diff, few files → haiku, default effort.
        assert_eq!(auto_select(0, 0), ("haiku", None));
        assert_eq!(
            auto_select(ESCALATE_DIFF_BYTES - 1, ESCALATE_FILE_COUNT - 1),
            ("haiku", None)
        );
        // Either threshold (size or file count) escalates to sonnet + effort.
        assert_eq!(
            auto_select(ESCALATE_DIFF_BYTES, 1),
            ("sonnet", Some("medium"))
        );
        assert_eq!(
            auto_select(100, ESCALATE_FILE_COUNT),
            ("sonnet", Some("medium"))
        );
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
