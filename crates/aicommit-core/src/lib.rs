//! Generate a git commit message from a diff.
//!
//! This crate does one thing: turn a change into a commit message. It does not
//! read your repository, it does not commit, and it never invokes `git`. You
//! hand it a diff; it hands you a string, and what you do with that string —
//! commit it, show it in an editor's commit box, put it on a clipboard — is
//! entirely yours.
//!
//! # System dependencies
//!
//! This crate performs no I/O. Agent execution is supplied through
//! [`agent_text::Agent`], so callers choose the concrete transport.
//!
//! # Example
//!
//! ```no_run
//! use agent_text::ClaudeCode;
//! use aicommit_core::{auto_select, CommitRequest};
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let request = CommitRequest::new(std::fs::read_to_string("change.patch")?)
//!     .with_file_count(1);
//!
//! let choice = auto_select(request.diff.len(), request.file_count);
//! let mut agent = ClaudeCode::new().with_default_model(choice.model);
//! if let Some(effort) = choice.effort {
//!     agent = agent.with_default_effort(effort);
//! }
//! let generated = aicommit_core::generate_commit_message(&request, &agent).await?;
//!
//! println!("{}", generated.message);
//! # Ok(())
//! # }
//! ```
//!
//! For finer control, do it by hand: [`build_prompt`], then
//! [`agent_text::Agent::generate`], then [`clean_message`].

mod compress;
mod error;
mod model;
mod prompt;
mod request;

pub use agent_text::{Agent, GenerationRequest, Usage};
pub use compress::{
    CompressOptions, CompressionReport, Detail, FileReport, MovedBlock, SubstitutionCluster,
    compress_diff,
};
pub use error::{CoreError, Result};
pub use model::{
    ESCALATE_DIFF_BYTES, ESCALATE_FILE_COUNT, Effort, LARGE_DIFF_MODEL, ModelChoice,
    SMALL_DIFF_MODEL, auto_select, auto_select_with_models,
};
pub use prompt::{
    DEFAULT_MAX_DIFF_BYTES, build_prompt, build_prompt_with_max, build_system_prompt, truncate_diff,
};
pub use request::CommitRequest;

/// A commit message, ready to use, plus what it cost to produce.
#[derive(Debug, Clone)]
pub struct Generated {
    /// The message: cleaned, non-empty, and safe to hand to `git commit -m`.
    pub message: String,
    /// `None` when the agent reports no usage.
    pub usage: Option<Usage>,
}

/// Build the prompt for `request`, run it through `agent`, and clean up the
/// answer.
///
/// This is the whole library in one call. Cleaning and the non-empty check live
/// here rather than in the adapter, so every [`Agent`] gets them.
pub async fn generate_commit_message(
    request: &CommitRequest,
    agent: &(impl Agent + ?Sized),
) -> Result<Generated> {
    let prompt = build_prompt(request);
    let generation = agent.generate(&prompt).await?;

    let message = clean_message(&generation.text);
    if message.is_empty() {
        return Err(CoreError::EmptyMessage);
    }
    Ok(Generated {
        message,
        usage: generation.usage,
    })
}

/// Strip the stray code fences and surrounding whitespace models sometimes add.
pub fn clean_message(raw: &str) -> String {
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

    /// An agent that returns a canned answer, so `generate_commit_message` can
    /// be tested without a model.
    struct StubAgent {
        text: &'static str,
        usage: Option<Usage>,
    }

    #[agent_text::async_trait]
    impl Agent for StubAgent {
        async fn generate(
            &self,
            prompt: &GenerationRequest,
        ) -> std::result::Result<agent_text::Generation, agent_text::Error> {
            assert!(
                prompt
                    .system_prompt
                    .as_deref()
                    .unwrap()
                    .contains("Conventional Commits")
            );
            Ok(agent_text::Generation {
                text: self.text.to_string(),
                usage: self.usage.clone(),
                model: Some("stub".to_string()),
                elapsed: std::time::Duration::ZERO,
            })
        }
    }

    struct FailingAgent;

    #[agent_text::async_trait]
    impl Agent for FailingAgent {
        async fn generate(
            &self,
            _: &GenerationRequest,
        ) -> std::result::Result<agent_text::Generation, agent_text::Error> {
            Err(agent_text::Error::InvalidResponse {
                message: "no model today".to_string(),
            })
        }
    }

    #[test]
    fn clean_message_strips_fences() {
        assert_eq!(clean_message("hello"), "hello");
        assert_eq!(clean_message("```\nhello\n```"), "hello");
        assert_eq!(clean_message("```text\nhello\nworld\n```"), "hello\nworld");
        assert_eq!(clean_message("  hello  "), "hello");
    }

    #[tokio::test]
    async fn generate_cleans_and_passes_usage_through() {
        let usage = Usage {
            total_input_tokens: Some(12),
            output_tokens: Some(34),
            cost_usd: Some(0.5),
            ..Default::default()
        };
        let agent = StubAgent {
            text: "```\nfeat: add a thing\n```",
            usage: Some(usage.clone()),
        };
        let g = generate_commit_message(&CommitRequest::new("DIFF"), &agent)
            .await
            .unwrap();
        assert_eq!(g.message, "feat: add a thing");
        assert_eq!(g.usage, Some(usage));
    }

    #[tokio::test]
    async fn generate_rejects_a_blank_answer() {
        // Whitespace-only and fence-only answers both clean down to nothing.
        for text in ["   \n  ", "```\n\n```"] {
            let agent = StubAgent { text, usage: None };
            let err = generate_commit_message(&CommitRequest::new("DIFF"), &agent)
                .await
                .unwrap_err();
            assert!(
                matches!(err, CoreError::EmptyMessage),
                "{text:?} -> {err:?}"
            );
        }
    }

    #[tokio::test]
    async fn generate_wraps_an_agent_failure() {
        let err = generate_commit_message(&CommitRequest::new("DIFF"), &FailingAgent)
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::Agent(_)));
        assert!(err.to_string().contains("no model today"));
    }
}
