//! The input to commit-message generation: everything the prompt needs, and
//! nothing about how the change was obtained or how the message will be used.

/// A change to describe, plus the context that shapes the message.
///
/// Deliberately free of I/O and of any notion of git: the caller collects the
/// diff however it likes (shelling out to `git`, a libgit2 binding, an editor's
/// in-memory SCM state) and reads the template file itself.
///
/// Every field but `diff` is optional in practice — `Default` gives you a bare
/// request, and [`CommitRequest::new`] is the common starting point.
#[derive(Debug, Clone, Default)]
pub struct CommitRequest {
    /// The unified diff to describe, in full. Do not pre-truncate: the model is
    /// selected from the true size, and the prompt builder truncates afterwards.
    pub diff: String,

    /// A changed-file inventory (the shape of `git diff --stat`), used as a
    /// checklist so a small change buried in — or truncated out of — a large
    /// diff still reaches the model. Empty means "omit the section".
    pub stat: String,

    /// How many files the change touches. Feeds model selection only; it never
    /// appears in the prompt.
    pub file_count: usize,

    /// The message being revised, when amending. `None` means "not amending".
    pub prev_message: Option<String>,

    /// The *contents* of a template the message must follow — not a path.
    /// Reading the file is the caller's I/O.
    pub template: Option<String>,

    /// Free-form steering the model should prioritize, e.g. "focus on the perf
    /// win". Each entry becomes its own line.
    pub instructions: Vec<String>,

    /// Whether this revises an existing commit. Adds an explanatory note to the
    /// system prompt; pair it with [`CommitRequest::prev_message`].
    pub amend: bool,
}

impl CommitRequest {
    /// A request for `diff`, with no stat, template, instructions, or amend.
    pub fn new(diff: impl Into<String>) -> Self {
        Self {
            diff: diff.into(),
            ..Default::default()
        }
    }
}
