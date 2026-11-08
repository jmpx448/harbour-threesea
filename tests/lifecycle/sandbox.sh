#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Run by scripts/test-lifecycle.sh inside the SDK target: a packaged file and the binary replaced
# and removed from outside while the check looks on from inside. The binary staying as it was shows
# that the sandbox is really there, and why the check has to look at the other file.
set -eu
FILE=/usr/share/lifecycle-test/qml/main.qml
BINARY=/usr/bin/lifecycle-test
mkdir -p "$(dirname "$FILE")"
echo first > "$FILE"
OUT=/tmp/sandbox.out
ERR=/tmp/sandbox.err
firejail --noprofile --private-bin=lifecycle-test --whitelist=/usr/share/lifecycle-test \
    "$BINARY" --watch "$FILE" "$BINARY" > "$OUT" 2> "$ERR" &
WATCH=$!
sleep 3
cp "$BINARY" "$BINARY.new"
mv "$BINARY.new" "$BINARY"
echo second > "$FILE.new"
mv "$FILE.new" "$FILE"
sleep 1
rm -rf /usr/share/lifecycle-test "$BINARY"
wait "$WATCH" || true
SEEN=$(grep -E '^(current|replaced|removed|binary-)' "$OUT" | tr '\n' ' ')
if [ "$SEEN" != "current replaced removed binary-unchanged " ]; then
    echo "FAILED: from the sandbox: $SEEN"
    cut -c1-200 "$ERR"
    exit 1
fi
echo "sandbox: ok"
