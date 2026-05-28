# git-aicommit

A tiny Rust CLI that drafts a commit message from your staged changes using
[Claude Code](https://docs.claude.com/en/docs/claude-code) (Haiku), then opens
`git commit` with the message pre-filled so you can review, edit, or abort.

## How it works

1. Parses standard `git commit` flags (see [Supported flags](#supported-flags)) to decide what to diff and how to prompt.
2. Checks you're in a git repo and that there's something to commit.
3. For plain and `--amend` commits, runs `git hook run --ignore-missing pre-commit` as an early check — if hooks fail, the tool aborts before making any API call. (For `-a` and pathspec commits the staged index isn't what gets committed, so hooks run at commit time instead.)
4. Feeds the relevant diff to `claude -p --model haiku` over stdin.
5. Cleans up the response and runs `git commit -e -m "<message>" …`, inheriting your terminal so `$EDITOR` opens normally.

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

`git aicommit` aims to be a drop-in for `git commit`: it understands the common flags and forwards anything else straight through. Pass `--model` (if you use it) before any git flags; run `git aicommit --help` for a summary.

### Supported flags

**Shape the diff the AI sees:**

```sh
git aicommit -a                # include all tracked changes (like `git commit -a`)
git aicommit src/foo.rs        # commit only these paths (working-tree content, like git's --only)
git aicommit -p                # stage hunks interactively first, then summarize what you staged
git aicommit --amend           # regenerate the message from the previous message + combined diff
```

**Steer the AI:**

```sh
git aicommit -m "call out the perf fix"     # an instruction, NOT a literal message (repeatable)
git aicommit -t .gitmessage                 # make the output follow a template file
```

**Forwarded verbatim to `git commit`** — `-e`/`--edit` (on by default), `-n`/`--no-verify`, `-s`/`--signoff`, `-S`/`--gpg-sign`, `--author`, `--date`, `--allow-empty`, `--no-edit`, and anything else not listed here:

```sh
git aicommit --no-verify --signoff
```

`--no-verify` serves double duty: it skips the pre-commit pre-check (so no API tokens are spent if you intend to bypass hooks) **and** passes `--no-verify` to the final `git commit`.

**Preview without committing:**

```sh
git aicommit --dry-run         # print the diff + generated message, then exit
```

**Bypass the AI entirely** — when the message already comes from elsewhere, git handles the commit directly with no API call:

```sh
git aicommit --fixup HEAD~2
git aicommit --squash <commit>
git aicommit -C <commit>       # also -c / -F / --reuse-message / --reedit-message / --file
```

Use `--` to separate paths from flags when a filename could look like a flag:

```sh
git aicommit -- --weird-filename
```

## Notes

- The prompt asks for Conventional Commits style (`feat:`, `fix:`, etc.), imperative subject ≤72 chars, optional body explaining the *why*.
- By default the editor opens so you can review before committing; quit with an empty message to abort. `--no-edit` commits the generated message directly, and `--dry-run` never commits.
- No API key handling here; auth is delegated entirely to the `claude` CLI.
