#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Builds tests/callengine for i486 in the SDK image and runs it in the emulator (as the device
# user, for the PulseAudio session). See docs/emulator.md.
set -eu
SFOS=${SFOS_VERSION:-5.1.0.11}
IMAGE=harbour-threesea-sdk:$SFOS
ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=$ROOT/target/callengine-test
mkdir -p "$OUT"
docker run --rm -i -e SFOS="$SFOS" -v "$ROOT:/src:ro" "$IMAGE" sh -euc '
    T=SailfishOS-$SFOS-i486
    sb2 -t $T -m sdk-install -R zypper --non-interactive in -y gstreamer1.0-devel \
        gstreamer1.0-plugins-base-devel gstreamer1.0-plugins-bad-devel meson ninja abseil-cpp-devel openssl-devel >&2
    B=$HOME/build; mkdir -p $B/tests $B/bridge $B/core $B/third_party
    cp -r /src/tests/callengine $B/tests/; cp -r /src/bridge/callengine.* /src/bridge/dtlscertificate.* /src/bridge/vp8filter.h /src/bridge/aec $B/bridge/
    cp -r /src/core/include $B/core/
    cp -r /src/third_party/webrtc-audio-processing $B/third_party/
    # The rpm i486 optflags enable SSE (the emulator CPU has it); a bare build must ask for it.
    cd $B && CFLAGS=-msse2 CXXFLAGS=-msse2 sb2 -t $T meson setup wap-build third_party/webrtc-audio-processing --prefix=$B/wap \
        --libdir=lib --default-library=static --buildtype=release >&2
    sb2 -t $T meson install -C wap-build >&2
    cd $B/tests/callengine && sb2 -t $T qmake WAP_PREFIX=$B/wap && sb2 -t $T make >&2
    cat callengine-test' > "$OUT/callengine-test"
KEY=${THREESEA_EMU_KEY:-$HOME/SailfishOS/emulator/share/id_ed25519}
OPTS="-i $KEY -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR"
scp -P 2223 $OPTS "$OUT/callengine-test" root@127.0.0.1:/tmp/callengine-test
# THREESEA_TEST_OFFER=<local file>: answer that offer only (see tests/callengine/main.cpp).
if [ -n "${THREESEA_TEST_OFFER:-}" ]; then
  scp -P 2223 $OPTS "$THREESEA_TEST_OFFER" root@127.0.0.1:/tmp/callengine-offer.sdp
  ssh -p 2223 $OPTS root@127.0.0.1 chmod 644 /tmp/callengine-offer.sdp
  REMOTE_OFFER=/tmp/callengine-offer.sdp
fi
# Expanded here (the remote command is built locally), quoted for both remote shells.
ssh -p 2223 $OPTS root@127.0.0.1 "chmod 755 /tmp/callengine-test &&
    su -l defaultuser -c 'XDG_RUNTIME_DIR=/run/user/100000 GST_DEBUG=${GST_DEBUG:-2} \
        THREESEA_TEST_TURN=\"${THREESEA_TEST_TURN:-}\" THREESEA_TEST_OFFER=${REMOTE_OFFER:-} \
        THREESEA_TEST_BAD_FINGERPRINT=${THREESEA_TEST_BAD_FINGERPRINT:-} THREESEA_TEST_MUTED=${THREESEA_TEST_MUTED:-} \
        THREESEA_TEST_VIDEO=${THREESEA_TEST_VIDEO:-} /tmp/callengine-test'"
