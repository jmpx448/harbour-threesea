#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Start the emulator VM headless with a portrait-native 768x1280 screen, like the SDK does
# (docs/emulator.md). The guest's vboxvideo driver only takes the size hint while it
# initializes, and the SDK's GUI frontend is what normally sends it — so we send it repeatedly
# during the first seconds of boot. Width must keep the line pitch 256-byte aligned (720 garbles).
set -eu
VM=${THREESEA_VM:-"Sailfish OS Emulator 5.1.0.11"}
W=768 H=1280
VBoxManage setextradata "$VM" CustomVideoMode1 ${W}x${H}x32
VBoxManage setextradata "$VM" GUI/LastGuestSizeHint $W,$H
VBoxManage setextradata "$VM" GUI/AutoresizeGuest false
VBoxManage startvm "$VM" --type "${THREESEA_VM_TYPE:-headless}"
i=0
while [ $i -lt 15 ]; do
  VBoxManage controlvm "$VM" setvideomodehint $W $H 32 2>/dev/null || true
  timeout 1 tail -f /dev/null || true
  i=$((i + 1))
done
echo "started; ssh -p 2223 -i ~/SailfishOS/emulator/share/id_ed25519 defaultuser@127.0.0.1"
