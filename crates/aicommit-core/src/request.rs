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
///
/// Marked `#[non_exhaustive]`: build one with `..Default::default()` (or
/// [`CommitRequest::new`]) so later fields do not break you.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
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

    /// Whether [`diff`](Self::diff) was produced by [`crate::compress_diff`] and
    /// therefore contains `#` annotations in place of some bodies.
    ///
    /// When set, the system prompt gains a short legend explaining the notation.
    /// Leave it false for a verbatim diff so the prompt is unchanged.
    pub compressed: bool,
}

impl CommitRequest {
    /// A request for `diff`, with no stat, template, instructions, or amend.
    pub fn new(diff: impl Into<String>) -> Self {
        Self {
            diff: diff.into(),
            ..Default::default()
        }
    }

    /// Set the changed-file inventory (the shape of `git diff --stat`).
    #[must_use]
    pub fn with_stat(mut self, stat: impl Into<String>) -> Self {
        self.stat = stat.into();
        self
    }

    /// Set how many files the change touches, which feeds model selection.
    #[must_use]
    pub fn with_file_count(mut self, file_count: usize) -> Self {
        self.file_count = file_count;
        self
    }

    /// Set the message being revised, and mark the request as an amend.
    #[must_use]
    pub fn amending(mut self, prev_message: impl Into<String>) -> Self {
        self.prev_message = Some(prev_message.into());
        self.amend = true;
        self
    }

    /// Set the *contents* of a template the message must follow.
    #[must_use]
    pub fn with_template(mut self, template: Option<String>) -> Self {
        self.template = template;
        self
    }

    /// Set the free-form steering instructions to prioritize.
    #[must_use]
    pub fn with_instructions(mut self, instructions: Vec<String>) -> Self {
        self.instructions = instructions;
        self
    }

    /// Declare that the diff came from [`crate::compress_diff`], so the system
    /// prompt explains the `#` annotations it contains.
    #[must_use]
    pub fn compressed(mut self, compressed: bool) -> Self {
        self.compressed = compressed;
        self
    }
}
