# SPDX-License-Identifier: AGPL-3.0-only
# FileType: what a file to be sent goes as. Built and run by scripts/test-filetype.sh.
TARGET = filetype-test
QT = core
CONFIG += c++14 console
INCLUDEPATH += ../../bridge
SOURCES += main.cpp
HEADERS += ../../bridge/filetype.h
