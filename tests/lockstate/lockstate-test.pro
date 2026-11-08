# SPDX-License-Identifier: AGPL-3.0-only
# The app lock and the page stack: the real window, call page and import page on the real Silica,
# against a fake backend (Fakes.qml) and plain pages (pages/). Built and run by
# scripts/test-lockstate.sh; nothing of it goes into the package.
TARGET = lockstate-test
QT = core gui gui-private qml quick quick-private
CONFIG += c++14 console
SOURCES += main.cpp
