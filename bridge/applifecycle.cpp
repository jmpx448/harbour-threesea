// SPDX-License-Identifier: AGPL-3.0-only
#include "applifecycle.h"

#include <QCoreApplication>
#include <QEvent>
#include <QWindow>

AppLifecycle::AppLifecycle(QWindow *window, std::function<bool()> keepRunning, QObject *parent)
    : QObject(parent)
    , m_window(window)
    , m_keepRunning(std::move(keepRunning))
{
    m_window->installEventFilter(this);
}

void AppLifecycle::leaveIfUnwanted()
{
    if (!m_windowOpen && !m_keepRunning()) {
        QCoreApplication::quit();
    }
}

bool AppLifecycle::eventFilter(QObject *watched, QEvent *event)
{
    if (watched == m_window) {
        if (event->type() == QEvent::Show) {
            m_windowOpen = true;
        } else if (event->type() == QEvent::Close) {
            m_windowOpen = false;
        }
    }
    return QObject::eventFilter(watched, event);
}
