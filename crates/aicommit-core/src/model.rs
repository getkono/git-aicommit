//! Picking a model for a change, when the caller hasn't pinned one.

pub use agent_text::ReasoningEffort as Effort;

/// A model to run, and how hard it should think. `effort: None` means "the
/// model's default".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelChoice {
    pub model: String,
    pub effort: Option<Effort>,
}

impl ModelChoice {
    /// Pin `model`, at its default effort. Use this for a user-supplied name.
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            effort: None,
        }
    }
}

/// Diffs at or above this many bytes escalate from the cheap model.
pub const ESCALATE_DIFF_BYTES: usize = 16_000;
/// Diffs touching at least this many files escalate too — many small, unrelated
/// changes are exactly where a single-pass summary tends to drop detail.
pub const ESCALATE_FILE_COUNT: usize = 8;

/// The model used for changes below both escalation thresholds.
pub const SMALL_DIFF_MODEL: &str = "haiku";
/// The model used once either escalation threshold is crossed.
pub const LARGE_DIFF_MODEL: &str = "sonnet";

/// Choose a model for a diff of the given size, for callers that don't pin one.
///
/// Small diffs stay on a fast, cheap model at default effort; large or
/// many-file diffs escalate to a stronger model with `medium` effort so
/// secondary changes aren't lost in the summary. Pass the *full* diff length,
/// before any truncation.
pub fn auto_select(diff_len: usize, file_count: usize) -> ModelChoice {
    if diff_len >= ESCALATE_DIFF_BYTES || file_count >= ESCALATE_FILE_COUNT {
        ModelChoice {
            model: LARGE_DIFF_MODEL.to_string(),
            effort: Some(Effort::Medium),
        }
    } else {
        ModelChoice {
            model: SMALL_DIFF_MODEL.to_string(),
            effort: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small() -> ModelChoice {
        ModelChoice::new(SMALL_DIFF_MODEL)
    }
    fn large() -> ModelChoice {
        ModelChoice {
            model: LARGE_DIFF_MODEL.to_string(),
            effort: Some(Effort::Medium),
        }
    }

    #[test]
    fn auto_select_tiers() {
        // Small diff, few files → cheap model, default effort.
        assert_eq!(auto_select(0, 0), small());
        assert_eq!(
            auto_select(ESCALATE_DIFF_BYTES - 1, ESCALATE_FILE_COUNT - 1),
            small()
        );
        // Either threshold (size or file count) escalates.
        assert_eq!(auto_select(ESCALATE_DIFF_BYTES, 1), large());
        assert_eq!(auto_select(100, ESCALATE_FILE_COUNT), large());
    }

    #[test]
    fn effort_wire_form() {
        // Agent adapters pass this through; "medium" is what Claude expects.
        assert_eq!(Effort::Medium.as_str(), "medium");
        assert_eq!(Effort::Low.to_string(), "low");
        assert_eq!(Effort::High.to_string(), "high");
    }
}
