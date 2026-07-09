# aicommit-core

Turn a diff into a git commit message. Nothing more.

This crate is the engine behind [`git-aicommit`](https://github.com/getkono/git-aicommit),
extracted so other frontends can use it — a code editor that wants an AI-drafted
message for its commit box, a bot, a pre-commit hook in another language.

It does not read your repository, it does not commit, and **it never invokes
`git`**. You hand it a diff; it hands you a string. What happens to that string
is entirely up to you.

## System dependencies

Exactly one, and only if you use it: [`ClaudeCliBackend`] requires the
[Claude Code CLI](https://docs.claude.com/en/docs/claude-code) (`claude`) on
`PATH`, and is the only code in this crate that spawns a process. Authentication
is delegated entirely to that CLI — no credentials are read, stored, or passed
here.

Everything else is pure. Supply your own `Backend` and this crate needs nothing
on the system at all:

```console
$ cargo tree -p aicommit-core   # serde, serde_json, thiserror. That's the lot.
```

## Usage

```rust,no_run
use aicommit_core::{auto_select, ClaudeCliBackend, CommitRequest};

let request = CommitRequest {
    diff: my_diff,                  // full, un-truncated
    stat: my_diff_stat,             // optional: a `git diff --stat` inventory
    file_count: 3,
    ..Default::default()
};

// Small changes go to a fast model; large or many-file ones escalate.
let backend = ClaudeCliBackend::from_choice(auto_select(request.diff.len(), request.file_count));
let generated = aicommit_core::generate_commit_message(&request, &backend)?;

println!("{}", generated.message);
```

See `examples/editor.rs` for a runnable version.

`CommitRequest` also carries a template to follow, steering instructions, and
the previous message when amending. The diff is truncated for you at
`DEFAULT_MAX_DIFF_BYTES`, on a char boundary — pass the full diff, since the
model is chosen from its true size.

## Bringing your own backend

`generate_commit_message` builds the prompt, runs it through a `Backend`, strips
any stray code fences, and rejects an empty answer. Implement the trait to route
that prompt anywhere — an HTTP API, an in-process SDK, a fake for tests:

```rust
use aicommit_core::{Backend, BackendError, Completion, Prompt};

struct MyBackend;

impl Backend for MyBackend {
    fn complete(&self, prompt: &Prompt) -> Result<Completion, BackendError> {
        // prompt.system — the rules, template, and instructions
        // prompt.payload — the truncated diff and its inventory
        Ok(Completion { text: my_model(&prompt.system, &prompt.payload), usage: None })
    }
}
# fn my_model(_: &str, _: &str) -> String { unimplemented!() }
```

Return the model's output raw: the cleaning is applied above you, so every
backend gets it.

For finer control, skip `generate_commit_message` and compose the pieces
yourself: `build_prompt`, then `Backend::complete`, then `clean_message`.

## Stability

Pre-1.0. The API may change between minor versions while the shape settles.

## License

MIT OR Apache-2.0.

[`ClaudeCliBackend`]: https://docs.rs/aicommit-core/latest/aicommit_core/struct.ClaudeCliBackend.html
