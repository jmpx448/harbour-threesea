# SPDX-License-Identifier: AGPL-3.0-only
# MediaReader: reads on a worker, only the newest counts. Built and run by
# scripts/test-mediareader.sh.
TARGET = mediareader-test
QT = core
CONFIG += c++14 console
INCLUDEPATH += ../../bridge
SOURCES += main.cpp ../../bridge/mediareader.cpp
HEADERS += ../../bridge/mediareader.h
