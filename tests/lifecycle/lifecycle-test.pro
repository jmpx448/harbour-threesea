# SPDX-License-Identifier: AGPL-3.0-only
# When the process ends, and the package changing under it. Built and run by
# scripts/test-lifecycle.sh.
TARGET = lifecycle-test
QT = core gui
CONFIG += c++14 console
INCLUDEPATH += ../../bridge
SOURCES += main.cpp ../../bridge/applifecycle.cpp ../../bridge/installcheck.cpp
HEADERS += ../../bridge/applifecycle.h ../../bridge/installcheck.h
