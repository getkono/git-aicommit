# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
