# SPDX-License-Identifier: AGPL-3.0-only
# What a picture carries out of the app: PhotoEdit and FilePreparer. Built and run by
# scripts/test-photoedit.sh.
TARGET = photoedit-test
QT = core gui
CONFIG += c++14 console
INCLUDEPATH += ../../bridge
SOURCES += main.cpp ../../bridge/photoedit.cpp ../../bridge/filepreparer.cpp
HEADERS += ../../bridge/photoedit.h ../../bridge/filepreparer.h ../../bridge/filetype.h
