# SPDX-License-Identifier: AGPL-3.0-only
# Every page opened once: the real window and pages on the real Silica, against a fake C++ side
# generated from the bridge's headers. Built and run by scripts/test-pages.sh; nothing of it goes
# into the package.
TARGET = pageload-test
QT = core gui gui-private qml quick quick-private
CONFIG += c++14 console
INCLUDEPATH += bridge
SOURCES += main.cpp bridge/jsonlistmodel.cpp bridge/contactlistmodel.cpp bridge/enterkeyfilter.cpp bridge/emojihighlighter.cpp
HEADERS += bridge/jsonlistmodel.h bridge/contactlistmodel.h bridge/enterkeyfilter.h bridge/emojihighlighter.h
