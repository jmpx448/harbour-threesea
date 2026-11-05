#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Every scripts/test-*.sh, one after the other, in the Sailfish SDK image (scripts/sdk-build.sh
# builds it): a new test script is run without being added anywhere. scripts/test-callengine.sh
# needs the emulator to run, so here it is only built; scripts/test-ffi-asan.sh runs on the host
# (nightly), from scripts/check.sh --full and its own CI job. Each one's output is shown if it fails.
# Exit 1 if any failed, 2 if the image is missing.
# Usage: scripts/run-sdk-tests.sh [test-name ...]   (e.g. models pages; default: all)
set -u
cd "$(dirname "$0")/.."
IMAGE=harbour-threesea-sdk:${SFOS_VERSION:-5.1.0.11}
if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
    echo "no $IMAGE: scripts/sdk-build.sh builds it" >&2
    exit 2
fi
if [ $# -gt 0 ]; then
    scripts=$(for name in "$@"; do echo "scripts/test-$name.sh"; done)
else
    scripts=$(ls scripts/test-*.sh | grep -v '^scripts/test-ffi-asan.sh$')
fi
log=$(mktemp)
trap 'rm -f "$log"' EXIT
failed=0
for script in $scripts; do
    name=${script#scripts/test-}
    name=${name%.sh}
    case $name in
        callengine) set -- --build-only ;;
        *) set -- ;;
    esac
    if "$script" "$@" >"$log" 2>&1; then
        echo "ok   $name${1:+ (built only)}"
    else
        echo "FAIL $name"
        tail -n 60 "$log"
        failed=1
    fi
done
exit $failed
