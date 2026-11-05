# SPDX-License-Identifier: AGPL-3.0-only
TARGET = harbour-threesea

VERSION = 0.2.0
DEFINES += APP_VERSION=\\\"$$VERSION\\\"

# i18n: id-based (qsTrId) like Jolla's apps. The base .ts carries the engineering English
# (//% comments); keep unfinished entries so it becomes the fallback .qm.
CONFIG += sailfishapp sailfishapp_i18n sailfishapp_i18n_idbased sailfishapp_i18n_unfinished c++14 link_pkgconfig
# sailfishapp listed explicitly: setting PKGCONFIG here otherwise drops it from the link line.
PKGCONFIG += sailfishapp sailfishsecrets

# The Rust core is built beforehand by scripts/build-core.sh into
# target/<rust triple>/release/libthreesea_core.a; RUST_TRIPLE is passed by the spec file.
isEmpty(RUST_TRIPLE): RUST_TRIPLE = x86_64-unknown-linux-gnu
CORE_LIB = $$PWD/target/$$RUST_TRIPLE/release/libthreesea_core.a
!exists($$CORE_LIB): error("Missing $$CORE_LIB - run scripts/build-core.sh first")

# Hardening beyond the SDK's optflags (plain -fstack-protector, no RELRO flags): full RELRO for the
# binary, which parses network data and media through native libraries, the stronger stack
# protection, and checked libc calls also where optflags don't reach (the AEC wrapper below). The
# core (scripts/build-core.sh) and the bundled echo canceller (rpm/harbour-threesea.spec) get the same.
HARDENING_FLAGS = -fstack-protector-strong -D_FORTIFY_SOURCE=2
QMAKE_CXXFLAGS += $$HARDENING_FLAGS
QMAKE_LFLAGS += -Wl,-z,relro,-z,now

QT += dbus multimedia network sensors

INCLUDEPATH += $$PWD/core/include
# libcrypto: used by the bundled SQLCipher (system OpenSSL 3, Harbour-allowed).
LIBS += $$CORE_LIB -lcrypto -lpthread -ldl -lm
PRE_TARGETDEPS += $$CORE_LIB

SOURCES += \
    bridge/animatedimage.cpp \
    bridge/applifecycle.cpp \
    bridge/gifdecoder.cpp \
    bridge/callmanager.cpp \
    bridge/callvideo.cpp \
    bridge/groupcallmanager.cpp \
    bridge/contactlistmodel.cpp \
    bridge/dbusinterface.cpp \
    bridge/emojiimageprovider.cpp \
    bridge/enterkeyfilter.cpp \
    bridge/emojihighlighter.cpp \
    bridge/filepreparer.cpp \
    bridge/mediareader.cpp \
    bridge/installcheck.cpp \
    bridge/jsonlistmodel.cpp \
    bridge/main.cpp \
    bridge/mediaimageprovider.cpp \
    bridge/proximitymonitor.cpp \
    bridge/secretstore.cpp \
    bridge/softsound.cpp \
    bridge/threesebackend.cpp \
    bridge/videoplayer.cpp \
    bridge/voiceplayer.cpp \
    bridge/voicerecorder.cpp

HEADERS += \
    bridge/animatedimage.h \
    bridge/applifecycle.h \
    bridge/endofprocess.h \
    bridge/gifdecoder.h \
    bridge/callmanager.h \
    bridge/callvideo.h \
    bridge/groupcallmanager.h \
    bridge/contactlistmodel.h \
    bridge/dbusinterface.h \
    bridge/emojiimageprovider.h \
    bridge/enterkeyfilter.h \
    bridge/emojihighlighter.h \
    bridge/filepreparer.h \
    bridge/mediareader.h \
    bridge/installcheck.h \
    bridge/jsonlistmodel.h \
    bridge/mediaimageprovider.h \
    bridge/peerimage.h \
    bridge/proximitymonitor.h \
    bridge/offlinenetwork.h \
    bridge/coreresult.h \
    bridge/secretstore.h \
    bridge/softsound.h \
    bridge/threesebackend.h \
    bridge/videoplayer.h \
    bridge/voiceplayer.h \
    bridge/voicerecorder.h

DISTFILES += \
    qml/harbour-threesea.qml \
    qml/cover/CoverPage.qml \
    qml/pages/*.qml \
    qml/pages/settings/*.qml \
    qml/components/*.qml \
    qml/components/*.js \
    rpm/harbour-threesea.spec \
    harbour-threesea.desktop

SAILFISHAPP_ICONS = 86x86 108x108 128x128 172x172

# Twemoji sprite sheets (CC-BY 4.0), from Threema Libre via scripts/make-emoji-table.py.
emojis.files = emojis/*.png
emojis.path = /usr/share/$${TARGET}/emojis
INSTALLS += emojis

# Developer tool for the emulator (`qmake CONFIG+=demo`, set by scripts/sdk-build.sh for i486
# only): the made-up chats, asked for over D-Bus (scripts/emulator-demo.sh) so that no build
# for a phone carries anything of them — not a line of QML, not a word in the translations,
# not a property in the binary. The core command behind it is the Cargo feature `demo`.
demo: DEFINES += THREESEA_DEMO

# Builds for GitHub may check there for a newer release (opt-in, ADR 0014); a store's build
# (`CONFIG+=store`) leaves it out.
!store: DEFINES += THREESEA_UPDATE_CHECK

# Full flavour (ADR 0004): `qmake CONFIG+=full`. Adds the systemd user service that starts the
# app in the background at login, and calls; the Lite build (Jolla Store rules) must not contain
# them.
full {
    DEFINES += THREESEA_FULL
    service.files = systemd/harbour-threesea.service
    service.path = /usr/lib/systemd/user
    INSTALLS += service
    # Calls (ADR 0012): the media engine on the system's GStreamer webrtcbin.
    PKGCONFIG += gstreamer-1.0 gstreamer-sdp-1.0 gstreamer-webrtc-1.0 gstreamer-app-1.0 gstreamer-video-1.0 gstreamer-rtp-1.0 nice
    SOURCES += bridge/callengine.cpp bridge/dtlscertificate.cpp bridge/groupcallengine.cpp \
        bridge/earpieceplayer.cpp bridge/videotranscoder.cpp
    HEADERS += bridge/callengine.h bridge/dtlscertificate.h bridge/groupcallengine.h bridge/vp8filter.h bridge/aec/echocanceller.h \
        bridge/earpieceplayer.h bridge/videotranscoder.h
    # Echo cancellation: the bundled webrtc-audio-processing, built statically by the spec file
    # into WAP_PREFIX (meson, third_party/webrtc-audio-processing). Our wrapper is the only file
    # that sees its headers and needs C++17 like the library (abseil); the app stays C++14.
    isEmpty(WAP_PREFIX): WAP_PREFIX = $$PWD/target/wap
    WAP_PC = PKG_CONFIG_PATH=$$WAP_PREFIX/lib/pkgconfig pkg-config
    WAP_CFLAGS = $$system($$WAP_PC --cflags webrtc-audio-processing-2)
    isEmpty(WAP_CFLAGS): error("webrtc-audio-processing missing in $$WAP_PREFIX (see rpm/harbour-threesea.spec)")
    AEC_SOURCES = bridge/aec/echocanceller.cpp
    aec.input = AEC_SOURCES
    aec.output = ${QMAKE_FILE_BASE}.o
    aec.commands = $$QMAKE_CXX -c -std=c++17 -O2 -fPIC $$HARDENING_FLAGS $$WAP_CFLAGS ${QMAKE_FILE_IN} -o ${QMAKE_FILE_OUT}
    aec.variable_out = OBJECTS
    QMAKE_EXTRA_COMPILERS += aec
    LIBS += $$system($$WAP_PC --static --libs webrtc-audio-processing-2)
}
DISTFILES += bridge/callengine.cpp bridge/callengine.h bridge/dtlscertificate.cpp bridge/dtlscertificate.h \
    bridge/groupcallengine.cpp bridge/groupcallengine.h bridge/vp8filter.h bridge/aec/echocanceller.cpp bridge/aec/echocanceller.h \
    bridge/earpieceplayer.cpp bridge/earpieceplayer.h bridge/videotranscoder.cpp bridge/videotranscoder.h
DISTFILES += systemd/harbour-threesea.service

TRANSLATIONS += translations/harbour-threesea.ts \
    translations/harbour-threesea-de.ts \
    translations/harbour-threesea-fr.ts
