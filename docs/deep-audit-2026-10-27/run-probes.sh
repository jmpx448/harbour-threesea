#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Offline reproductions and synthetic scaling measurements in a disposable source copy.
# Defect-test PASS means the vulnerable behavior is present, not remediated.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
AUDIT_COPY=$(mktemp -d /tmp/threesea-deep-probes.XXXXXX)
trap 'rm -rf "$AUDIT_COPY"' EXIT HUP INT TERM
cp "$ROOT/Cargo.toml" "$ROOT/Cargo.lock" "$ROOT/rust-toolchain.toml" "$AUDIT_COPY/"
cp -R "$ROOT/core" "$AUDIT_COPY/core"
ln -s "$ROOT/tests" "$AUDIT_COPY/tests"
ln -s "$ROOT/third_party" "$AUDIT_COPY/third_party"
cp "$ROOT/docs/deep-audit-2026-10-27/probes.rs" "$AUDIT_COPY/core/src/deep_audit_probes.rs"
cat >> "$AUDIT_COPY/core/src/app_web.rs" <<'RUST'

#[cfg(test)]
#[path = "deep_audit_probes.rs"]
mod deep_audit_probes;
RUST
cd "$AUDIT_COPY"
if [ -x "$ROOT/tools/protoc/bin/protoc" ]; then
    export PROTOC="$ROOT/tools/protoc/bin/protoc"
fi
CARGO_TARGET_DIR="${AUDIT_TARGET_DIR:-$ROOT/target}" cargo test --workspace --locked --offline \
    deep_audit_probes -- --test-threads=1 --nocapture
