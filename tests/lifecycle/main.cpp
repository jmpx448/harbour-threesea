// SPDX-License-Identifier: AGPL-3.0-only
// When the process ends by itself: a run nobody wants any more, and the package changing under it.
// Exit 0: passed.
#include "applifecycle.h"
#include "installcheck.h"

#include <QFile>
#include <QGuiApplication>
#include <QTemporaryDir>
#include <QThread>
#include <QTimer>
#include <QWindow>
#include <cstdio>
#include <functional>

namespace {

int failures = 0;

void check(bool ok, const char *what)
{
    if (!ok) {
        std::printf("FAILED: %s\n", what);
        ++failures;
    }
}

void writeFile(const QString &path, const char *content = "packaged")
{
    QFile file(path);
    file.open(QIODevice::WriteOnly);
    file.write(content);
}

// Replaces it the way a package manager does: a new file renamed over the old one, whose inode
// stays taken as long as the running process has it.
void replaceFile(const QString &path)
{
    const QString fresh = path + QStringLiteral(".new");
    writeFile(fresh);
    ::rename(fresh.toLocal8Bit().constData(), path.toLocal8Bit().constData());
}

// Whether the application quits after `action`, within `wait` ms.
bool quits(const std::function<void()> &action, int wait = 50)
{
    bool timedOut = false;
    QTimer::singleShot(0, action);
    QTimer::singleShot(wait, qApp, [&timedOut]() {
        timedOut = true;
        QCoreApplication::quit();
    });
    QCoreApplication::exec();
    return !timedOut;
}

struct Told {
    int times = 0;
    bool removed = false;
    bool windowOpen = false;
    bool windowAsked = false;
};

void listen(AppLifecycle &lifecycle, Told &told)
{
    QObject::connect(&lifecycle, &AppLifecycle::installationChanged, [&told](bool removed, bool windowOpen, bool asked) {
        ++told.times;
        told.removed = removed;
        told.windowOpen = windowOpen;
        told.windowAsked = asked;
    });
}

void testInstallCheck(const QString &dir)
{
    const QString path = dir + QStringLiteral("/check");
    writeFile(path, "first");
    const InstallCheck install(path.toLocal8Bit());
    check(install.state() == InstallCheck::Current, "the file it was started from");
    writeFile(path, "first and more");
    check(install.state() == InstallCheck::Replaced, "written over in place");

    const InstallCheck again(path.toLocal8Bit());
    replaceFile(path);
    check(again.state() == InstallCheck::Replaced, "another file at the path");
    QFile::remove(path);
    check(again.state() == InstallCheck::Removed, "nothing at the path");

    // What can't be looked at from the start says nothing, and ends nothing.
    const InstallCheck unknown(QByteArray(dir.toLocal8Bit() + "/never-there"));
    check(unknown.state() == InstallCheck::Current, "a path that was never there");
}

void testLeaving(const QString &dir)
{
    const QString path = dir + QStringLiteral("/leaving");
    writeFile(path);
    bool keepRunning = false;
    QWindow window;
    AppLifecycle lifecycle(&window, [&keepRunning]() { return keepRunning; }, InstallCheck(path.toLocal8Bit()), nullptr);

    const auto leave = [&lifecycle]() { lifecycle.leaveIfUnwanted(); };
    check(quits(leave), "no window, Keep running off: it goes");
    keepRunning = true;
    check(!quits(leave), "Keep running on, as it is now and not at the start: it stays");
    keepRunning = false;
    window.show();
    check(!quits(leave), "a window up: it stays");
    // Minimized to its cover: not closed.
    window.hide();
    check(!quits(leave), "hidden is not closed");
    window.show();
    window.close();
    check(quits(leave), "closed: it goes");
}

void testPackageChanging(const QString &dir)
{
    const QString path = dir + QStringLiteral("/changing");
    writeFile(path);
    QWindow window;
    AppLifecycle lifecycle(&window, []() { return true; }, InstallCheck(path.toLocal8Bit()), nullptr);
    lifecycle.setSettleInterval(1);
    Told told;
    listen(lifecycle, told);
    const auto look = [&lifecycle]() { lifecycle.checkInstallation(); };

    check(!quits(look), "nothing changed: it stays");
    check(told.times == 0, "and says nothing");
    check(!lifecycle.refuseWindow(), "and shows its window when asked");

    // With a window up it stays, and says so once.
    window.show();
    replaceFile(path);
    check(!quits(look), "updated with a window up: it stays");
    check(told.times == 1 && !told.removed && told.windowOpen, "and says that it should be closed");
    check(!quits(look), "looking again");
    check(told.times == 1, "says it once");
    check(!lifecycle.refuseWindow(), "the window that is up goes on answering");

    // Closing it then ends it, whatever Keep running says.
    check(quits([&window]() { window.close(); }), "closed after the update: it goes");
    check(told.times == 2 && !told.removed && !told.windowOpen, "and says that it ended");

    // A change that is undone while the package manager is at it says nothing.
    QWindow other;
    const QString backPath = dir + QStringLiteral("/back");
    writeFile(backPath);
    AppLifecycle undone(&other, []() { return true; }, InstallCheck(backPath.toLocal8Bit()), nullptr);
    undone.setSettleInterval(20);
    const QString aside = backPath + QStringLiteral(".aside");
    ::rename(backPath.toLocal8Bit().constData(), aside.toLocal8Bit().constData());
    bool saidUndone = false;
    QObject::connect(&undone, &AppLifecycle::installationChanged, [&saidUndone]() { saidUndone = true; });
    check(!quits([&]() {
        undone.checkInstallation();
        ::rename(aside.toLocal8Bit().constData(), backPath.toLocal8Bit().constData());
    }), "back before it settled: it stays");
    check(!saidUndone, "and says nothing");
}

void testNoWindow(const QString &dir)
{
    const QString path = dir + QStringLiteral("/nowindow");
    writeFile(path);
    QWindow window;
    AppLifecycle lifecycle(&window, []() { return true; }, InstallCheck(path.toLocal8Bit()), nullptr);
    lifecycle.setSettleInterval(1);
    Told told;
    listen(lifecycle, told);

    replaceFile(path);
    check(quits([&lifecycle]() { lifecycle.checkInstallation(); }), "updated without a window: it goes");
    check(told.times == 1 && !told.removed && !told.windowOpen, "and says so");

    QFile::remove(path);
    check(quits([&lifecycle]() { lifecycle.checkInstallation(); }), "removed without a window: it goes");
    check(told.times == 2 && told.removed, "and says that");
}

void testAskedForTheWindow(const QString &dir)
{
    const QString path = dir + QStringLiteral("/calls");
    writeFile(path);
    QWindow window;
    AppLifecycle lifecycle(&window, []() { return true; }, InstallCheck(path.toLocal8Bit()), nullptr);
    Told told;
    listen(lifecycle, told);

    replaceFile(path);
    bool refused = false;
    check(quits([&]() { refused = lifecycle.refuseWindow(); }), "asked for the window after an update: it ends");
    check(refused, "instead of showing old and new together");
    check(told.times == 1 && !told.removed && told.windowAsked, "and says how to get there");

    QFile::remove(path);
    check(quits([&]() { refused = lifecycle.refuseWindow(); }), "after a removal too");
    check(told.times == 2 && told.removed, "with what a removal asks for");
}

void testDoneAfterAnUpdate(const QString &dir)
{
    const QString path = dir + QStringLiteral("/done");
    writeFile(path);
    QWindow window;
    AppLifecycle lifecycle(&window, []() { return true; }, InstallCheck(path.toLocal8Bit()), nullptr);
    Told told;
    listen(lifecycle, told);
    check(!lifecycle.packageChanged(), "nothing changed yet");
    check(!quits([&lifecycle]() { lifecycle.leaveIfUnwanted(); }), "Keep running on: it stays");

    replaceFile(path);
    check(lifecycle.packageChanged(), "changed");
    window.show();
    check(!quits([&lifecycle]() { lifecycle.leaveIfUnwanted(); }), "a window up: it stays regardless");
    check(told.times == 0, "and has nothing to say from here");
    // Closing asks to quit too, with no loop running here to end. What matters is the next call.
    window.close();
    check(quits([&lifecycle]() { lifecycle.leaveIfUnwanted(); }), "its action done, no window: it goes");
    check(!told.windowAsked, "nobody asked for a window");
}

// Says each state as it comes, and ends with the removal, saying then what became of `binary`.
int watch(const char *path, const char *binary)
{
    const InstallCheck install{QByteArray(path)};
    const InstallCheck own{QByteArray(binary)};
    InstallCheck::State last = InstallCheck::Current;
    std::puts("current");
    std::fflush(stdout);
    for (int i = 0; i < 300; ++i) {
        const InstallCheck::State state = install.state();
        if (state != last) {
            last = state;
            std::puts(state == InstallCheck::Removed ? "removed" : "replaced");
            std::fflush(stdout);
            if (state == InstallCheck::Removed) {
                std::puts(own.state() == InstallCheck::Current ? "binary-unchanged" : "binary-changed");
                return 0;
            }
        }
        QThread::msleep(100);
    }
    return 1;
}

} // namespace

int main(int argc, char **argv)
{
    // For scripts/test-lifecycle.sh, in a sandbox like the app's: what the check sees from there.
    if (argc == 4 && QByteArray(argv[1]) == "--watch") {
        return watch(argv[2], argv[3]);
    }
    QGuiApplication app(argc, argv);
    QGuiApplication::setQuitOnLastWindowClosed(false);
    QTemporaryDir dir;
    testInstallCheck(dir.path());
    testLeaving(dir.path());
    testPackageChanging(dir.path());
    testNoWindow(dir.path());
    testAskedForTheWindow(dir.path());
    testDoneAfterAnUpdate(dir.path());
    if (failures == 0) {
        std::printf("lifecycle: ok\n");
    }
    return failures == 0 ? 0 : 1;
}
