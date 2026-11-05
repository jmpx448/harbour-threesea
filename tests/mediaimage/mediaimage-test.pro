# SPDX-License-Identifier: AGPL-3.0-only
# PeerImage::decode, what the image provider shows: EXIF rotation and sizes. Built and run by
# scripts/test-mediaimage.sh.
TARGET = mediaimage-test
QT = core gui
CONFIG += c++14 console
INCLUDEPATH += ../../bridge
SOURCES += main.cpp
HEADERS += ../../bridge/peerimage.h
