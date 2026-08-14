//! Building the typed request an agent sees: the system prompt (rules, optional
//! template, and steering instructions), an explicit task, and labeled context
//! for the diff, changed-file inventory, and previous message when amending.
//! Also owns truncation of an oversized diff.
//!
//! Everything here is pure: same inputs, same strings, no I/O.

use crate::request::CommitRequest;
use agent_text::{ContextItem, GenerationRequest};

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

const GENERATION_TASK: &str =
    "Generate the git commit message described by the system rules from the supplied change.";

/// Explains the annotations [`crate::compress_diff`] leaves in place of a body.
///
/// Added only when the diff was actually compressed, so an ordinary small change
/// gets exactly the prompt it always did.
const COMPRESSION_LEGEND: &str = "\
\n\nThe diff has been summarized to fit a size budget. EVERY changed file is \
listed, but some bodies are replaced by a `#` annotation:\n\
- `# whitespace-only reformat` — the text is identical ignoring whitespace \
(reindentation or re-wrapping). Describe it as formatting, not as a change in behavior.\n\
- `# s/<old>/<new>/ xN` — the same replacement, N times; the `# repeated edits` \
block at the top gives the totals. Describe it once, not per file.\n\
- `# relocated content only` — the lines moved unchanged; see `# relocated blocks`.\n\
- `# bulk data change` / `# generated file` / `# binary file changed` — the body \
is omitted as noise; mention it briefly if at all.\n\
- `# context omitted`, `# … N more … lines`, `# hunk headers only`, \
`# body omitted` — the change is larger than shown.\n\
Treat every listed file as part of the commit even when its body was omitted.";

/// Assemble the system prompt: base rules + optional template + optional
/// steering instructions + an amend note.
pub fn build_system_prompt(req: &CommitRequest) -> String {
    let mut prompt = String::from(SYSTEM_PROMPT);
    if req.compressed {
        prompt.push_str(COMPRESSION_LEGEND);
    }
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

/// System prompt, task, and labeled context, truncating the diff at
/// [`DEFAULT_MAX_DIFF_BYTES`].
pub fn build_prompt(req: &CommitRequest) -> GenerationRequest {
    build_prompt_with_max(req, DEFAULT_MAX_DIFF_BYTES)
}

/// [`build_prompt`] with the truncation limit under your control.
pub fn build_prompt_with_max(req: &CommitRequest, max_diff_bytes: usize) -> GenerationRequest {
    let diff = truncate_diff(&req.diff, max_diff_bytes);
    let mut prompt =
        GenerationRequest::new(GENERATION_TASK).with_system_prompt(build_system_prompt(req));
    if let Some(message) = req.prev_message.as_deref() {
        prompt
            .context
            .push(ContextItem::text("previous commit message", message.trim()));
    }
    if !req.stat.trim().is_empty() {
        prompt.context.push(ContextItem::text(
            "changed files (git diff --stat)",
            req.stat.trim(),
        ));
    }
    prompt.context.push(ContextItem::text("unified diff", diff));
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vec<String> from &str literals, for ergonomic struct literals.
    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
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
    fn the_compression_legend_appears_only_when_needed() {
        // An uncompressed diff must get byte-for-byte the prompt it always did,
        // so the common small-commit path cannot regress.
        let plain = CommitRequest::default();
        assert_eq!(build_system_prompt(&plain), SYSTEM_PROMPT);

        let compressed = CommitRequest::default().compressed(true);
        let s = build_system_prompt(&compressed);
        assert!(s.starts_with(SYSTEM_PROMPT));
        assert!(s.contains("whitespace-only reformat"));
        assert!(s.contains("s/<old>/<new>/ xN"));
        // The completeness instruction is the point of the legend.
        assert!(s.contains("Treat every listed file as part of the commit"));
    }

    #[test]
    fn the_legend_precedes_the_user_blocks() {
        // Steering instructions are documented as the highest priority, so they
        // must still come last.
        let req = CommitRequest {
            instructions: v(&["focus on perf"]),
            template: Some("TEMPLATE BODY".to_string()),
            compressed: true,
            ..Default::default()
        };
        let s = build_system_prompt(&req);
        let legend = s.find("whitespace-only reformat").expect("legend present");
        let template = s.find("TEMPLATE BODY").expect("template present");
        let instructions = s.find("focus on perf").expect("instructions present");
        assert!(legend < template && template < instructions);
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
        assert_eq!(p.context.len(), 2);
        assert_eq!(p.context[0].label, "changed files (git diff --stat)");
        assert_eq!(p.context[1].label, "unified diff");
        let agent_text::ContextValue::Text(diff) = &p.context[1].value else {
            panic!("diff context must be text");
        };
        assert!(diff.ends_with("\n\n[diff truncated]\n"));
        assert!(diff.contains(&"x".repeat(10)));
        assert!(!diff.contains(&"x".repeat(11)));
        assert_eq!(
            p.system_prompt.as_deref(),
            Some(build_system_prompt(&req).as_str())
        );
    }

    #[test]
    fn build_prompt_labels_amend_context_in_order() {
        let req = CommitRequest {
            diff: "DIFF".to_string(),
            stat: "file | 2 +-".to_string(),
            prev_message: Some("old message\n".to_string()),
            amend: true,
            ..Default::default()
        };
        let prompt = build_prompt(&req);
        assert_eq!(
            prompt
                .context
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            [
                "previous commit message",
                "changed files (git diff --stat)",
                "unified diff"
            ]
        );
        let agent_text::ContextValue::Text(previous) = &prompt.context[0].value else {
            panic!("previous message context must be text");
        };
        assert_eq!(previous, "old message");
    }
}
