// SPDX-License-Identifier: AGPL-3.0-only
#ifndef ENDOFPROCESS_H
#define ENDOFPROCESS_H

#include <QCoreApplication>
#include <QElapsedTimer>
#include <QEvent>
#include <QObject>
#include <QQuickView>
#include <QThreadPool>
#include <QUrl>

#include <functional>

// The event loop has ended and the process goes. Only what was posted to `receiver` is delivered
// until nothing is `underWay` or `ms` have gone: no other events, as nothing else is to start now.
// Says whether it all came.
inline bool deliverUnderWay(QObject *receiver, const std::function<bool()> &underWay, int ms)
{
    QElapsedTimer waited;
    waited.start();
    QCoreApplication::sendPostedEvents(receiver, QEvent::MetaCall);
    while (underWay() && waited.elapsed() < ms) {
        QThreadPool::globalInstance()->waitForDone(20);
        QCoreApplication::sendPostedEvents(receiver, QEvent::MetaCall);
    }
    return !underWay();
}

// In this order: a closing chat page still hands the core its waiting files and its draft, so the
// pages go while the core runs, and the core only stops when it has those. True: all stopped; false
// means something still runs against the backend (see ThreeseBackend::shutdown).
template <typename Backend>
bool endOfProcess(QQuickView &view, Backend &backend)
{
    view.hide();
    backend.prepareToEnd();
    view.setSource(QUrl());
    return backend.shutdown();
}

#endif // ENDOFPROCESS_H
