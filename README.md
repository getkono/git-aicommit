# git-aicommit

A tiny Rust CLI that drafts a commit message from your staged changes using
[Claude Code](https://docs.claude.com/en/docs/claude-code) (Haiku), then opens
`git commit` with the message pre-filled so you can review, edit, or abort.

## How it works

1. Checks you're in a git repo and that something is staged.
2. Runs `git hook run --ignore-missing pre-commit` as an early check — if hooks fail, the tool aborts before making any API call.
3. Feeds `git diff --cached` to `claude -p --model haiku` over stdin.
4. Cleans up the response and runs `git commit -e -m "<message>"`, inheriting your terminal so `$EDITOR` opens normally.

Large diffs are truncated at 60KB to keep the prompt sane.

## Requirements

- Rust (stable)
- `git` ≥ 2.36 (for `git hook run`)
- [`claude`](https://docs.claude.com/en/docs/claude-code) CLI, installed and authenticated
  (tested with Claude Code 2.x; requires a version that supports `--output-format json` and `--disable-slash-commands`)

## Install

**From crates.io** (requires Rust):

```sh
cargo install git-aicommit
```

**Pre-built binary** — download from the [latest release](https://github.com/getkono/git-ai-commits/releases/latest),
extract, and copy to a directory on your `$PATH`:

```sh
# Linux/macOS example
tar xzf git-aicommit-x86_64-unknown-linux-musl.tar.gz
mv git-aicommit ~/.local/bin/
```

**Build from source**:

```sh
cargo build --release
cp target/release/git-aicommit ~/.local/bin/   # or anywhere on $PATH
```

Naming the binary `git-aicommit` lets you invoke it as a git subcommand.

## Usage

```sh
git add -p
git aicommit
```

Your editor opens with the AI-generated message. Save to commit, or quit with an empty message to abort.

Any arguments after the known flags are forwarded verbatim to `git commit`:

```sh
# Skip hooks (pre-commit pre-check and final git commit hooks):
git aicommit --no-verify

# Sign the commit:
git aicommit --signoff

# Combine flags:
git aicommit --no-verify --signoff
```

`--no-verify` serves double duty: it skips the pre-commit pre-check (so no API tokens are spent if you intend to bypass hooks) **and** passes `--no-verify` to the final `git commit`.

## Notes

- The prompt asks for Conventional Commits style (`feat:`, `fix:`, etc.), imperative subject ≤72 chars, optional body explaining the *why*.
- Nothing is committed without your confirmation — the editor step is always run.
- No API key handling here; auth is delegated entirely to the `claude` CLI.
