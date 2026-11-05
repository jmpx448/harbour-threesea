# SPDX-License-Identifier: AGPL-3.0-only
# SecretStore (the database key in Sailfish Secrets) over a daemon the test controls. Built and
# run by scripts/test-secretstore.sh.
TARGET = secretstore-test
QT = core
CONFIG += c++14 console
INCLUDEPATH += include . ../../bridge ../../core/include
SOURCES += main.cpp fakesecrets.cpp ../../bridge/secretstore.cpp
HEADERS += fakesecrets.h ../../bridge/secretstore.h
