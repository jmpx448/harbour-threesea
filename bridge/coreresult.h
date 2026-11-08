// SPDX-License-Identifier: AGPL-3.0-only
#ifndef CORERESULT_H
#define CORERESULT_H

#include <QJSEngine>
#include <QJSValue>
#include <QJsonObject>
#include <QJsonValue>

// A command's result as a QML callback gets it. Null stays null: the core says "no such thing" that
// way (docs/ffi.md), and an empty object would be truthy and look like a message with nothing in
// it. An error reply has no result at all: an empty object, so that nobody trips over it.
inline QJSValue coreResultForScript(QJSEngine *engine, const QJsonValue &result)
{
    if (result.isNull()) {
        return QJSValue(QJSValue::NullValue);
    }
    return engine->toScriptValue(result.toObject().toVariantMap());
}

#endif // CORERESULT_H
