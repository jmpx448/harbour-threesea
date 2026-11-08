# SPDX-License-Identifier: AGPL-3.0-only
# The tokens of notification actions, and those actions waiting for the core. Built and run by
# scripts/test-notification-tokens.sh.
TARGET = notificationtokens-test
QT = core dbus qml
CONFIG += c++14 console
INCLUDEPATH += ../../bridge
DEFINES += NOTIFICATION_ACTIONS_JS=\\\"$$PWD/../../qml/components/NotificationActions.js\\\"
SOURCES += main.cpp ../../bridge/dbusinterface.cpp
HEADERS += ../../bridge/dbusinterface.h
