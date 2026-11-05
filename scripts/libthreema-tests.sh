#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# The vendored libthreema's own library tests (its forward secrecy and encoders, our changes to it
# included), in its own workspace with its own Cargo.lock.
set -eu
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
[ -n "${PROTOC:-}" ] || { [ -x tools/protoc/bin/protoc ] && export PROTOC="$PWD/tools/protoc/bin/protoc"; } || true
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target}" exec cargo test -q \
    --manifest-path third_party/libthreema/Cargo.toml -p libthreema --lib --locked
