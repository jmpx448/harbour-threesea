// SPDX-License-Identifier: AGPL-3.0-only
// MediaReader: the reading happens off the caller's thread, the answer on it, and only for the
// read still wanted. Exit 0: passed.
#include "mediareader.h"

#include <QCoreApplication>
#include <QEventLoop>
#include <QSemaphore>
#include <QThread>
#include <QThreadPool>
#include <cstdio>

namespace {

int failures = 0;

void check(bool ok, const char *what)
{
    if (!ok) {
        std::printf("FAILED: %s\n", what);
        ++failures;
    }
}

// Whatever the worker has answered reaches this thread.
void settle(QThreadPool &pool)
{
    pool.waitForDone();
    QCoreApplication::sendPostedEvents();
    QCoreApplication::processEvents(QEventLoop::AllEvents);
}

} // namespace

int main(int argc, char **argv)
{
    QCoreApplication app(argc, argv);
    QThreadPool pool;
    pool.setMaxThreadCount(1);

    {
        // On a worker, the answer here, the bytes taken over as they are.
        MediaReader reader(&pool);
        QThread *readOn = nullptr;
        QThread *answeredOn = nullptr;
        QByteArray taken;
        reader.start([&readOn]() {
            readOn = QThread::currentThread();
            return QByteArray("video");
        }, [&answeredOn, &taken](QByteArray &bytes) {
            answeredOn = QThread::currentThread();
            taken.swap(bytes);
        });
        check(taken.isEmpty(), "not answered in place");
        settle(pool);
        check(readOn && readOn != QThread::currentThread(), "read on a worker");
        check(answeredOn == QThread::currentThread(), "answered on the caller's thread");
        check(taken == "video", "the bytes arrive");
    }
    {
        // Nothing found is an answer too.
        MediaReader reader(&pool);
        int answers = 0;
        bool empty = false;
        reader.start([]() { return QByteArray(); }, [&](QByteArray &bytes) {
            ++answers;
            empty = bytes.isEmpty();
        });
        settle(pool);
        check(answers == 1 && empty, "an empty read is reported");
    }
    {
        // The worker is busy: of what waits, only the newest is read at all.
        MediaReader blocker(&pool);
        QSemaphore started;
        QSemaphore go;
        blocker.start([&]() {
            started.release();
            go.acquire();
            return QByteArray("first");
        }, [](QByteArray &) {});
        started.acquire();

        MediaReader reader(&pool);
        int reads = 0;
        QByteArray taken;
        int answers = 0;
        const MediaReader::Done done = [&](QByteArray &bytes) {
            ++answers;
            taken.swap(bytes);
        };
        reader.start([&reads]() {
            ++reads;
            return QByteArray("old");
        }, done);
        reader.start([&reads]() {
            ++reads;
            return QByteArray("new");
        }, done);
        go.release();
        settle(pool);
        check(reads == 1, "a read given up on takes no memory");
        check(answers == 1 && taken == "new", "only the newest is answered");
    }
    {
        // Cancelled while it reads: it finishes unseen.
        MediaReader reader(&pool);
        QSemaphore started;
        QSemaphore go;
        int answers = 0;
        reader.start([&]() {
            started.release();
            go.acquire();
            return QByteArray("video");
        }, [&](QByteArray &) { ++answers; });
        started.acquire();
        reader.cancel();
        go.release();
        settle(pool);
        check(answers == 0, "a cancelled read isn't answered");
    }
    {
        // The player is gone before the answer: nobody is called.
        int answers = 0;
        {
            MediaReader reader(&pool);
            reader.start([]() { return QByteArray("video"); }, [&](QByteArray &) { ++answers; });
            pool.waitForDone();
        }
        QCoreApplication::sendPostedEvents();
        QCoreApplication::processEvents(QEventLoop::AllEvents);
        check(answers == 0, "a reader that's gone isn't called");
    }
    if (failures == 0) {
        std::printf("mediareader: passed\n");
    }
    return failures == 0 ? 0 : 1;
}
