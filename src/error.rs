//! The crate-wide error type.
//!
//! Variants are categorized by subsystem (matching the module split), so failure
//! modes are auditable at a glance. The string-carrying variants preserve the
//! rich, context-bearing messages the tool has always produced — including the
//! echoed `git …` command in diff errors — while the type tells you which
//! subsystem failed.

/// Crate-wide result alias.
pub(crate) type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    /// An intercepted git-commit flag was malformed (e.g. `-t` with no file).
    #[error("{0}")]
    Flags(String),

    /// The current directory is not inside a git repository.
    #[error("not inside a git repository")]
    NotARepo,

    /// There is nothing to commit; carries the mode-specific explanation.
    #[error("{0}")]
    NoChanges(String),

    /// A git subprocess failed to launch or exited non-zero, or a precondition
    /// failed (amend with no commits, staging aborted, hooks failed, …).
    #[error("{0}")]
    Git(String),

    /// The `claude` CLI failed to run, reported an error, or returned no message.
    #[error("{0}")]
    Claude(String),

    /// Reading the `-t`/`--template` file failed.
    #[error("failed to read template `{path}`: {source}")]
    Template {
        path: String,
        #[source]
        source: std::io::Error,
    },
}
