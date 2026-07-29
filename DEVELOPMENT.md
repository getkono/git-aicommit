# Development Notes

## Layout

A Cargo workspace of two crates:

| Crate | What it is |
| --- | --- |
| `crates/aicommit-core` | The library: builds the commit request, picks the model, and runs an `agent_text::Agent`. Frontend-agnostic. Never invokes `git`. |
| `crates/git-aicommit` | The CLI: parses git-commit flags, shells out to `git`, draws spinners, opens the editor. |

The boundary is enforced by the dependency graph, not by convention — the
library cannot reach `clap`, `indicatif`, or `git.rs`. Three invariants are worth
re-checking after any change to the core:

```sh
cargo tree -p aicommit-core -e normal | grep -E 'clap|indicatif|tokio' # must find nothing
grep -rn 'Command::new' crates/aicommit-core/src/                    # must find nothing
cargo tree -p aicommit-core -e normal | grep agent-text              # shared agent seam
```

The two crates are versioned independently. `git-aicommit` keeps its 1.x line;
`aicommit-core` moves on its own 0.x cadence, since its API is new and still
settling.

## Releasing

> **The `publish` job in `.github/workflows/release.yml` is currently broken.**
> A bare `cargo publish` does not work at a workspace root. It must publish
> `aicommit-core` first, wait for the crates.io index to serve that version, and
> only then publish `git-aicommit`, whose dependency on it cannot resolve any
> earlier. Fix this before cutting the next release. Also decide whether the
> library publishes on every tag or only when its API changes — the binary's
> tags are what drive releases. The build, GitHub-release, and Homebrew-tap jobs
> are unaffected.


Releases are cut with the `just release` recipe, which requires
[git-cliff](https://git-cliff.org) (`cargo install git-cliff`):

```sh
just release 0.4.0
```

This bumps the version in `crates/git-aicommit/Cargo.toml`/`Cargo.lock`, prepends a new section to
`CHANGELOG.md` for the commits since the last tag, commits, creates an annotated
`v0.4.0` tag, and pushes — which triggers the GitHub Actions release workflow
(builds binaries, creates the GitHub release, publishes to crates.io, and updates
the Homebrew tap — see [Homebrew distribution](#homebrew-distribution)).

## Changelog

`CHANGELOG.md` follows [Keep a Changelog](https://keepachangelog.com) and is
generated from [Conventional Commits](https://www.conventionalcommits.org) by
git-cliff (configured in `cliff.toml`). The committed file lists released
versions only — `just release` prepends each new section — so you rarely touch
it by hand. Pending (not-yet-released) changes are viewed on demand rather than
tracked in the file.

```sh
# See what the next release's section would look like (the unreleased commits):
just changelog-preview

# Rebuild the whole file from git history (repair / regenerate):
just changelog
```

Commit subjects drive the grouping, so write them in Conventional Commits style
(`feat:`, `fix:`, `docs:`, `refactor:`, `ci:`, …). `chore: bump version …` and
merge commits are skipped automatically.

## Homebrew distribution

`brew install getkono/tap/git-aicommit` installs a pre-built binary from a custom
tap, [`getkono/homebrew-tap`](https://github.com/getkono/homebrew-tap). The tap's
`Formula/git-aicommit.rb` is regenerated automatically by the `update-tap` job in
`.github/workflows/release.yml` on every `v*` tag: it computes the SHA-256 of each
release tarball, fills the placeholders in `.github/homebrew/git-aicommit.rb` (the
checked-in template), and commits the rendered formula to the tap.

### One-time setup (maintainer)

Until these exist, `update-tap` logs and skips, so releases still succeed.

1. Create a **public** repo `getkono/homebrew-tap` with a `Formula/` directory.
   The `homebrew-` prefix is required for `brew install getkono/tap/...` to resolve.
2. Mint a **fine-grained personal access token** scoped to *only* that repo, with
   **Contents: Read and write**.
3. Add it to this repo as the **`HOMEBREW_TAP_TOKEN`** Actions secret
   (Settings → Secrets and variables → Actions).

### Editing the formula

Edit the template at `.github/homebrew/git-aicommit.rb`, keeping the capitalised
`__VERSION__`/`__SHA256_*__` tokens intact — the workflow substitutes them. Lint
locally with `ruby -c` or `brew style .github/homebrew/git-aicommit.rb`.

## Removing broken release/tag

```sh
# Delete tag locally and remotely
git tag -d v<ver>
git push origin :refs/tags/v<ver>

# Re-create and push the tag at the current HEAD
git tag -a v<ver> -m "Release <ver>"
git push origin v<ver>
```
