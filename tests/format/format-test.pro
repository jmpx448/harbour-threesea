# SPDX-License-Identifier: AGPL-3.0-only
# The text helpers of qml/components on the real QML engine, and what StyledText makes of their
# tags. Built and run by scripts/test-format.sh.
TARGET = format-test
QT = core gui qml quick quick-private gui-private
CONFIG += c++14 console
SOURCES += main.cpp
