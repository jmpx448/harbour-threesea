# SPDX-License-Identifier: AGPL-3.0-only
# The list models' update(): results and row signals; a command's result as QML gets it. Built and run by scripts/test-models.sh.
TARGET = models-test
QT = core qml
CONFIG += c++14 console
INCLUDEPATH += ../../bridge
SOURCES += main.cpp ../../bridge/jsonlistmodel.cpp ../../bridge/contactlistmodel.cpp
HEADERS += ../../bridge/jsonlistmodel.h ../../bridge/contactlistmodel.h ../../bridge/coreresult.h
