# SPDX-License-Identifier: AGPL-3.0-only
# The tokens of notification actions. Built and run by scripts/test-notification-tokens.sh.
TARGET = notificationtokens-test
QT = core dbus
CONFIG += c++14 console
INCLUDEPATH += ../../bridge
SOURCES += main.cpp ../../bridge/dbusinterface.cpp
HEADERS += ../../bridge/dbusinterface.h
