# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.5.0](https://github.com/getkono/git-aicommit/releases/tag/v1.5.0) - 2026-08-03

### Features

- Select between Codex and Claude

### Documentation

- Document Codex-first agent selection

## [1.4.0](https://github.com/getkono/git-aicommit/releases/tag/v1.4.0) - 2026-07-29

### Documentation

- Document the shared agent boundary

### Refactor

- Generate commit messages through agent-text

## [1.3.6](https://github.com/getkono/git-aicommit/releases/tag/v1.3.6) - 2026-07-10

### Features

- **core:** Add the Backend trait's Claude Code CLI implementation
- **core:** Add aicommit-core, a frontend-agnostic message generator

### Bug Fixes

- **build:** Resolve the git dir instead of assuming ./.git

### Documentation

- Document the library surface and its single system dependency

### Refactor

- **cli:** Generate commit messages through aicommit-core
- Convert to a cargo workspace

## [1.3.5](https://github.com/getkono/git-aicommit/releases/tag/v1.3.5) - 2026-06-26

### Features

- Auto-select model and effort from diff size
- Add -y/--yes to commit without opening the editor

### Bug Fixes

- Keep small and unrelated changes in generated messages

### Documentation

- Document automatic model selection
- Document the -y/--yes flag
- Note multi-change commit message behavior

### CI/Build

- Bump actions/* to v5 to clear Node 20 deprecation

## [0.3.4](https://github.com/getkono/git-aicommit/releases/tag/v0.3.4) - 2026-06-15

### Features

- Add --push flag to push after a successful commit

## [0.3.3](https://github.com/getkono/git-aicommit/releases/tag/v0.3.3) - 2026-06-04

### Bug Fixes

- Support pre-commit pre-check on Git < 2.36

## [0.3.2](https://github.com/getkono/git-aicommit/releases/tag/v0.3.2) - 2026-05-29

### Features

- Add -V/--version reporting build metadata

## [0.3.1](https://github.com/getkono/git-aicommit/releases/tag/v0.3.1) - 2026-05-29

### Features

- Distribute via Homebrew tap on macOS and Linux
- Implement comprehensive git commit flag support
- Generate CHANGELOG with git-cliff and back-fill release history
- Run pre-commit hooks before generating commit message

### Refactor

- Split main into focused modules

### Miscellaneous

- Add rust-toolchain.toml with stable channel

## [0.3.0](https://github.com/getkono/git-aicommit/releases/tag/v0.3.0) - 2026-05-18

### Features

- Run pre-commit hooks before AI call and forward extra args to git commit

### Bug Fixes

- Handle sed backup file in version bump command

### Documentation

- Document required Claude Code CLI version

## [0.2.0](https://github.com/getkono/git-aicommit/releases/tag/v0.2.0) - 2026-04-19

### Features

- Add justfile for release automation

### Documentation

- Add development notes for tag management

### Refactor

- Remove test hook and improve code style
- Move system prompt to constant and use CLI flag

### CI/Build

- Use rust-lang crates-io-auth-action for secure publishing

### Miscellaneous

- Exclude DEVELOPMENT.md from published package
- Add development automation recipes and git hooks

## [0.1.1](https://github.com/getkono/git-aicommit/releases/tag/v0.1.1) - 2026-04-10

### Features

- Add --model flag to select Claude model at runtime

## [0.1.0](https://github.com/getkono/git-aicommit/releases/tag/v0.1.0) - 2026-04-10

### Features

- Add GitHub Actions CI/release workflows and update install docs
- Parse JSON responses and display token usage metrics
- Add progress spinners to CLI operations
- Implement git-aicommit CLI tool

### Miscellaneous

- Add dual licenses
- Add MIT License to the project
