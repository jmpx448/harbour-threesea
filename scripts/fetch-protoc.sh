#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Fetch a pinned protoc (needed by libthreema's build.rs via prost-build) into tools/protoc.
# Usage: scripts/fetch-protoc.sh   then   export PROTOC="$PWD/tools/protoc/bin/protoc"
set -eu
VERSION=29.3
SHA256=3e866620c5be27664f3d2fa2d656b5f3e09b5152b42f1bedbf427b333e90021a
ROOT=$(cd "$(dirname "$0")/.." && pwd)
DEST="$ROOT/tools/protoc"
[ -x "$DEST/bin/protoc" ] && { echo "$DEST/bin/protoc"; exit 0; }
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
curl -sfLo "$TMP/protoc.zip" \
  "https://github.com/protocolbuffers/protobuf/releases/download/v$VERSION/protoc-$VERSION-linux-x86_64.zip"
echo "$SHA256  $TMP/protoc.zip" | sha256sum -c - >/dev/null
mkdir -p "$DEST"
unzip -q -o "$TMP/protoc.zip" -d "$DEST"
echo "$DEST/bin/protoc"
