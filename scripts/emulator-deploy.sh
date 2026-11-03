#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Install the i486 RPM into the running emulator VM (see docs/emulator.md) and stop any running
# instance so the next launch uses the new build.
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
KEY=${THREESEA_EMU_KEY:-$HOME/SailfishOS/emulator/share/id_ed25519}
OPTS="-i $KEY -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR"
# FLAVOUR=full (default) or lite: which build (RPMS/<flavour>).
DIR=$ROOT/RPMS/${FLAVOUR:-full}
RPM=$(ls -t "$DIR"/harbour-threesea-"${FLAVOUR:-full}"-[0-9]*.i486.rpm | head -1)
scp -P 2223 $OPTS "$RPM" root@127.0.0.1:/tmp/threesea.rpm
ssh -p 2223 $OPTS root@127.0.0.1 \
  'pkill -f "^/usr/bin/harbour-threesea" || true; sleep 1; rpm -Uvh --force --nodeps /tmp/threesea.rpm >/dev/null && rpm -q harbour-threesea'
