//! The seam between this crate's pure logic and whatever actually runs a model.
//!
//! Implement [`Backend`] to plug in your own transport — an HTTP API, an
//! in-process SDK, a fake for tests. The one implementation shipped here is
//! [`ClaudeCliBackend`](crate::ClaudeCliBackend), and it is the only code in
//! this crate that touches the system.

use crate::prompt::Prompt;

/// Whatever went wrong inside a [`Backend`]. Boxed rather than an enum, so a
/// third-party backend is never forced to express its failures in our terms.
pub type BackendError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// What a completion cost, when the backend reports it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Usage {
    pub input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub output_tokens: u64,
    /// `None` when the backend doesn't price its own calls.
    pub cost_usd: Option<f64>,
}

/// A model's answer.
#[derive(Debug, Clone)]
pub struct Completion {
    /// The raw text, exactly as the model produced it.
    pub text: String,
    /// `None` when the backend reports no usage at all.
    pub usage: Option<Usage>,
}

/// Runs a [`Prompt`] against a model.
pub trait Backend {
    /// Complete `prompt`.
    ///
    /// Return the model's output **raw**: do not strip code fences, trim, or
    /// reject an empty answer. [`generate_commit_message`] applies that
    /// cleaning uniformly, so every backend gets it for free — and a caller who
    /// invokes `complete` directly sees exactly what the model said.
    ///
    /// [`generate_commit_message`]: crate::generate_commit_message
    fn complete(&self, prompt: &Prompt) -> Result<Completion, BackendError>;
}
