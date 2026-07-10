//! The one [`Backend`] shipped with this crate, and the only code in it that
//! touches the system: it spawns the `claude` CLI in non-interactive print
//! mode, feeds the payload on stdin, and parses the JSON response.
//!
//! Authentication is delegated entirely to that CLI; no credentials are read,
//! stored, or passed here.

use std::io::Write;
use std::process::{Command, Stdio};

use crate::backend::{Backend, BackendError, Completion, Usage};
use crate::model::{Effort, ModelChoice};
use crate::prompt::Prompt;

/// The `claude` CLI's `--output-format json` envelope.
#[derive(serde::Deserialize)]
struct ClaudeResponse {
    is_error: bool,
    result: Option<String>,
    #[serde(default)]
    total_cost_usd: f64,
    #[serde(default)]
    usage: ClaudeUsage,
}

#[derive(serde::Deserialize, Default)]
struct ClaudeUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

/// Everything that can go wrong between spawning `claude` and holding its text.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ClaudeError {
    #[error("failed to spawn `{binary}` (is it on PATH?): {source}")]
    Spawn {
        binary: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to open claude stdin")]
    StdinUnavailable,

    #[error("failed to write prompt to claude: {0}")]
    Stdin(#[source] std::io::Error),

    #[error("failed to wait on claude: {0}")]
    Wait(#[source] std::io::Error),

    #[error("claude exited with {status}: {output}")]
    Exit { status: String, output: String },

    #[error("failed to parse claude JSON response: {source}\nraw: {raw}")]
    Parse {
        raw: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("claude reported an error in its response: {0}")]
    Reported(String),

    #[error("claude response missing `result` field")]
    MissingResult,
}

/// The default binary looked up on `PATH`.
const DEFAULT_BINARY: &str = "claude";

/// Runs a prompt through the `claude` CLI.
///
/// The model is fixed per instance because it is chosen per-diff (see
/// [`auto_select`](crate::auto_select)); build a new backend for each change.
///
/// ```no_run
/// use aicommit_core::{Backend, ClaudeCliBackend, CommitRequest};
///
/// let backend = ClaudeCliBackend::new("haiku");
/// let generated = aicommit_core::generate_commit_message(
///     &CommitRequest::new("diff --git a/x b/x\n..."),
///     &backend,
/// )?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct ClaudeCliBackend {
    binary: String,
    model: String,
    effort: Option<Effort>,
    extra_args: Vec<String>,
}

impl ClaudeCliBackend {
    /// Run `model` via the `claude` binary on `PATH`, at its default effort.
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            binary: DEFAULT_BINARY.to_string(),
            model: model.into(),
            effort: None,
            extra_args: Vec::new(),
        }
    }

    /// Run the model and effort that [`auto_select`](crate::auto_select) picked.
    pub fn from_choice(choice: ModelChoice) -> Self {
        Self::new(choice.model).with_effort(choice.effort)
    }

    /// Use a specific `claude` binary rather than searching `PATH`.
    pub fn with_binary(mut self, path: impl Into<String>) -> Self {
        self.binary = path.into();
        self
    }

    /// Set the thinking effort. `None` leaves the model's default.
    pub fn with_effort(mut self, effort: Option<Effort>) -> Self {
        self.effort = effort;
        self
    }

    /// Append arbitrary flags to the `claude` invocation, after the ones this
    /// backend sets. An escape hatch; nothing here validates them.
    pub fn with_extra_args(mut self, args: Vec<String>) -> Self {
        self.extra_args = args;
        self
    }

    /// The full argument list, minus the binary name. Pure, so the invocation
    /// can be asserted on without spawning anything.
    fn args(&self, system_prompt: &str) -> Vec<String> {
        let mut args: Vec<String> = [
            "-p",
            "--model",
            &self.model,
            "--output-format",
            "json",
            // No tools, no session, no slash commands: this is a one-shot
            // summarization of text we hand it, and nothing more.
            "--tools",
            "",
            "--no-session-persistence",
            "--disable-slash-commands",
            "--system-prompt",
            system_prompt,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        if let Some(level) = self.effort {
            args.push("--effort".to_string());
            args.push(level.as_str().to_string());
        }
        args.extend(self.extra_args.iter().cloned());
        args
    }

    /// Spawn `claude`, write the payload to its stdin, and collect its stdout.
    fn run(&self, prompt: &Prompt) -> std::result::Result<String, ClaudeError> {
        let mut child = Command::new(&self.binary)
            .args(self.args(&prompt.system))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| ClaudeError::Spawn {
                binary: self.binary.clone(),
                source,
            })?;

        child
            .stdin
            .as_mut()
            .ok_or(ClaudeError::StdinUnavailable)?
            .write_all(prompt.payload.as_bytes())
            .map_err(ClaudeError::Stdin)?;

        let out = child.wait_with_output().map_err(ClaudeError::Wait)?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stdout = String::from_utf8_lossy(&out.stdout);
            let output = if !stderr.trim().is_empty() {
                stderr
            } else {
                stdout
            };
            return Err(ClaudeError::Exit {
                status: out.status.to_string(),
                output: output.into_owned(),
            });
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

impl Backend for ClaudeCliBackend {
    fn complete(&self, prompt: &Prompt) -> std::result::Result<Completion, BackendError> {
        let stdout = self.run(prompt)?;
        let parsed = parse_response(&stdout)?;
        Ok(parsed)
    }
}

/// Turn the CLI's JSON envelope into a [`Completion`]. The text is returned raw
/// — `generate_commit_message` does the cleaning.
fn parse_response(stdout: &str) -> std::result::Result<Completion, ClaudeError> {
    let parsed: ClaudeResponse =
        serde_json::from_str(stdout).map_err(|source| ClaudeError::Parse {
            raw: stdout.to_string(),
            source,
        })?;

    if parsed.is_error {
        return Err(ClaudeError::Reported(stdout.to_string()));
    }
    let text = parsed.result.ok_or(ClaudeError::MissingResult)?;

    Ok(Completion {
        text,
        usage: Some(Usage {
            input_tokens: parsed.usage.input_tokens,
            cache_creation_input_tokens: parsed.usage.cache_creation_input_tokens,
            output_tokens: parsed.usage.output_tokens,
            cost_usd: Some(parsed.total_cost_usd),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_are_minimal_and_ordered() {
        let args = ClaudeCliBackend::new("haiku").args("SYS");
        assert_eq!(
            args,
            [
                "-p",
                "--model",
                "haiku",
                "--output-format",
                "json",
                "--tools",
                "",
                "--no-session-persistence",
                "--disable-slash-commands",
                "--system-prompt",
                "SYS",
            ]
        );
    }

    #[test]
    fn args_carry_effort_and_extras() {
        let backend = ClaudeCliBackend::from_choice(ModelChoice {
            model: "sonnet".to_string(),
            effort: Some(Effort::Medium),
        })
        .with_extra_args(vec!["--verbose".to_string()]);

        let args = backend.args("SYS");
        assert_eq!(args[1..3], ["--model", "sonnet"]);
        // Effort follows the fixed flags; extras come last.
        assert_eq!(args[args.len() - 3..], ["--effort", "medium", "--verbose"]);
    }

    #[test]
    fn with_binary_overrides_the_path() {
        let backend = ClaudeCliBackend::new("haiku").with_binary("/opt/claude");
        assert_eq!(backend.binary, "/opt/claude");
    }

    #[test]
    fn parse_response_extracts_text_and_usage() {
        let json = r#"{
            "is_error": false,
            "result": "feat: do the thing",
            "total_cost_usd": 0.0034,
            "usage": {"input_tokens": 10, "cache_creation_input_tokens": 5, "output_tokens": 7}
        }"#;
        let c = parse_response(json).unwrap();
        assert_eq!(c.text, "feat: do the thing");
        assert_eq!(
            c.usage,
            Some(Usage {
                input_tokens: 10,
                cache_creation_input_tokens: 5,
                output_tokens: 7,
                cost_usd: Some(0.0034),
            })
        );
    }

    #[test]
    fn parse_response_returns_text_uncleaned() {
        // Fence stripping belongs to `generate_commit_message`, not the backend.
        let json = r#"{"is_error": false, "result": "```\nhi\n```", "usage": {}}"#;
        assert_eq!(parse_response(json).unwrap().text, "```\nhi\n```");
    }

    #[test]
    fn parse_response_rejects_bad_envelopes() {
        let cases = [
            (r#"not json"#, "failed to parse"),
            (
                r#"{"is_error": true, "result": "x", "usage": {}}"#,
                "reported an error",
            ),
            (r#"{"is_error": false, "usage": {}}"#, "missing `result`"),
        ];
        for (json, expected) in cases {
            let err = parse_response(json).unwrap_err().to_string();
            assert!(err.contains(expected), "{json} -> {err}");
        }
    }
}
