# SPDX-License-Identifier: AGPL-3.0-only
# The voice recorder's stops: what is sent, kept or thrown away. Built and run by
# scripts/test-voicerecorder.sh.
TARGET = voicerecorder-test
QT = core gui gui-private multimedia
CONFIG += c++14 console
INCLUDEPATH += ../../bridge
SOURCES += main.cpp ../../bridge/voicerecorder.cpp
HEADERS += ../../bridge/voicerecorder.h
