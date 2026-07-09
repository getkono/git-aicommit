//! Building the inputs a backend sees: the system prompt (rules, optional
//! template, and steering instructions) and the payload (the diff, prefixed with
//! a changed-file inventory and the previous commit message when amending).
//! Also owns truncation of an oversized diff.
//!
//! Everything here is pure: same inputs, same strings, no I/O.

use crate::request::CommitRequest;

const SYSTEM_PROMPT: &str = "\
You are generating a git commit message for staged changes provided as a unified diff.\n\
\n\
Rules:\n\
- Follow Conventional Commits style (e.g. feat:, fix:, refactor:, docs:, chore:, test:).\n\
- First line: imperative mood, <= 72 chars, no trailing period.\n\
- Then a blank line.\n\
- Then an optional short body (wrapped at ~72 chars) explaining the WHY, not the what.\n\
- The change may bundle several unrelated edits; do NOT omit the smaller ones. \
Put the primary change in the subject, then list every other notable or \
unrelated change as a body bullet (- ...) so none are dropped.\n\
- When a \"Changed files\" inventory (git diff --stat) precedes the diff, treat it \
as a checklist: every file with a substantive change should be reflected in the message.\n\
- Output ONLY the commit message. No code fences, no preamble, no explanation.";

/// How much diff [`build_prompt`] will send before truncating, chosen to stay
/// well inside a context window and token budget.
pub const DEFAULT_MAX_DIFF_BYTES: usize = 60_000;

/// The two strings a [`Backend`](crate::Backend) needs: the instructions, and
/// the change to describe.
#[derive(Debug, Clone)]
pub struct Prompt {
    /// The rules, template, and steering instructions.
    pub system: String,
    /// The (already truncated) diff, with its inventory and amend preamble.
    pub payload: String,
}

/// Assemble the system prompt: base rules + optional template + optional
/// steering instructions + an amend note.
pub fn build_system_prompt(req: &CommitRequest) -> String {
    let mut prompt = String::from(SYSTEM_PROMPT);
    if let Some(tmpl) = &req.template {
        prompt.push_str(
            "\n\nThe commit message MUST follow this template exactly. \
             Preserve its structure and headings; fill in the content:\n",
        );
        prompt.push_str(tmpl.trim_end());
    }
    if !req.instructions.is_empty() {
        prompt.push_str("\n\nAdditional instructions from the user (prioritize these):\n");
        prompt.push_str(&req.instructions.join("\n"));
    }
    if req.amend {
        prompt.push_str(
            "\n\nThis revises an existing commit (--amend). You are given the previous \
             commit message and the combined diff of the amended commit; produce an \
             improved message describing the full change.",
        );
    }
    prompt
}

/// The payload: the diff, prefixed with a changed-file inventory so no small
/// change is overlooked, and with the previous commit message when amending.
/// Empty sections are omitted. `diff` is used verbatim — truncate it first.
pub fn build_payload(diff: &str, stat: &str, prev_msg: Option<&str>) -> String {
    let mut payload = String::new();
    if let Some(m) = prev_msg {
        payload.push_str(&format!(
            "Previous commit message:\n{}\n\n---\n\n",
            m.trim()
        ));
    }
    if !stat.trim().is_empty() {
        payload.push_str(&format!(
            "Changed files (git diff --stat):\n{}\n\n---\n\n",
            stat.trim()
        ));
    }
    payload.push_str(diff);
    payload
}

/// Truncate the diff to `max_bytes` on a char boundary (so we never split a
/// UTF-8 sequence), appending a marker when it was cut.
pub fn truncate_diff(diff: &str, max_bytes: usize) -> String {
    if diff.len() > max_bytes {
        let mut end = max_bytes;
        while !diff.is_char_boundary(end) {
            end -= 1;
        }
        let mut s = diff[..end].to_string();
        s.push_str("\n\n[diff truncated]\n");
        s
    } else {
        diff.to_string()
    }
}

/// System prompt + payload, truncating the diff at [`DEFAULT_MAX_DIFF_BYTES`].
pub fn build_prompt(req: &CommitRequest) -> Prompt {
    build_prompt_with_max(req, DEFAULT_MAX_DIFF_BYTES)
}

/// [`build_prompt`] with the truncation limit under your control.
pub fn build_prompt_with_max(req: &CommitRequest, max_diff_bytes: usize) -> Prompt {
    let diff = truncate_diff(&req.diff, max_diff_bytes);
    Prompt {
        system: build_system_prompt(req),
        payload: build_payload(&diff, &req.stat, req.prev_message.as_deref()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vec<String> from &str literals, for ergonomic struct literals.
    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn payload_blocks() {
        // No stat, no previous message → just the diff.
        assert_eq!(build_payload("DIFF", "", None), "DIFF");

        // The stat is prepended as a labeled inventory before the diff.
        let with_stat = build_payload("DIFF", " file | 2 +-", None);
        assert!(with_stat.starts_with("Changed files (git diff --stat):\nfile | 2 +-\n\n---\n\n"));
        assert!(with_stat.ends_with("DIFF"));

        // Amend prefix comes first, then the inventory, then the diff.
        let amend = build_payload("DIFF", "stat", Some("old msg\n"));
        assert!(amend.starts_with("Previous commit message:\nold msg\n\n---\n\n"));
        assert!(amend.contains("Changed files (git diff --stat):\nstat\n\n---\n\n"));
        assert!(amend.ends_with("DIFF"));

        // A blank/whitespace stat is omitted entirely.
        assert_eq!(build_payload("DIFF", "   ", None), "DIFF");
    }

    #[test]
    fn system_prompt_blocks() {
        let plain = CommitRequest::default();
        let s = build_system_prompt(&plain);
        assert!(s.starts_with(SYSTEM_PROMPT));
        assert!(!s.contains("template"));
        assert!(!s.contains("revises an existing commit"));

        let req = CommitRequest {
            instructions: v(&["focus on perf"]),
            ..Default::default()
        };
        assert!(build_system_prompt(&req).contains("focus on perf"));

        let req = CommitRequest {
            template: Some("TEMPLATE BODY".to_string()),
            ..Default::default()
        };
        assert!(build_system_prompt(&req).contains("TEMPLATE BODY"));

        let req = CommitRequest {
            amend: true,
            ..Default::default()
        };
        assert!(build_system_prompt(&req).contains("revises an existing commit"));
    }

    #[test]
    fn truncate_diff_passthrough_and_boundary() {
        const MAX: usize = DEFAULT_MAX_DIFF_BYTES;

        // Short input is returned unchanged.
        assert_eq!(truncate_diff("short diff", MAX), "short diff");

        // Over-long input is cut and gains the marker.
        let big = "x".repeat(MAX + 100);
        let out = truncate_diff(&big, MAX);
        assert!(out.len() < big.len());
        assert!(out.ends_with("\n\n[diff truncated]\n"));

        // A multi-byte char straddling the limit is dropped wholesale, never split.
        let mut s = "a".repeat(MAX - 1);
        s.push('é'); // 2 bytes, occupying indices MAX-1..=MAX
        s.push_str(&"b".repeat(100));
        let out = truncate_diff(&s, MAX);
        assert!(out.ends_with("\n\n[diff truncated]\n"));
        assert!(out.starts_with(&"a".repeat(MAX - 1)));
        assert!(!out.contains('é'));

        // The limit is honored, not just the default.
        assert_eq!(truncate_diff("abcdef", 3), "abc\n\n[diff truncated]\n");
    }

    #[test]
    fn build_prompt_truncates_the_diff() {
        let req = CommitRequest {
            diff: "x".repeat(100),
            stat: "f | 1 +".to_string(),
            ..Default::default()
        };
        let p = build_prompt_with_max(&req, 10);
        assert!(p.payload.starts_with("Changed files (git diff --stat):"));
        assert!(p.payload.ends_with("\n\n[diff truncated]\n"));
        assert!(p.payload.contains(&"x".repeat(10)));
        assert!(!p.payload.contains(&"x".repeat(11)));
        assert_eq!(p.system, build_system_prompt(&req));
    }
}
