# SPDX-License-Identifier: AGPL-3.0-only
# The bundled map renderer on a Silica page, offline. Built and run by scripts/test-mapview.sh;
# nothing of it goes into the package.
TARGET = mapview-test
QT = core gui qml quick
CONFIG += c++14 console
INCLUDEPATH += bridge
SOURCES += main.cpp
HEADERS += bridge/maplogfilter.h
