#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

usage() {
  cat <<'EOF'
Usage: scripts/release/release.sh <version>

Environment:
  RELEASE_REMOTE=origin          Git remote to push to
  RELEASE_BRANCH=main            Branch expected for release commits
  RELEASE_PUSH=1                 Push commit + tag when set to 1
  RELEASE_SKIP_CHECKS=0          Skip local smoke checks when set to 1
  RELEASE_CHECK_COMMAND=...      Override the default local check command

This command:
  1. Validates a clean git worktree on the release branch
  2. Synchronizes release versions across Cargo crates, extension manifest, and plugin config
  3. Runs local smoke checks
  4. Creates chore(release): vX.Y.Z
  5. Creates annotated tag vX.Y.Z
  6. Pushes branch + tag so GitHub Actions can publish the release
EOF
}

if [[ $# -lt 1 ]]; then
  usage
  exit 2
fi

VERSION="$1"
TAG="v$VERSION"
REMOTE="${RELEASE_REMOTE:-origin}"
BRANCH="${RELEASE_BRANCH:-main}"
PUSH="${RELEASE_PUSH:-1}"
SKIP_CHECKS="${RELEASE_SKIP_CHECKS:-0}"
CHECK_COMMAND="${RELEASE_CHECK_COMMAND:-cargo test --workspace && cd extension && if [[ ! -d node_modules ]]; then bun install; fi && bun x vitest}"

python3 - <<PY
from pathlib import Path
import sys

sys.path.insert(0, str(Path("$ROOT_DIR") / "scripts" / "release"))
from release_utils import validate_version

validate_version("$VERSION")
PY

if [[ -n "$(git status --porcelain)" ]]; then
  echo "[ERROR] Release flow requires a clean worktree. Commit or stash your changes first."
  exit 1
fi

CURRENT_BRANCH="$(git rev-parse --abbrev-ref HEAD)"
if [[ "$CURRENT_BRANCH" != "$BRANCH" ]]; then
  echo "[ERROR] Releases must start from '$BRANCH' (current: '$CURRENT_BRANCH')."
  exit 1
fi

if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
  echo "[ERROR] Tag already exists locally: $TAG"
  exit 1
fi

if git ls-remote --tags "$REMOTE" "refs/tags/$TAG" | grep -q .; then
  echo "[ERROR] Tag already exists on $REMOTE: $TAG"
  exit 1
fi

echo "[INFO] Syncing release version surfaces to $VERSION"
mapfile -t VERSION_FILES < <(python3 "$ROOT_DIR/scripts/release/sync_versions.py" --version "$VERSION" --print-files)
if [[ "${#VERSION_FILES[@]}" -eq 0 ]]; then
  echo "[ERROR] Version surfaces already match $VERSION. Pick a new version."
  exit 1
fi
for path in "${VERSION_FILES[@]}"; do
  echo "  - $path"
done

if [[ "$SKIP_CHECKS" != "1" ]]; then
  echo "[INFO] Running local smoke checks"
  bash -lc "$CHECK_COMMAND"
else
  echo "[INFO] RELEASE_SKIP_CHECKS=1, skipping local smoke checks"
fi

echo "[INFO] Rebuilding tracked extension bundles for $TAG"
(
  cd extension
  if [[ ! -d node_modules ]]; then
    bun install
  fi
  export RZN_BUILD_SIGNATURE="$TAG"
  bun run build
)

mapfile -t WORKTREE_LINES < <(git status --porcelain)
for line in "${WORKTREE_LINES[@]}"; do
  path="${line:3}"
  case "$path" in
    Cargo.lock|crates/*/Cargo.toml|extension/package.json|extension/src/manifest.base.json|scripts/plugins/config/rzn-browser.json|extension/dist-chrome/*|extension/dist-firefox/*)
      ;;
    *)
      echo "[ERROR] Release flow found unexpected worktree changes after checks: $path"
      echo "        Keep the release commit boring. Fix or revert that noise first."
      exit 1
      ;;
  esac
done

git add "${VERSION_FILES[@]}"
git add Cargo.lock
git add extension/dist-chrome extension/dist-firefox

git commit -m "chore(release): $TAG"
git tag -a "$TAG" -m "Release $TAG"

if [[ "$PUSH" == "1" ]]; then
  echo "[INFO] Pushing $BRANCH and $TAG to $REMOTE"
  git push "$REMOTE" "HEAD:$BRANCH"
  git push "$REMOTE" "refs/tags/$TAG"
  echo "[OK] Release commit and tag pushed. GitHub Actions will build and publish the release."
else
  echo "[OK] Release commit and tag created locally. Push them when you're ready:"
  echo "  git push $REMOTE HEAD:$BRANCH"
  echo "  git push $REMOTE refs/tags/$TAG"
fi
