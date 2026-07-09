//! This crate's error type.
//!
//! Deliberately small. Anything a *frontend* can get wrong — not a git repo,
//! nothing staged, a template file that won't open — is the frontend's error to
//! define, not ours.

use crate::backend::BackendError;

/// This crate's result alias.
pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CoreError {
    /// The backend failed to produce a completion. The cause is the backend's
    /// own error type — [`ClaudeError`](crate::ClaudeError) for the bundled one.
    #[error("backend failed: {0}")]
    Backend(#[source] BackendError),

    /// The backend succeeded but said nothing usable.
    #[error("the backend returned an empty commit message")]
    EmptyMessage,
}
