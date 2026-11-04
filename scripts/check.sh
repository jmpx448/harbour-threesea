#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# The repository's checks in one go (what CI runs; CLAUDE.md §7.7, Phase 0):
#   private data   no line matching .local/private-patterns (local, never committed: one
#                  extended regex per line, e.g. test credentials) in the tracked files, or with
#                  --staged in what is about to be committed
#   logging        scripts/check-logging.py (CLAUDE.md §7.3)
#   format         cargo fmt --check (our crate; libthreema keeps its own style)
#   lints, tests   cargo clippy (no warnings; twice, the emulator's `demo` feature too),
#                  cargo test
#   advisories     cargo audit, if installed
#   libthreema     its library tests, and the list models', GIF decoder's, app lock's and
#                  notification tokens' tests in the SDK
#                  if its image is there (--full only: they take a while)
# --quick: private data, logging and format only (the pre-commit hook, scripts/hooks).
set -eu
cd "$(dirname "$0")/.."
quick=false
full=false
staged=false
for arg in "$@"; do
    case $arg in
        --quick) quick=true ;;
        --full) full=true ;;
        --staged) staged=true ;;
        *) echo "usage: $0 [--quick] [--full] [--staged]" >&2; exit 2 ;;
    esac
done
export PATH="$HOME/.cargo/bin:$PATH"
[ -x tools/protoc/bin/protoc ] && export PROTOC="$PWD/tools/protoc/bin/protoc"
failed=0
step() { printf '== %s\n' "$1"; }
fail() { echo "FAILED: $1" >&2; failed=1; }

step "private data"
patterns=.local/private-patterns
if [ -s "$patterns" ]; then
    # Where, never what: the match itself is what must not be shown around.
    if $staged; then
        found=$(git diff --cached --name-only --diff-filter=ACMR -z \
            | xargs -0 -r -I{} sh -c 'git show ":$1" | grep -q -I -E -f "$2" && echo "$1"' _ {} "$patterns" || true)
    else
        found=$(git grep -l -I -E -f "$patterns" -- . ':!third_party' || true)
    fi
    if [ -n "$found" ]; then
        echo "$found" >&2
        fail "private data (patterns in $patterns)"
    fi
else
    echo "no $patterns: skipped (see the comment at the top of this script)"
fi

step "logging"
python3 scripts/check-logging.py || fail "logging"

step "format"
cargo fmt -p threesea-core -- --check >/dev/null || fail "format (cargo fmt -p threesea-core)"

if $quick; then
    [ $failed -eq 0 ] && echo "OK" || exit 1
    exit 0
fi

step "clippy"
cargo clippy -q -p threesea-core --all-targets -- -D warnings || fail "clippy"
# The emulator's demo data is behind a feature, so it needs its own pass (core/src/demo.rs).
cargo clippy -q -p threesea-core --features demo --all-targets -- -D warnings ||
    fail "clippy (demo)"

step "tests"
log=$(mktemp)
if cargo test -q -p threesea-core --features demo >"$log" 2>&1; then
    grep -E "^test result" "$log" || true
else
    # The failures and their output.
    grep -E -A15 "^failures:|panicked|^error" "$log" | head -80
    fail "tests"
fi
rm -f "$log"

step "advisories"
if command -v cargo-audit >/dev/null 2>&1; then
    cargo audit -q || fail "cargo audit"
else
    echo "cargo-audit not installed: skipped"
fi

if $full; then
    if docker image inspect "harbour-threesea-sdk:${SFOS_VERSION:-5.1.0.11}" >/dev/null 2>&1; then
        step "list models (SDK)"
        scripts/test-models.sh >/dev/null 2>&1 || fail "list models (scripts/test-models.sh)"
        step "GIF decoder (SDK)"
        scripts/test-gifdecoder.sh >/dev/null 2>&1 || fail "GIF decoder (scripts/test-gifdecoder.sh)"
        step "app lock (SDK)"
        scripts/test-lockstate.sh >/dev/null 2>&1 || fail "app lock (scripts/test-lockstate.sh)"
        step "notification tokens (SDK)"
        scripts/test-notification-tokens.sh >/dev/null 2>&1 ||
            fail "notification tokens (scripts/test-notification-tokens.sh)"
        step "voice recorder (SDK)"
        scripts/test-voicerecorder.sh >/dev/null 2>&1 || fail "voice recorder (scripts/test-voicerecorder.sh)"
    fi
    step "libthreema"
    CARGO_TARGET_DIR="$PWD/target" cargo test -q --manifest-path third_party/libthreema/Cargo.toml \
        -p libthreema --lib --locked >/dev/null 2>&1 || fail "libthreema tests"
fi

[ $failed -eq 0 ] && echo "OK" || exit 1
