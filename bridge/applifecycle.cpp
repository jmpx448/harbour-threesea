// SPDX-License-Identifier: AGPL-3.0-only
#include "applifecycle.h"

#include <QCoreApplication>
#include <QEvent>
#include <QGuiApplication>
#include <QWindow>

AppLifecycle::AppLifecycle(QWindow *window, std::function<bool()> keepRunning, const InstallCheck &install, QObject *parent)
    : QObject(parent)
    , m_window(window)
    , m_keepRunning(std::move(keepRunning))
    , m_install(install)
{
    m_window->installEventFilter(this);
    m_tick.setInterval(5 * 60 * 1000);
    connect(&m_tick, &QTimer::timeout, this, &AppLifecycle::checkInstallation);
    m_tick.start();
    // The package manager may still be at it: the new QML still arriving, and at the end of the
    // transaction the Full flavour's script looks for this process to start the new one in its
    // place. Shorter than the wake job's hold on the device (wakeLimit in the window).
    m_settle.setSingleShot(true);
    m_settle.setInterval(30 * 1000);
    connect(&m_settle, &QTimer::timeout, this, &AppLifecycle::settled);
    connect(qGuiApp, &QGuiApplication::applicationStateChanged, this, [this](Qt::ApplicationState state) {
        if (state == Qt::ApplicationActive) {
            checkInstallation();
        }
    });
}

void AppLifecycle::leaveIfUnwanted()
{
    if (m_windowOpen) {
        return;
    }
    const InstallCheck::State state = m_install.state();
    if (state != InstallCheck::Current) {
        end(state, false);
    } else if (!m_keepRunning()) {
        QCoreApplication::quit();
    }
}

bool AppLifecycle::packageChanged() const
{
    return m_install.state() != InstallCheck::Current;
}

void AppLifecycle::checkInstallation()
{
    if (!m_settle.isActive() && packageChanged()) {
        m_settle.start();
    }
}

void AppLifecycle::settled()
{
    const InstallCheck::State state = m_install.state();
    if (state == InstallCheck::Current) {
        return;
    }
    if (!m_windowOpen) {
        end(state, false);
    } else if (state != m_told) {
        qWarning("threesea: the installed package changed (%s)", state == InstallCheck::Removed ? "removed" : "updated");
        m_told = state;
        emit installationChanged(state == InstallCheck::Removed, true, false);
    }
}

bool AppLifecycle::refuseWindow()
{
    if (m_windowOpen) {
        return false;
    }
    const InstallCheck::State state = m_install.state();
    if (state == InstallCheck::Current) {
        return false;
    }
    end(state, true);
    return true;
}

void AppLifecycle::end(InstallCheck::State state, bool windowAsked)
{
    qWarning("threesea: the installed package changed (%s), ending", state == InstallCheck::Removed ? "removed" : "updated");
    m_settle.stop();
    emit installationChanged(state == InstallCheck::Removed, false, windowAsked);
    QCoreApplication::quit();
}

bool AppLifecycle::eventFilter(QObject *watched, QEvent *event)
{
    if (watched == m_window) {
        if (event->type() == QEvent::Show) {
            m_windowOpen = true;
        } else if (event->type() == QEvent::Close) {
            m_windowOpen = false;
            // Whatever Keep running says: nothing stays behind on an old or removed package.
            const InstallCheck::State state = m_install.state();
            if (state != InstallCheck::Current) {
                end(state, false);
            }
        }
    }
    return QObject::eventFilter(watched, event);
}
