#!/usr/bin/env bash
set -euo pipefail

# Release script for covrs
# Publishes to crates.io, tags the version in git, and pushes the tag.
#
# Usage:
#   ./scripts/release.sh          # use version from Cargo.toml
#   ./scripts/release.sh 0.2.0    # bump to specified version first

CARGO_TOML="Cargo.toml"

# --- helpers ---
die() { echo "ERROR: $*" >&2; exit 1; }

current_version() {
  grep '^version' "$CARGO_TOML" | head -1 | sed 's/.*"\(.*\)".*/\1/'
}

# --- pre-flight checks ---
command -v cargo >/dev/null 2>&1 || die "cargo not found"
command -v git   >/dev/null 2>&1 || die "git not found"

# Ensure working tree is clean
if [ -n "$(git status --porcelain)" ]; then
  die "Working tree is dirty. Commit or stash changes first."
fi

# Ensure we're on main branch
BRANCH=$(git rev-parse --abbrev-ref HEAD)
if [ "$BRANCH" != "main" ] && [ "$BRANCH" != "master" ]; then
  die "Not on main/master branch (currently on '$BRANCH'). Switch branches first."
fi

# --- optional version bump ---
if [ "${1:-}" != "" ]; then
  NEW_VERSION="$1"
  echo "Bumping version to $NEW_VERSION ..."
  sed -i "s/^version = \".*\"/version = \"$NEW_VERSION\"/" "$CARGO_TOML"
  cargo check --quiet  # validate Cargo.toml after edit
  git add "$CARGO_TOML"
  git commit -m "Bump version to $NEW_VERSION"
fi

VERSION=$(current_version)
TAG="v$VERSION"

echo "Releasing covrs $VERSION ..."

# Ensure tag doesn't already exist
if git rev-parse "$TAG" >/dev/null 2>&1; then
  die "Tag $TAG already exists."
fi

# --- build & test ---
echo "Running CI checks ..."
bash "$(dirname "$0")/ci.sh"

# --- publish to crates.io ---
echo "Publishing to crates.io ..."
cargo publish

# --- tag & push ---
echo "Tagging $TAG ..."
git tag -a "$TAG" -m "$VERSION"

echo "Pushing tag $TAG ..."
git push origin "$TAG"

echo ""
echo "✅ covrs $VERSION released!"
echo "   crates.io: https://crates.io/crates/covrs/$VERSION"
echo "   git tag:   $TAG"
