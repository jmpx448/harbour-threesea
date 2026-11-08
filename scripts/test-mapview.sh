#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# The bundled map renderer (scripts/maplibre) with the app's qml/components/ThreemaMap.qml on a real
# Silica page in the SDK (i486, sb2; no device): Mesa's llvmpipe draws into a tiny Wayland
# compositor (tests/mapview/shotcompositor.c) that keeps the frame. Nothing reaches a real server:
# the target's /etc/hosts sends map.threema.ch and other.example to the test's own HTTPS server
# (tests/mapview/pinserver.py; checked before anything runs), which logs every connection.
#
#   fixture    an offline style: the map draws, with a cache file
#   allowed    the server's key pinned: the style is fetched and drawn
#   wrong-pin  other pins: the TLS handshake fails, no request is sent, nothing drawn
#   other-host a URL outside the origin: not even a connection
#   redirect   the map server redirects elsewhere: not followed
#   no-policy  without the app's variables: nothing fetched at all
#   local-file a file:// or asset:// style: not read either
#
# In every case nothing the renderer logs names a host, a URL or a tile (its records keep only
# severity and event; the allowed style's tiles all fail, so there are such records), and the
# bridge's guard (bridge/maplogfilter.h) withholds a record in MapLibre's own form. A position
# arriving after the map started doesn't move it (MapViewTest.qml).
#
# Fails on any QML warning or a case gone wrong; target/mapview.png is the fixture's frame.
# Usage: scripts/test-mapview.sh   (the frame comes back on the container's stdout)
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
SFOS=${SFOS_VERSION:-5.1.0.11}
LIB=$ROOT/target/maplibre/$SFOS-i486
"$ROOT/scripts/maplibre/build.sh" i486 >&2
docker run --rm -i -v "$ROOT/tests/mapview:/src:ro" -v "$ROOT/qml/components:/components:ro" \
  -v "$ROOT/bridge/maplogfilter.h:/bridge/maplogfilter.h:ro" \
  -v "$LIB:/maplibre:ro" "harbour-threesea-sdk:$SFOS" sh -euc '
    T=SailfishOS-'"$SFOS"'-i486
    R=/srv/mer/targets/$T
    # Software GL for the test only (the device has its own driver).
    sb2 -t $T -m sdk-install -R zypper --non-interactive in mesa-llvmpipe-dri-swrast-driver \
        "pkgconfig(wayland-server)" >&2
    B=$HOME/mapview && mkdir -p $B/components $B/lib && cp /src/* $B/ && cp /components/ThreemaMap.qml $B/components/ && mkdir -p $B/bridge && cp /bridge/maplogfilter.h $B/bridge/
    # Laid out as in the package (the module finds the library through its RUNPATH), inside the
    # home directory, where sb2 shows it to the target'"'"'s programs.
    cp -a /maplibre/core/usr/lib/libQMapLibre.so.3* $B/lib/ && cp -a /maplibre/qml $B/lib/qml
    cd $B
    sb2 -t $T gcc -O1 -o shotcompositor shotcompositor.c $(sb2 -t $T pkg-config --cflags --libs wayland-server) >&2
    sb2 -t $T qmake >&2 && sb2 -t $T make -s >&2

    # The hosts the map may try, all on this container'"'"'s own server; checked first.
    # (sb2 resolves through the container'"'"'s file, the target'"'"'s for good measure.)
    printf "127.0.0.1 map.threema.ch other.example\n::1 map.threema.ch other.example\n" > hosts
    cat hosts >> $R/etc/hosts && sudo sh -c "cat hosts >> /etc/hosts"
    for host in map.threema.ch other.example; do
        addresses=$(sb2 -t $T getent ahosts $host | cut -d" " -f1 | sort -u | tr "\n" " ")
        case "$addresses" in
            "127.0.0.1 "|"::1 "|"127.0.0.1 ::1 ") ;;
            *) echo "$host is not only local: $addresses" >&2; exit 1 ;;
        esac
    done
    # A test CA the target trusts, and the server'"'"'s certificate for both names.
    O="sb2 -t $T openssl"
    $O req -x509 -newkey rsa:2048 -nodes -days 2 -subj /CN=threesea-test-ca -keyout ca.key -out ca.pem \
       -addext basicConstraints=critical,CA:TRUE -addext keyUsage=critical,keyCertSign 2>/dev/null
    $O req -newkey rsa:2048 -nodes -subj /CN=map.threema.ch -keyout server.key -out server.csr 2>/dev/null
    printf "subjectAltName=DNS:map.threema.ch,DNS:other.example\n" > ext
    $O x509 -req -in server.csr -CA ca.pem -CAkey ca.key -CAcreateserial -days 2 -extfile ext -out server.pem 2>/dev/null
    # Trusted where the target'"'"'s libcurl looks: its bundle and hashed directory (the paths the
    # target'"'"'s absolute symlinks lead to; written directly, as from here they would lead into the
    # container'"'"'s own files).
    HASH=$($O x509 -in ca.pem -noout -hash)
    E=$R/etc/pki/ca-trust/extracted/pem
    sudo sh -c "cat ca.pem >> $E/tls-ca-bundle.pem && cp ca.pem $E/directory-hash/$HASH.0"
    $O x509 -in server.pem -pubkey -noout > server.pub
    PIN=sha256//$($O pkey -pubin -in server.pub -outform der | $O dgst -sha256 -binary | base64)
    WRONG=sha256//AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=
    : > server.log
    sudo python3 -I pinserver.py server.pem server.key server-style.json $B/server.log 443 &
    SERVER=$!
    sleep 1

    export XDG_RUNTIME_DIR=$B/run && mkdir -p -m 700 $XDG_RUNTIME_DIR
    failed=0
    # run <case> <seconds> <fixture|url> [cache] -- [VAR=value ...]
    run() {
        name=$1 seconds=$2 style=$3
        shift 3
        cache=
        if [ "$1" != -- ]; then cache=$1; shift; fi
        shift
        rm -f socket
        sb2 -t $T ./shotcompositor $B/$name.ppm $((seconds + 3)) > socket 2>>compositor.log &
        compositor=$!
        while [ ! -s socket ]; do sleep 0.2; done
        : > server.log
        env -u THREESEA_MAP_ORIGIN -u THREESEA_MAP_PINS "$@" WAYLAND_DISPLAY=$(cat socket) \
          QT_QPA_PLATFORM=wayland-egl EGL_PLATFORM=wayland LIBGL_ALWAYS_SOFTWARE=1 \
          QT_WAYLAND_DISABLE_WINDOWDECORATION=1 \
          sb2 -t $T ./mapview-test $B $B/lib/qml $seconds $style $cache > $name.out 2>&1 \
          || { echo "FAIL $name: QML warnings or no window" >&2; cat $name.out >&2; failed=1; }
        wait $compositor 2>/dev/null || true
        cp server.log $name.log
        echo "== $name: server saw: $(tr "\n" "," < $name.log | cut -c1-200)" >&2
        grep "MapLibre" $name.out | sort | uniq -c | head -5 >&2 || true
        # Where a map was looked at stays out of every log line (CLAUDE.md §7.3).
        if grep -nE "threema\.ch|other\.example|https?:|file:|asset:|tiles/|[0-9]+/[0-9]+/[0-9]+|Failed to load|Refused" $name.out >&2; then
            echo "FAIL $name: the log names a place, URL or tile" >&2; failed=1
        fi
    }
    expect() { # <case> <grep -E pattern> <present|absent>
        if grep -qE "$2" $1.log; then found=present; else found=absent; fi
        [ $found = $3 ] || { echo "FAIL $1: \"$2\" $found in the server log" >&2; failed=1; }
    }
    frame() { python3 -I framecheck.py $1.ppm $2 ${3:-} >&2 || failed=1; }

    OK="THREESEA_MAP_ORIGIN=https://map.threema.ch/ THREESEA_MAP_PINS=$PIN"
    STYLE=https://map.threema.ch/styles/test/style.json
    run fixture 15 fixture $B/cache/tiles.db --
    frame fixture drawn; expect fixture . absent
    [ -s $B/cache/tiles.db ] || { echo "FAIL fixture: no cache file" >&2; failed=1; }
    run allowed 10 $STYLE -- $OK
    frame allowed has ff00ff; expect allowed "^http map.threema.ch /styles/test/style.json$" present
    run wrong-pin 10 $STYLE -- THREESEA_MAP_ORIGIN=https://map.threema.ch/ THREESEA_MAP_PINS=$WRONG
    frame wrong-pin lacks ff00ff; expect wrong-pin "^http" absent
    run other-host 10 https://other.example/styles/test/style.json -- $OK
    frame other-host lacks ff00ff; expect other-host . absent
    run redirect 10 https://map.threema.ch/redirect -- $OK
    expect redirect "^http map.threema.ch /redirect$" present; expect redirect "other.example" absent
    frame redirect lacks ff00ff
    run no-policy 10 $STYLE --
    frame no-policy lacks ff00ff; expect no-policy . absent
    run local-file 10 file://$B/server-style.json -- $OK
    frame local-file lacks ff00ff; expect local-file . absent
    run asset 10 asset://server-style.json -- $OK
    frame asset lacks ff00ff; expect asset . absent
    sudo kill $SERVER 2>/dev/null || true
    [ $failed = 0 ] && echo "all map cases passed" >&2
    cat fixture.ppm
    exit $failed' > "$ROOT/target/mapview.ppm"
python3 -I "$ROOT/tests/mapview/ppm2png.py" "$ROOT/target/mapview.ppm" "$ROOT/target/mapview.png"
