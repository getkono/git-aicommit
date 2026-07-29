# aicommit-core

Turn a diff into a git commit message. Nothing more.

This crate is the engine behind
[`git-aicommit`](https://github.com/getkono/git-aicommit), extracted so other
frontends can use it.

It does not read your repository, invoke `git`, spawn an agent, or commit. You
provide the change and an [`agent_text::Agent`]; it returns a cleaned commit
message and portable usage metadata.

## Usage

```rust,no_run
use aicommit_core::{CommitRequest, auto_select};
use agent_text::ClaudeCode;

# async fn run(my_diff: String, my_diff_stat: String)
#     -> Result<(), Box<dyn std::error::Error>>
# {
let request = CommitRequest {
    diff: my_diff,      // full, un-truncated
    stat: my_diff_stat, // optional `git diff --stat` inventory
    file_count: 3,
    ..Default::default()
};

let choice = auto_select(request.diff.len(), request.file_count);
let mut agent = ClaudeCode::new().with_default_model(choice.model);
if let Some(effort) = choice.effort {
    agent = agent.with_default_effort(effort);
}

let generated = aicommit_core::generate_commit_message(&request, &agent).await?;
println!("{}", generated.message);
# Ok(())
# }
```

See `examples/editor.rs` for a runnable version.

`CommitRequest` also carries a template, steering instructions, a changed-file
inventory, and the previous message when amending. The diff is truncated on a
UTF-8 boundary at `DEFAULT_MAX_DIFF_BYTES`; pass the full diff so model
selection sees its true size.

## Agent boundary

`generate_commit_message`:

1. builds an [`agent_text::GenerationRequest`] containing the commit rules,
   explicit task, and labeled previous-message/stat/diff context;
2. awaits the supplied `Agent`;
3. strips stray code fences and rejects an empty commit message.

Use another `Agent` implementation to route generation through another CLI,
service, in-process model, or test fake. Agent execution, authentication,
tools, sessions, and provider-specific protocol handling remain outside this
crate.

For finer control, compose `build_prompt`, `Agent::generate`, and
`clean_message` yourself.

## Compatibility

Version 0.2 replaces the synchronous `Backend` API and bundled Claude process
implementation from version 0.1 with the async `agent-text` boundary.

Pre-1.0, the API may change between minor versions while the shape settles.

## License

MIT OR Apache-2.0.

[`agent_text::Agent`]: https://docs.rs/agent-text/latest/agent_text/trait.Agent.html
[`agent_text::GenerationRequest`]: https://docs.rs/agent-text/latest/agent_text/struct.GenerationRequest.html
