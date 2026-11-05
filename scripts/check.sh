#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# The repository's checks in one go (CLAUDE.md §7.7, Phase 0). CI (.github/workflows/ci.yml) runs
# the default set, and libthreema's and the SDK's tests and builds as jobs of their own.
#   private data   no line matching .local/private-patterns (local, never committed: one
#                  extended regex per line, e.g. test credentials) in the tracked files, or with
#                  --staged in what is about to be committed
#   logging        scripts/check-logging.py (CLAUDE.md §7.3)
#   TLS pins       scripts/check-pins.py: the same as upstream's (CLAUDE.md §7.5)
#   format         cargo fmt --check (our crate; libthreema keeps its own style)
#   lints, tests   cargo clippy (no warnings; twice, the emulator's `demo` feature too),
#                  cargo test
#   UI and core    scripts/check-qml.py: the commands the UI sends against the core's, QML
#                  imports against the Jolla Store's list, every tests/ program run by a script
#   notices        THIRD-PARTY-NOTICES.txt is current (scripts/make-notices.py)
#   fuzz targets   they still build (core/fuzz, its own workspace; fuzzing itself: docs/build.md)
#   texts          how many texts await the translation step (scripts/check-qml.py --texts);
#                  with --release a failure: a release has them all
#   advisories     cargo audit, if installed
# --full adds libthreema's own tests (scripts/libthreema-tests.sh), every SDK test
#   (scripts/run-sdk-tests.sh; fails without the SDK image, which scripts/sdk-build.sh builds) and
#   the C ABI under AddressSanitizer (scripts/test-ffi-asan.sh, nightly toolchain).
# --quick: private data, logging, TLS pins and format only (the pre-commit hook, scripts/hooks).
set -eu
cd "$(dirname "$0")/.."
quick=false
full=false
staged=false
release=false
for arg in "$@"; do
    case $arg in
        --quick) quick=true ;;
        --full) full=true ;;
        --staged) staged=true ;;
        --release) release=true ;;
        *) echo "usage: $0 [--quick] [--full] [--staged] [--release]" >&2; exit 2 ;;
    esac
done
export PATH="$HOME/.cargo/bin:$PATH"
[ -x tools/protoc/bin/protoc ] && export PROTOC="$PWD/tools/protoc/bin/protoc"
failed=0
step() { printf '== %s\n' "$1"; }
fail() { echo "FAILED: $1" >&2; failed=1; }
# Runs a command quietly; its output only if it fails.
quiet() {
    what=$1
    shift
    out=$(mktemp)
    if ! "$@" >"$out" 2>&1; then
        tail -n 80 "$out"
        fail "$what"
    fi
    rm -f "$out"
}

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

step "TLS pins"
python3 scripts/check-pins.py || fail "TLS pins"

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

step "UI and core"
python3 -I scripts/check-qml.py || fail "UI and core (scripts/check-qml.py)"

step "licence notices"
# THIRD-PARTY-NOTICES.txt lists every crate in Cargo.lock that the app links.
python3 -I scripts/make-notices.py --check || fail "licence notices (python3 -I scripts/make-notices.py)"

step "fuzz targets"
quiet "fuzz targets (core/fuzz)" env CARGO_TARGET_DIR="$PWD/target/fuzz-check" \
    cargo check -q --manifest-path core/fuzz/Cargo.toml --bins

step "texts"
missing=$(python3 -I scripts/check-qml.py --texts | wc -l)
if [ "$missing" -eq 0 ]; then
    echo "all in translations/"
elif $release; then
    python3 -I scripts/check-qml.py --texts | head -40
    fail "texts (scripts/check-qml.py --texts; scripts/update-translations.sh, then the translations)"
else
    echo "$missing entries await the translation step (scripts/check-qml.py --texts)"
fi

step "advisories"
if command -v cargo-audit >/dev/null 2>&1; then
    cargo audit -q || fail "cargo audit"
else
    echo "cargo-audit not installed: skipped"
fi

if $full; then
    step "libthreema"
    quiet "libthreema tests (scripts/libthreema-tests.sh)" scripts/libthreema-tests.sh
    step "SDK tests"
    scripts/run-sdk-tests.sh || fail "SDK tests (scripts/run-sdk-tests.sh)"
    step "C ABI under AddressSanitizer"
    quiet "C ABI under AddressSanitizer (scripts/test-ffi-asan.sh)" scripts/test-ffi-asan.sh
fi

[ $failed -eq 0 ] && echo "OK" || exit 1
