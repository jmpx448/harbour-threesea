# SPDX-License-Identifier: AGPL-3.0-only
# Flavours (ADR 0004): Lite (Jolla Store rules) by default; `--with full` adds the background
# service at login and calls. `--with store` leaves out the GitHub update check (ADR 0014) for a
# store's build, which brings its own updates.
%bcond_with full
%bcond_with store
# Emulator builds only (scripts/sdk-build.sh sets it for i486): the demo chats.
%bcond_with demo
%if %{with store}
%define store_config CONFIG+=store
%else
%define store_config %{nil}
%endif
%if %{with demo}
%define demo_config CONFIG+=demo
%else
%define demo_config %{nil}
%endif
Name:       harbour-threesea
Summary:    Unofficial Threema client for Sailfish OS
Version:    0.2.0
Release:    1
License:    AGPLv3
URL:        https://github.com/penttimus/harbour-threesea
Source0:    %{name}-%{version}.tar.bz2
Requires:   sailfishsilica-qt5 >= 0.10.9
# The database key lives in Sailfish Secrets; not every image ships the daemon (the 5.1 emulator
# does not).
Requires:   amber-qml-plugin-mpris
Requires:   sailfishsecretsdaemon
Requires:   sailfishsecretsdaemon-secretsplugins-default
BuildRequires:  pkgconfig(sailfishapp) >= 1.0.2
BuildRequires:  pkgconfig(Qt5Core)
BuildRequires:  pkgconfig(Qt5Qml)
BuildRequires:  pkgconfig(Qt5Quick)
BuildRequires:  pkgconfig(Qt5DBus)
BuildRequires:  pkgconfig(Qt5Multimedia)
BuildRequires:  pkgconfig(sailfishsecrets)
BuildRequires:  pkgconfig(openssl)
BuildRequires:  desktop-file-utils
%if %{with full}
# Calls (ADR 0012): GStreamer's webrtcbin with libnice, DTLS/SRTP, SCTP, Opus and PulseAudio.
BuildRequires:  pkgconfig(gstreamer-1.0)
BuildRequires:  pkgconfig(gstreamer-sdp-1.0)
BuildRequires:  pkgconfig(gstreamer-webrtc-1.0)
BuildRequires:  pkgconfig(gstreamer-app-1.0)
BuildRequires:  pkgconfig(gstreamer-video-1.0)
BuildRequires:  pkgconfig(gstreamer-rtp-1.0)
BuildRequires:  pkgconfig(nice)
# Echo cancellation: third_party/webrtc-audio-processing (BSD), built here and linked statically.
BuildRequires:  meson
BuildRequires:  ninja
BuildRequires:  pkgconfig(absl_base)
Requires:   gstreamer1.0-plugins-base
Requires:   gstreamer1.0-plugins-good
Requires:   gstreamer1.0-plugins-bad
Requires:   libnice
%endif

# Harbour: no shared libs of our own, so nothing to provide/require automatically.
%define __provides_exclude_from ^%{_datadir}/.*$

%description
Native Sailfish OS client for the Threema messenger, built on
Threema's libthreema. Requires a Threema Libre license from the
Threema Shop. Not affiliated with or supported by Threema AG.

%prep
%setup -q -n %{name}-%{version}

%build
# The Rust core is prebuilt by scripts/build-core.sh (see docs/build.md and
# docs/decisions/0002-rust-toolchain.md).
%ifarch aarch64
%define rust_triple aarch64-unknown-linux-gnu
%endif
%ifarch %{arm}
%define rust_triple armv7-unknown-linux-gnueabihf
%endif
%ifarch %{ix86}
%define rust_triple i686-unknown-linux-gnu
%endif
%if %{with full}
# On top of what rpm has exported (%%{optflags}: the optimisation and the target's CPU, which also
# selects SSE on i486 and NEON on armv7hl), for this command alone: %%qmake5 below takes $CXXFLAGS
# for the bridge.
CFLAGS="$CFLAGS -fstack-protector-strong -D_FORTIFY_SOURCE=2" \
CXXFLAGS="$CXXFLAGS -fstack-protector-strong -D_FORTIFY_SOURCE=2" \
meson setup target/wap-build third_party/webrtc-audio-processing --prefix=$PWD/target/wap \
    --libdir=lib --default-library=static --buildtype=release
meson install -C target/wap-build
%qmake5 RUST_TRIPLE=%{rust_triple} CONFIG+=full %{store_config} %{demo_config} WAP_PREFIX=$PWD/target/wap
%else
%qmake5 RUST_TRIPLE=%{rust_triple} %{store_config} %{demo_config}
%endif
%make_build

%install
%qmake5_install
strip %{buildroot}%{_bindir}/%{name}
desktop-file-install --delete-original --dir %{buildroot}%{_datadir}/applications \
    %{buildroot}%{_datadir}/applications/*.desktop
%if %{with full}
# The proximity sensor (voice messages at the ear): Sailjail's Sensors permission is not
# Harbour-allowed, so only the Full flavour asks for it; without it the feature stays off.
sed -i 's/^Permissions=.*/&;Sensors/' \
    %{buildroot}%{_datadir}/applications/%{name}.desktop
# Enabled for every user's session (the app itself does nothing if the user switched
# background running off).
mkdir -p %{buildroot}/usr/lib/systemd/user/post-user-session.target.wants
ln -s ../harbour-threesea.service \
    %{buildroot}/usr/lib/systemd/user/post-user-session.target.wants/harbour-threesea.service
%endif

%if %{with full}
# Lite has no scripts (Harbour restricts them): the app notices itself (bridge/applifecycle.h).
# Full keeps running in the background, so an update used to leave the old version running
# until the user turned that off, closed the app and reopened it. Stop the running instance
# (SIGTERM quits it cleanly) and start the new one in the background for each user who had
# it running; nobody else gets it started. Never fails the installation.
%posttrans
pids=""
uids=""
for proc in /proc/[0-9]*; do
    case "$(readlink "$proc/exe" 2>/dev/null)" in
        /usr/bin/harbour-threesea|"/usr/bin/harbour-threesea (deleted)")
            pids="$pids ${proc#/proc/}"
            uid=$(stat -c %u "$proc" 2>/dev/null) || continue
            case " $uids " in *" $uid "*) ;; *) uids="$uids $uid" ;; esac
            ;;
    esac
done
if [ -n "$pids" ]; then
    kill -TERM $pids 2>/dev/null || :
    for i in $(seq 1 30); do
        alive=""
        for pid in $pids; do [ -d "/proc/$pid" ] && alive="$alive $pid"; done
        [ -z "$alive" ] && break
        sleep 0.5
    done
    [ -n "$alive" ] && kill -KILL $alive 2>/dev/null || :
    for uid in $uids; do
        user=$(id -nu "$uid" 2>/dev/null) || continue
        bus=/run/user/$uid/dbus/user_bus_socket
        [ -S "$bus" ] || continue
        (runuser -u "$user" -- env XDG_RUNTIME_DIR=/run/user/$uid \
            DBUS_SESSION_BUS_ADDRESS=unix:path=$bus \
            /usr/bin/gdbus call --session --timeout 60 --dest org.threesea.harbour-threesea \
            --object-path / --method org.threesea.harbourthreesea.background \
            >/dev/null 2>&1 &) || :
    done
fi
exit 0

# Removing the app stops it too.
%preun
if [ "$1" = 0 ]; then
    for proc in /proc/[0-9]*; do
        case "$(readlink "$proc/exe" 2>/dev/null)" in
            /usr/bin/harbour-threesea|"/usr/bin/harbour-threesea (deleted)")
                kill -TERM "${proc#/proc/}" 2>/dev/null || : ;;
        esac
    done
fi
exit 0
%endif

%files
%defattr(-,root,root,-)
%{_bindir}/%{name}
%{_datadir}/%{name}
%{_datadir}/applications/%{name}.desktop
%{_datadir}/icons/hicolor/*/apps/%{name}.png
%if %{with full}
/usr/lib/systemd/user/harbour-threesea.service
/usr/share/lipstick/notificationcategories/x-threesea.call.conf
/usr/lib/systemd/user/post-user-session.target.wants/harbour-threesea.service
%endif
