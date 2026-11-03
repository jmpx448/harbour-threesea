# SPDX-License-Identifier: AGPL-3.0-only
# GifDecoder: frames off the caller's thread, looping, limits. Built and run by
# scripts/test-gifdecoder.sh.
TARGET = gifdecoder-test
QT = core gui
CONFIG += c++14 console
INCLUDEPATH += ../../bridge
SOURCES += main.cpp ../../bridge/gifdecoder.cpp
HEADERS += ../../bridge/gifdecoder.h
