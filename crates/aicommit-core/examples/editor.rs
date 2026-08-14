//! What a code editor would do: hand a diff to the library, get a commit
//! message back, and decide for itself what to do with it.
//!
//! Note what is absent — no `git`, no terminal, no commit. The diff below is a
//! string literal; a real editor would pull it from whatever SCM layer it
//! already has.
//!
//!     cargo run -p aicommit-core --example editor
//!
//! Requires the `claude` CLI on PATH. Swap `ClaudeCode` for your own `Agent`
//! implementation and even that goes away.

use agent_text::ClaudeCode;
use aicommit_core::{CommitRequest, auto_select, generate_commit_message};

const DIFF: &str = "\
diff --git a/src/cache.rs b/src/cache.rs
--- a/src/cache.rs
+++ b/src/cache.rs
@@ -10,7 +10,7 @@ impl Cache {
     pub fn get(&self, key: &str) -> Option<&Entry> {
-        self.entries.get(key)
+        self.entries.get(key).filter(|e| !e.is_expired())
     }
 }
";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let request = CommitRequest::new(DIFF)
        .with_stat(" src/cache.rs | 2 +-")
        .with_file_count(1)
        .with_instructions(vec!["mention the user-visible effect".to_string()]);

    // Pick a model from the size of the change, or pass `ModelChoice::new("haiku")`.
    let choice = auto_select(request.diff.len(), request.file_count);
    let mut agent = ClaudeCode::new().with_default_model(choice.model);
    if let Some(effort) = choice.effort {
        agent = agent.with_default_effort(effort);
    }

    let generated = generate_commit_message(&request, &agent).await?;

    // The message is yours. Put it in a commit box, a clipboard, a text field.
    println!("{}", generated.message);
    if let Some(usage) = generated.usage
        && let (Some(input), Some(output)) = (usage.total_input_tokens, usage.output_tokens)
    {
        eprintln!("({input} in / {output} out)");
    }
    Ok(())
}
