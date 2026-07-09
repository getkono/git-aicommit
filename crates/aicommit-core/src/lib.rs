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
//! None. [`build_prompt`], [`auto_select`], and [`clean_message`] are pure
//! functions. Running a model is delegated to a [`Backend`] you supply, so this
//! crate spawns nothing and opens nothing.
//!
//! # Example
//!
//! ```no_run
//! use aicommit_core::{auto_select, Backend, CommitRequest, ModelChoice};
//! # struct MyBackend;
//! # impl Backend for MyBackend {
//! #     fn complete(&self, _: &aicommit_core::Prompt)
//! #         -> Result<aicommit_core::Completion, aicommit_core::BackendError> { unimplemented!() }
//! # }
//! # fn backend_for(_: ModelChoice) -> MyBackend { MyBackend }
//!
//! let request = CommitRequest {
//!     diff: std::fs::read_to_string("change.patch")?,
//!     file_count: 1,
//!     ..Default::default()
//! };
//!
//! let backend = backend_for(auto_select(request.diff.len(), request.file_count));
//! let generated = aicommit_core::generate_commit_message(&request, &backend)?;
//!
//! println!("{}", generated.message);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! For finer control, do it by hand: [`build_prompt`], then
//! [`Backend::complete`], then [`clean_message`].

mod backend;
mod error;
mod model;
mod prompt;
mod request;

pub use backend::{Backend, BackendError, Completion, Usage};
pub use error::{CoreError, Result};
pub use model::{
    ESCALATE_DIFF_BYTES, ESCALATE_FILE_COUNT, Effort, LARGE_DIFF_MODEL, ModelChoice,
    SMALL_DIFF_MODEL, auto_select,
};
pub use prompt::{
    DEFAULT_MAX_DIFF_BYTES, Prompt, build_payload, build_prompt, build_prompt_with_max,
    build_system_prompt, truncate_diff,
};
pub use request::CommitRequest;

/// A commit message, ready to use, plus what it cost to produce.
#[derive(Debug, Clone)]
pub struct Generated {
    /// The message: cleaned, non-empty, and safe to hand to `git commit -m`.
    pub message: String,
    /// `None` when the backend reports no usage.
    pub usage: Option<Usage>,
}

/// Build the prompt for `request`, run it through `backend`, and clean up the
/// answer.
///
/// This is the whole library in one call. Cleaning and the non-empty check live
/// here rather than in the backend, so every [`Backend`] gets them.
pub fn generate_commit_message(
    request: &CommitRequest,
    backend: &(impl Backend + ?Sized),
) -> Result<Generated> {
    let prompt = build_prompt(request);
    let completion = backend.complete(&prompt).map_err(CoreError::Backend)?;

    let message = clean_message(&completion.text);
    if message.is_empty() {
        return Err(CoreError::EmptyMessage);
    }
    Ok(Generated {
        message,
        usage: completion.usage,
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

    /// A backend that returns a canned answer, so `generate_commit_message` can
    /// be tested without a model — and so the trait is proven implementable
    /// from outside `claude.rs`.
    struct StubBackend {
        text: &'static str,
        usage: Option<Usage>,
    }

    impl Backend for StubBackend {
        fn complete(&self, prompt: &Prompt) -> std::result::Result<Completion, BackendError> {
            assert!(prompt.system.contains("Conventional Commits"));
            Ok(Completion {
                text: self.text.to_string(),
                usage: self.usage.clone(),
            })
        }
    }

    struct FailingBackend;

    impl Backend for FailingBackend {
        fn complete(&self, _: &Prompt) -> std::result::Result<Completion, BackendError> {
            Err("no model today".into())
        }
    }

    #[test]
    fn clean_message_strips_fences() {
        assert_eq!(clean_message("hello"), "hello");
        assert_eq!(clean_message("```\nhello\n```"), "hello");
        assert_eq!(clean_message("```text\nhello\nworld\n```"), "hello\nworld");
        assert_eq!(clean_message("  hello  "), "hello");
    }

    #[test]
    fn generate_cleans_and_passes_usage_through() {
        let usage = Usage {
            input_tokens: 12,
            output_tokens: 34,
            cost_usd: Some(0.5),
            ..Default::default()
        };
        let backend = StubBackend {
            text: "```\nfeat: add a thing\n```",
            usage: Some(usage.clone()),
        };
        let g = generate_commit_message(&CommitRequest::new("DIFF"), &backend).unwrap();
        assert_eq!(g.message, "feat: add a thing");
        assert_eq!(g.usage, Some(usage));
    }

    #[test]
    fn generate_rejects_a_blank_answer() {
        // Whitespace-only and fence-only answers both clean down to nothing.
        for text in ["   \n  ", "```\n\n```"] {
            let backend = StubBackend { text, usage: None };
            let err = generate_commit_message(&CommitRequest::new("DIFF"), &backend).unwrap_err();
            assert!(
                matches!(err, CoreError::EmptyMessage),
                "{text:?} -> {err:?}"
            );
        }
    }

    #[test]
    fn generate_wraps_a_backend_failure() {
        let err =
            generate_commit_message(&CommitRequest::new("DIFF"), &FailingBackend).unwrap_err();
        assert!(matches!(err, CoreError::Backend(_)));
        assert!(err.to_string().contains("no model today"));
    }
}
