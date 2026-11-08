// SPDX-License-Identifier: AGPL-3.0-only
#ifndef APPLIFECYCLE_H
#define APPLIFECYCLE_H

#include "installcheck.h"

#include <QObject>
#include <QTimer>

#include <functional>

class QWindow;

// When the process ends other than by the user closing its window, always from what holds now.
// Whether a window is up can't be asked of QWindow: it also goes invisible when minimized to its
// cover, so this follows the window's own show and close.
//
// It also notices the package changing under the process (InstallCheck): looking on the wake job's
// tick, every few minutes while the device is up, when the app comes to the front, and for every
// call wanting the window. Without a window the process ends then, also right after doing what a
// call without one asked for. With one it stays until that is closed, as the user is in the middle
// of something; pages it loads from now on come from the new package.
class AppLifecycle : public QObject
{
    Q_OBJECT

public:
    AppLifecycle(QWindow *window, std::function<bool()> keepRunning, const InstallCheck &install, QObject *parent);

    // A run that was only for something without a window (the login service's start, a
    // notification's action) has done it: with no window up it ends if Keep running is off or the
    // package has changed.
    Q_INVOKABLE void leaveIfUnwanted();
    Q_INVOKABLE bool packageChanged() const;
    // Has a look: the wake job calls this, whose tick also comes while the device sleeps.
    Q_INVOKABLE void checkInstallation();

    // A call wants the window. True: this process has no window up and its package has changed,
    // so it ends instead, as what it would show is half old and half new. The bus can't start the
    // new version while this one is alive (the launcher refuses a second start), so that takes
    // another tap, and it says so.
    bool refuseWindow();
    void setSettleInterval(int ms) { m_settle.setInterval(ms); }

signals:
    // `removed`: the package is gone, else a new version is there. `windowOpen`: the process stays
    // until the window is closed, else it is about to end. `windowAsked`: it ends instead of
    // showing the window it was just asked for.
    void installationChanged(bool removed, bool windowOpen, bool windowAsked);

protected:
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    QWindow *m_window;
    void settled();
    void end(InstallCheck::State state, bool windowAsked);

    std::function<bool()> m_keepRunning;
    InstallCheck m_install;
    QTimer m_tick;
    QTimer m_settle;
    InstallCheck::State m_told = InstallCheck::Current;
    bool m_windowOpen = false;
};

#endif // APPLIFECYCLE_H
