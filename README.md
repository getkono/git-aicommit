# git-aicommit

A tiny Rust CLI that drafts a commit message from your staged changes using
[OpenAI Codex](https://developers.openai.com/codex/cli/) or
[Claude Code](https://docs.claude.com/en/docs/claude-code), then opens
`git commit` with the message pre-filled so you can review, edit, or abort.

## Install

```sh
# Homebrew (macOS/Linux)
brew install getkono/tap/git-aicommit

# crates.io (requires Rust)
cargo install git-aicommit

# Pre-built binary — download from the latest release, then:
#   https://github.com/getkono/git-aicommit/releases/latest
tar xzf git-aicommit-x86_64-unknown-linux-musl.tar.gz
mv git-aicommit ~/.local/bin/

# From source (requires Rust)
cargo build --release
cp target/release/git-aicommit ~/.local/bin/
```

The binary must be named `git-aicommit` and be on your `$PATH` for git to pick it
up as the `git aicommit` subcommand.

## Requirements

- `git`
- At least one agent CLI, installed and authenticated:
  [`codex`](https://developers.openai.com/codex/cli/) 0.146.0+ or
  [`claude`](https://docs.claude.com/en/docs/claude-code) 2.x

## Usage

```sh
git add -p
git aicommit
```

Your editor opens with the drafted message. Save to commit, or quit with an empty
message to abort.

```sh
git aicommit -a                          # include all tracked changes
git aicommit --amend                     # redraft from previous message + diff
git aicommit -m "call out the perf fix"  # steering instruction, NOT a literal message
git aicommit -y --push                   # skip the editor, push after committing
git aicommit --dry-run                   # print the message and exit
```

`git aicommit` is a drop-in for `git commit`: unrecognized flags are forwarded
verbatim, and `git aicommit --help` lists everything it handles itself.

## Choosing the agent and model

Codex runs when `codex` is on your `$PATH`, otherwise Claude. Pass
`--agent codex|claude` to pin one.

The model is picked from the size of the change — `gpt-5.6-luna` → `gpt-5.6-terra`
for Codex, `haiku` → `sonnet` for Claude — escalating once a diff exceeds 16KB or
touches 8 files, since that's where a single-pass summary starts dropping detail.
Pass `--model <name>` to pin one.

`--agent` and `--model` must come before any git flags. There is no API key
handling here; auth is delegated entirely to the local `codex` or `claude` CLI.

## Notes

- The prompt asks for Conventional Commits style (`feat:`, `fix:`, …), an
  imperative subject ≤72 chars, and an optional body explaining the *why*. When a
  commit bundles unrelated changes, the subject leads with the primary one and the
  body itemizes the rest.
- For plain and `--amend` commits, the `pre-commit` hook runs *before* any API
  call, so a failing hook costs you nothing. `-n`/`--no-verify` skips both that
  pre-check and the hook at commit time.

## Using it from your own application

The message-generation logic lives in a separate library crate,
[`aicommit-core`](crates/aicommit-core), so other frontends can reuse it. Hand it a
diff and it hands you a string; it never invokes `git`. See its
[README](crates/aicommit-core/README.md) and `examples/editor.rs`.
