#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Import a new upstream libthreema release (docs/decisions/0006-vendored-libthreema.md):
#   1. move the reference submodule to the new tag,
#   2. commit the pristine domain/libthreema of that tag on branch vendor/libthreema,
#   3. merge vendor/libthreema into the current branch (git merges upstream changes with ours).
# Usage: scripts/update-libthreema.sh <upstream tag, e.g. 6.6.0-1300>
set -eu
TAG=${1:?usage: $0 <upstream tag>}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
SUB="$ROOT/third_party/threema-android"
cd "$ROOT"
[ -z "$(git status --porcelain)" ] || { echo "working tree not clean" >&2; exit 1; }

git -C "$SUB" fetch --depth 1 origin tag "$TAG"
git -C "$SUB" checkout -q "$TAG"
COMMIT=$(git -C "$SUB" rev-parse HEAD)

WORK=$(mktemp -d)
trap 'git worktree remove --force "$WORK/wt" 2>/dev/null || true; rm -rf "$WORK"' EXIT
git -C "$SUB" archive "$TAG" domain/libthreema | tar xf - -C "$WORK"
git worktree add -q "$WORK/wt" vendor/libthreema
rm -rf "$WORK/wt/third_party/libthreema"
cp -a "$WORK/domain/libthreema" "$WORK/wt/third_party/libthreema"
git -C "$WORK/wt" add -A third_party/libthreema
git -C "$WORK/wt" commit -q -m "vendor: libthreema from threema-android $TAG ($COMMIT), unmodified"

git add third_party/threema-android
git commit -q -m "Reference submodule: threema-android $TAG"
git merge --no-ff -m "Merge vendor/libthreema $TAG" vendor/libthreema || {
  echo "Resolve the merge conflicts in third_party/libthreema, then commit." >&2
  exit 1
}
echo "Merged libthreema $TAG. Update docs/build.md, run the tests and the libthreema tests."
