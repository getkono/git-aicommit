# git-aicommit

A tiny Rust CLI that drafts a commit message from your staged changes using
[OpenAI Codex](https://developers.openai.com/codex/cli/) or
[Claude Code](https://docs.claude.com/en/docs/claude-code), then opens
`git commit` with the message pre-filled so you can review, edit, or abort.
When both agent CLIs are installed, Codex is preferred; use `--agent` to choose
explicitly. The model is picked automatically from the diff size or pinned with
`--model`.

## How it works

1. Parses standard `git commit` flags (see [Supported flags](#supported-flags)) to decide what to diff and how to prompt.
2. Checks you're in a git repo and that there's something to commit.
3. For plain and `--amend` commits, runs the `pre-commit` hook as an early check (`git hook run` on Git ≥ 2.36, executing the hook script directly on older git) — if it fails, the tool aborts before making any API call. (For `-a` and pathspec commits the staged index isn't what gets committed, so hooks run at commit time instead.)
4. Selects Codex first when available, otherwise Claude, unless you pin one with `--agent`. It picks a provider-specific model from the diff size (Luna/Terra for Codex, Haiku/Sonnet for Claude) unless you pin one with `--model`, then sends the relevant diff to that local agent CLI.
5. Cleans up the response and runs `git commit -e -m "<message>" …`, inheriting your terminal so `$EDITOR` opens normally.

Large diffs are truncated at 60KB to keep the prompt sane.

## Requirements

- Rust (stable)
- `git` (any reasonably recent version; the pre-commit pre-check uses `git hook run` on ≥ 2.36 and falls back to running the hook script directly on older git)
- At least one supported agent CLI installed and authenticated:
  - [`codex`](https://developers.openai.com/codex/cli/) 0.146.0 or newer
  - [`claude`](https://docs.claude.com/en/docs/claude-code) 2.x with
    `--output-format json` and `--disable-slash-commands` support

## Install

**Homebrew** (macOS and Linux):

```sh
brew install getkono/tap/git-aicommit
```

`git-aicommit` calls `git` and one of the supported agent CLIs at runtime; the
formula does not install those tools. See [Requirements](#requirements).
`brew info getkono/tap/git-aicommit` repeats this.

**From crates.io** (requires Rust):

```sh
cargo install git-aicommit
```

**Pre-built binary** — download from the [latest release](https://github.com/getkono/git-aicommit/releases/latest),
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

`git aicommit` aims to be a drop-in for `git commit`: it understands the common
flags and forwards anything else straight through. It uses Codex when `codex`
is on `PATH`, otherwise Claude when `claude` is available. Pass
`--agent codex|claude` to override that choice. By default it selects
`gpt-5.6-luna` or `gpt-5.6-terra` for Codex and `haiku` or `sonnet` for Claude,
escalating on large or many-file diffs. Pass `--model <name>` to pin a model for
the selected agent. `--agent` and `--model` must come before any Git flags.

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
git aicommit --agent claude                       # override Codex-first detection
git aicommit --agent codex --model gpt-5.6-terra # pin both agent and model
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

**Commit without reviewing:**

```sh
git aicommit -y                # commit the generated message directly, no editor
```

`-y`/`--yes` skips the editor review and commits the generated message as-is. It
does **not** skip hooks — use `-n`/`--no-verify` for that.

**Push after committing:**

```sh
git aicommit -a --push         # run `git push` once the commit succeeds
```

`--push` runs a bare `git push` (so git uses the branch's configured upstream/remote) only after the commit goes through — if you abort the commit or it fails, nothing is pushed.

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
- When a commit bundles several unrelated changes, the message leads with the primary one in the subject and itemizes the rest as body bullets. A `git diff --stat` inventory of every changed file is sent alongside the diff so small or buried changes aren't dropped.
- By default the editor opens so you can review before committing; quit with an empty message to abort. `-y`/`--yes` (or `--no-edit`) commits the generated message directly, and `--dry-run` never commits.
- No API key handling here; auth is delegated entirely to the selected local
  `codex` or `claude` CLI. If neither executable is installed, generation exits
  with an actionable error.

## Using it from your own application

The message-generation logic lives in a separate library crate,
[`aicommit-core`](crates/aicommit-core), so other frontends can reuse it — an
editor wanting an AI-drafted message for its commit box, say. Hand it a diff and
it hands you a string; what you do with that string is yours. It never invokes
`git`, and it accepts any async
[`agent_text::Agent`](https://docs.rs/agent-text/latest/agent_text/trait.Agent.html).
The application chooses the concrete adapter; this CLI selects between
[`agent_text::Codex`](https://docs.rs/agent-text/latest/agent_text/struct.Codex.html)
and
[`agent_text::ClaudeCode`](https://docs.rs/agent-text/latest/agent_text/struct.ClaudeCode.html).

```rust,no_run
let choice = auto_select(diff.len(), file_count);
let mut agent = agent_text::ClaudeCode::new().with_default_model(choice.model);
if let Some(effort) = choice.effort {
    agent = agent.with_default_effort(effort);
}
let generated = aicommit_core::generate_commit_message(&request, &agent).await?;
println!("{}", generated.message);
```

See its [README](crates/aicommit-core/README.md) and `examples/editor.rs`.
