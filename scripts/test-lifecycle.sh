#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Builds tests/lifecycle with the Sailfish SDK (i486) and runs it there, without a window system
# (sb2, no device needed). Then what the check sees from a sandbox like Sailjail's (firejail with
# the arguments sailjail gives it, in the target as a chroot, hence --privileged): a packaged file
# under /usr/share being replaced and removed. --pid=host because firejail silently runs a program
# without any sandbox if it sees no kernel threads, as in a container's own process namespace.
# No booster or launcher in this.
set -eu
SFOS=${SFOS_VERSION:-5.1.0.11}
IMAGE=harbour-threesea-sdk:$SFOS
ROOT=$(cd "$(dirname "$0")/.." && pwd)
docker run --rm -i --privileged --pid=host -e SFOS="$SFOS" -v "$ROOT:/src:ro" "$IMAGE" sh -euc '
    T=SailfishOS-$SFOS-i486
    B=$HOME/build; mkdir -p $B/tests $B/bridge
    cp -r /src/tests/lifecycle $B/tests/
    cp /src/bridge/applifecycle.* /src/bridge/installcheck.* $B/bridge/
    cd $B/tests/lifecycle && sb2 -t $T qmake >/dev/null && sb2 -t $T make -s >/dev/null 2>&1 || sb2 -t $T make
    QT_QPA_PLATFORM=minimal sb2 -t $T ./lifecycle-test

    R=/srv/mer/targets/$T
    sudo mount -t proc proc $R/proc
    sudo mount -t tmpfs tmpfs $R/tmp
    sudo mount -t sysfs sys $R/sys
    sudo mount --bind /dev $R/dev
    # firejail refuses what it does not find owned by root, and needs a hosts file.
    echo "127.0.0.1 localhost" | sudo tee $R/etc/hosts >/dev/null
    sudo chown -R root:root $R/usr/lib/firejail $R/etc
    sudo cp lifecycle-test $R/usr/bin/lifecycle-test
    sudo cp /src/tests/lifecycle/sandbox.sh $R/tmp/sandbox.sh
    sudo chroot $R sh /tmp/sandbox.sh'
