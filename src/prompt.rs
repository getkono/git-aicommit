//! Building the inputs Claude sees: the system prompt (rules, optional template,
//! and steering instructions) and the stdin payload (the diff, prefixed with the
//! previous commit message when amending). Also owns truncation of an oversized
//! diff before it goes into the payload.

use crate::flags::ParsedArgs;

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

/// Cap the diff we feed Claude so we don't blow the context window / token budget.
const MAX_DIFF_BYTES: usize = 60_000;

/// Assemble the system prompt: base rules + optional template + optional
/// steering instructions + an amend note. `template_contents` is the (already
/// read) template file, if any.
pub(crate) fn build_system_prompt(p: &ParsedArgs, template_contents: Option<&str>) -> String {
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

/// The stdin payload for Claude: the diff, prefixed with a changed-file inventory
/// (`git diff --stat`) so no small change is overlooked, and with the previous
/// commit message when amending. Empty sections are omitted.
pub(crate) fn build_stdin_payload(diff: &str, stat: &str, prev_msg: Option<&str>) -> String {
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

/// Truncate the diff to [`MAX_DIFF_BYTES`] on a char boundary (so we never split
/// a UTF-8 sequence), appending a marker when it was cut.
pub(crate) fn truncate_diff(diff: &str) -> String {
    if diff.len() > MAX_DIFF_BYTES {
        let mut end = MAX_DIFF_BYTES;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Vec<String> from &str literals, for ergonomic struct literals.
    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn stdin_payload_blocks() {
        // No stat, no previous message → just the diff.
        assert_eq!(build_stdin_payload("DIFF", "", None), "DIFF");

        // The stat is prepended as a labeled inventory before the diff.
        let with_stat = build_stdin_payload("DIFF", " file | 2 +-", None);
        assert!(with_stat.starts_with("Changed files (git diff --stat):\nfile | 2 +-\n\n---\n\n"));
        assert!(with_stat.ends_with("DIFF"));

        // Amend prefix comes first, then the inventory, then the diff.
        let amend = build_stdin_payload("DIFF", "stat", Some("old msg\n"));
        assert!(amend.starts_with("Previous commit message:\nold msg\n\n---\n\n"));
        assert!(amend.contains("Changed files (git diff --stat):\nstat\n\n---\n\n"));
        assert!(amend.ends_with("DIFF"));

        // A blank/whitespace stat is omitted entirely.
        assert_eq!(build_stdin_payload("DIFF", "   ", None), "DIFF");
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
    fn truncate_diff_passthrough_and_boundary() {
        // Short input is returned unchanged.
        assert_eq!(truncate_diff("short diff"), "short diff");

        // Over-long input is cut and gains the marker.
        let big = "x".repeat(MAX_DIFF_BYTES + 100);
        let out = truncate_diff(&big);
        assert!(out.len() < big.len());
        assert!(out.ends_with("\n\n[diff truncated]\n"));

        // A multi-byte char straddling the limit is dropped wholesale, never split.
        let mut s = "a".repeat(MAX_DIFF_BYTES - 1);
        s.push('é'); // 2 bytes, occupying indices MAX_DIFF_BYTES-1..=MAX_DIFF_BYTES
        s.push_str(&"b".repeat(100));
        let out = truncate_diff(&s);
        assert!(out.ends_with("\n\n[diff truncated]\n"));
        assert!(out.starts_with(&"a".repeat(MAX_DIFF_BYTES - 1)));
        assert!(!out.contains('é'));
    }
}
