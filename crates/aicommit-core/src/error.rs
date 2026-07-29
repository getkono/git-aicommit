//! This crate's error type.
//!
//! Deliberately small. Anything a *frontend* can get wrong — not a git repo,
//! nothing staged, a template file that won't open — is the frontend's error to
//! define, not ours.

/// This crate's result alias.
pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CoreError {
    /// The configured agent failed to generate a result.
    #[error("agent failed: {0}")]
    Agent(#[from] agent_text::Error),

    /// The agent succeeded but said nothing usable.
    #[error("the agent returned an empty commit message")]
    EmptyMessage,
}
