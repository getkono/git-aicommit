# Release: bump Cargo.toml version, commit, tag, and push to trigger GH Actions release workflow.
# Usage: just release 0.2.0
release version:
    #!/usr/bin/env bash
    set -euo pipefail

    # Validate semver-ish format
    if ! echo "{{version}}" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+'; then
        echo "Error: version must be in X.Y.Z format (got '{{version}}')"
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

    # Bump version in Cargo.toml
    sed -i "s/^version = \".*\"/version = \"{{version}}\"/" Cargo.toml

    # Update Cargo.lock
    cargo update --workspace --precise "{{version}}" 2>/dev/null || cargo generate-lockfile

    git add Cargo.toml Cargo.lock
    git commit -m "chore: bump version to {{version}}"

    # Annotated tag triggers the release workflow
    git tag -a "$TAG" -m "Release {{version}}"

    BRANCH="$(git rev-parse --abbrev-ref HEAD)"
    git push origin "$BRANCH"
    git push origin "$TAG"

    echo "Pushed $TAG — GitHub Actions release workflow is now running."
