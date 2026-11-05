#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Runs the C ABI's tests (core/src/ffi.rs) under AddressSanitizer: a buffer released with another
# length than it was handed out with, or a write past a caller's buffer, is undefined behaviour the
# ordinary tests can't see. Needs the nightly toolchain (development only, docs/build.md).
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
PROTOC=${PROTOC:-$(scripts/fetch-protoc.sh)}
export PROTOC
RUSTFLAGS="-Zsanitizer=address" CARGO_TARGET_DIR="$ROOT/target/asan" \
    cargo +nightly test -q -p threesea-core --lib --target x86_64-unknown-linux-gnu -- ffi:: group_call
