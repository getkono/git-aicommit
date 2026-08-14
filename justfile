# Default recipe — show available commands
default:
    @just --list

install:
    cargo install --path crates/git-aicommit

# Format all code
fmt:
    cargo fmt --all

# Check formatting (CI mode)
fmt-check:
    cargo fmt --all -- --check

# Run clippy lints
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Auto-fix clippy lints
clippy-fix:
    cargo clippy --fix --allow-dirty --allow-staged --workspace --all-targets -- -D warnings

# Run the test suite
test:
    cargo test --workspace

# Install git hooks
setup:
    lefthook install
    cargo build

# Preview the changelog section for the not-yet-released commits.
changelog-preview:
    @git cliff --unreleased

# Rebuild CHANGELOG.md from scratch (released versions, from git history).
changelog:
    #!/usr/bin/env bash
    set -euo pipefail

    # The committed changelog lists released versions only; `just release` adds
    # each new section. This recipe rebuilds the whole file from git history
    # (use `just changelog-preview` to see the not-yet-released section).
    if ! command -v git-cliff >/dev/null 2>&1; then
        echo "Error: git-cliff not found — install with: cargo install git-cliff"
        exit 1
    fi

    ROOT="$(git rev-list --max-parents=0 HEAD | tail -1)"
    LATEST_TAG="$(git describe --tags --abbrev=0 2>/dev/null || true)"

    if [ -n "$LATEST_TAG" ]; then
        # Range up to the latest tag so an untagged merge at HEAD can't get
        # mis-attributed to the latest tagged release.
        git cliff "${ROOT}..${LATEST_TAG}" --output CHANGELOG.md
    else
        git cliff --output CHANGELOG.md
    fi

    # Trim the trailing blank lines git-cliff leaves at EOF.
    printf '%s\n' "$(cat CHANGELOG.md)" > CHANGELOG.md.tmp && mv CHANGELOG.md.tmp CHANGELOG.md

# Release: bump the binary crate's version, commit, tag, and push to trigger GH Actions release workflow.
# Only `git-aicommit` is versioned here; `aicommit-core` moves on its own 0.x cadence.
# Usage: just release 0.2.0
release version:
    #!/usr/bin/env bash
    set -euo pipefail

    # Validate semver-ish format
    if ! echo "{{version}}" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+'; then
        echo "Error: version must be in X.Y.Z format (got '{{version}}')"
        exit 1
    fi

    # git-cliff generates the changelog entry for this release
    if ! command -v git-cliff >/dev/null 2>&1; then
        echo "Error: git-cliff not found — install with: cargo install git-cliff"
        exit 1
    fi

    # Ensure working tree is clean
    if ! git diff --quiet || ! git diff --cached --quiet; then
        echo "Error: working tree has uncommitted changes — commit or stash first"
        exit 1
    fi

    TAG="v{{version}}"

    # Abort if tag already exists
    if git tag --list | grep -qx "$TAG"; then
        echo "Error: tag $TAG already exists"
        exit 1
    fi

    # Bump version in the binary crate's manifest
    MANIFEST="crates/git-aicommit/Cargo.toml"
    sed -i.bak "s/^version = \".*\"/version = \"{{version}}\"/" "$MANIFEST" && rm -f "$MANIFEST.bak"

    # Update Cargo.lock
    cargo update --workspace --precise "{{version}}" 2>/dev/null || cargo generate-lockfile

    # Prepend the changelog section for this release. --unreleased scopes to the
    # not-yet-tagged commits; --tag labels that section with this version.
    git cliff --tag "$TAG" --unreleased --prepend CHANGELOG.md

    git add "$MANIFEST" Cargo.lock CHANGELOG.md
    git commit -m "chore: bump version to {{version}}"

    # Annotated tag triggers the release workflow
    git tag -a "$TAG" -m "Release {{version}}"

    BRANCH="$(git rev-parse --abbrev-ref HEAD)"
    git push origin "$BRANCH"
    git push origin "$TAG"

    echo "Pushed $TAG — GitHub Actions release workflow is now running."
