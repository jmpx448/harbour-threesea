#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Fill the running emulator's app with made-up chats, or take them away again. Works only with
# an emulator build (`qmake CONFIG+=demo` and the core's `demo` feature, both set for i486 by
# scripts/sdk-build.sh); a build for a phone has neither, and the D-Bus method does not exist
# there. See docs/emulator.md and core/src/demo.rs.
#
# Usage: scripts/emulator-demo.sh [add|remove] [messages]
#   messages: how long the demo chat with a long history is (default 400; only with "add")
set -eu
WHAT=${1:-add}
KEY=${THREESEA_EMU_KEY:-$HOME/SailfishOS/emulator/share/id_ed25519}
OPTS="-i $KEY -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR"
case $WHAT in
    add) REMOVE=false ;;
    remove) REMOVE=true ;;
    *) echo "usage: $0 [add|remove]" >&2; exit 2 ;;
esac
ssh -p 2223 $OPTS root@127.0.0.1 "su - defaultuser -c '
    dbus-send --session --print-reply=literal \
      --dest=org.threesea.harbour-threesea / \
      org.threesea.harbourthreesea.demoData boolean:$REMOVE' >/dev/null"
echo "demo data: $WHAT requested (watch the app, or the log for \"demo data\")"
