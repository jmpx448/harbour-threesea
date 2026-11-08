# SPDX-License-Identifier: AGPL-3.0-only
# The loudspeaker switch without the bus. Built and run by scripts/test-callroute.sh.
TARGET = callroute-test
QT = core dbus
CONFIG += c++14 console
INCLUDEPATH += ../../bridge
SOURCES += main.cpp ../../bridge/callroute.cpp
HEADERS += ../../bridge/callroute.h
