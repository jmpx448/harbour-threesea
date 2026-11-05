// SPDX-License-Identifier: AGPL-3.0-only
#ifndef APPLIFECYCLE_H
#define APPLIFECYCLE_H

#include <QObject>

#include <functional>

class QWindow;

// When the process ends other than by the user closing its window, always from what holds now.
// Whether a window is up can't be asked of QWindow: it also goes invisible when minimized to its
// cover, so this follows the window's own show and close.
class AppLifecycle : public QObject
{
    Q_OBJECT

public:
    AppLifecycle(QWindow *window, std::function<bool()> keepRunning, QObject *parent);

    // A run that was only for something without a window (the login service's start, a
    // notification's action) has done it: with Keep running off and no window up it ends.
    Q_INVOKABLE void leaveIfUnwanted();

protected:
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    QWindow *m_window;
    std::function<bool()> m_keepRunning;
    bool m_windowOpen = false;
};

#endif // APPLIFECYCLE_H
