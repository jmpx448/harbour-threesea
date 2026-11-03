# SPDX-License-Identifier: AGPL-3.0-only
# Loopback test of the call media engine (two webrtcbin pipelines in one process).
# Built and run by scripts/test-callengine.sh (SDK build, run in the emulator).
TARGET = callengine-test
QT = core gui
CONFIG += c++14 link_pkgconfig console
PKGCONFIG += gstreamer-1.0 gstreamer-sdp-1.0 gstreamer-webrtc-1.0 gstreamer-app-1.0 gstreamer-video-1.0
INCLUDEPATH += ../../bridge
SOURCES += main.cpp ../../bridge/callengine.cpp ../../bridge/dtlscertificate.cpp
HEADERS += ../../bridge/callengine.h ../../bridge/dtlscertificate.h
# As in harbour-threesea.pro (full).
WAP_PC = PKG_CONFIG_PATH=$$WAP_PREFIX/lib/pkgconfig pkg-config
AEC_SOURCES = ../../bridge/aec/echocanceller.cpp
aec.input = AEC_SOURCES
aec.output = ${QMAKE_FILE_BASE}.o
aec.commands = $$QMAKE_CXX -c -std=c++17 -O2 -fPIC $$system($$WAP_PC --cflags webrtc-audio-processing-2) ${QMAKE_FILE_IN} -o ${QMAKE_FILE_OUT}
aec.variable_out = OBJECTS
QMAKE_EXTRA_COMPILERS += aec
LIBS += $$system($$WAP_PC --static --libs webrtc-audio-processing-2) -lcrypto
