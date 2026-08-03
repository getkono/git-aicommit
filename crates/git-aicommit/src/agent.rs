//! Selecting and configuring the local agent CLI used for generation.

use agent_text::{Agent, ClaudeCode, Codex};
use aicommit_core::ModelChoice;

use crate::cli::AgentChoice;
use crate::error::{Error, Result};

const CODEX_SMALL_DIFF_MODEL: &str = "gpt-5.6-luna";
const CODEX_LARGE_DIFF_MODEL: &str = "gpt-5.6-terra";
const CLAUDE_SMALL_DIFF_MODEL: &str = "haiku";
const CLAUDE_LARGE_DIFF_MODEL: &str = "sonnet";

impl AgentChoice {
    pub(crate) fn binary(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }

    pub(crate) fn display_name(self) -> &'static str {
        match self {
            Self::Codex => "OpenAI Codex",
            Self::Claude => "Claude Code",
        }
    }

    pub(crate) fn model_tiers(self) -> (&'static str, &'static str) {
        match self {
            Self::Codex => (CODEX_SMALL_DIFF_MODEL, CODEX_LARGE_DIFF_MODEL),
            Self::Claude => (CLAUDE_SMALL_DIFF_MODEL, CLAUDE_LARGE_DIFF_MODEL),
        }
    }

    pub(crate) fn build(self, choice: &ModelChoice) -> Box<dyn Agent> {
        match self {
            Self::Codex => {
                let mut agent = Codex::new().with_default_model(choice.model.clone());
                if let Some(effort) = choice.effort {
                    agent = agent.with_default_effort(effort);
                }
                Box::new(agent)
            }
            Self::Claude => {
                let mut agent = ClaudeCode::new().with_default_model(choice.model.clone());
                if let Some(effort) = choice.effort {
                    agent = agent.with_default_effort(effort);
                }
                Box::new(agent)
            }
        }
    }
}

/// Resolve an explicit choice or prefer Codex among the agents found on `PATH`.
pub(crate) fn select(requested: Option<AgentChoice>) -> Result<AgentChoice> {
    resolve(requested, |agent| which::which(agent.binary()).is_ok())
}

fn resolve(
    requested: Option<AgentChoice>,
    mut is_available: impl FnMut(AgentChoice) -> bool,
) -> Result<AgentChoice> {
    if let Some(agent) = requested {
        return is_available(agent)
            .then_some(agent)
            .ok_or(Error::AgentUnavailable {
                agent: agent.display_name(),
                binary: agent.binary(),
            });
    }

    [AgentChoice::Codex, AgentChoice::Claude]
        .into_iter()
        .find(|agent| is_available(*agent))
        .ok_or(Error::NoAgentAvailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_selection_prefers_codex() {
        assert_eq!(resolve(None, |_| true).unwrap(), AgentChoice::Codex);
    }

    #[test]
    fn automatic_selection_uses_the_only_available_agent() {
        assert_eq!(
            resolve(None, |agent| agent == AgentChoice::Codex).unwrap(),
            AgentChoice::Codex
        );
        assert_eq!(
            resolve(None, |agent| agent == AgentChoice::Claude).unwrap(),
            AgentChoice::Claude
        );
    }

    #[test]
    fn automatic_selection_requires_an_agent() {
        assert!(matches!(
            resolve(None, |_| false),
            Err(Error::NoAgentAvailable)
        ));
    }

    #[test]
    fn explicit_selection_is_honored_or_rejected() {
        assert_eq!(
            resolve(Some(AgentChoice::Claude), |_| true).unwrap(),
            AgentChoice::Claude
        );
        assert!(matches!(
            resolve(Some(AgentChoice::Codex), |agent| agent
                == AgentChoice::Claude),
            Err(Error::AgentUnavailable {
                binary: "codex",
                ..
            })
        ));
    }

    #[test]
    fn agents_have_provider_specific_model_tiers() {
        assert_eq!(
            AgentChoice::Codex.model_tiers(),
            ("gpt-5.6-luna", "gpt-5.6-terra")
        );
        assert_eq!(AgentChoice::Claude.model_tiers(), ("haiku", "sonnet"));
    }
}
