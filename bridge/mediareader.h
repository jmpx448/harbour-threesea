// SPDX-License-Identifier: AGPL-3.0-only
#ifndef MEDIAREADER_H
#define MEDIAREADER_H

#include <QAtomicInt>
#include <QByteArray>
#include <QObject>
#include <QRunnable>
#include <QSharedPointer>

#include <functional>

class QThreadPool;

// Reads decrypted media on a worker thread for a player on the GUI thread: a large file takes
// seconds, which the interface shouldn't wait for. One read at a time for each reader: a newer one
// or cancel() ends what's asked before, and those bytes are wiped unseen. What is on its way when
// the reader goes is wiped too.
class MediaReader : public QObject
{
    Q_OBJECT
public:
    using Read = std::function<QByteArray()>;
    // Takes the bytes over (swap them out): what stays in them is wiped afterwards. Empty: `read`
    // found nothing.
    using Done = std::function<void(QByteArray &bytes)>;

    // `read` must stay valid as long as `pool` runs, which the owner of the two sees to.
    explicit MediaReader(QThreadPool *pool, QObject *parent = nullptr);

    void start(const Read &read, const Done &done);
    void cancel();

private:
    QThreadPool *m_pool;
    // Also read by the worker: a read cancelled before its turn takes no memory.
    QSharedPointer<QAtomicInt> m_current;
};

// Internal: the worker's end. An object of its own, so it needs nothing of a reader that's gone.
class MediaReadTask : public QObject, public QRunnable
{
    Q_OBJECT
public:
    MediaReadTask(const MediaReader::Read &read, const QSharedPointer<QByteArray> &result,
                  const QSharedPointer<QAtomicInt> &current, int generation);
    void run() override;

signals:
    void finished();

private:
    MediaReader::Read m_read;
    QSharedPointer<QByteArray> m_result;
    QSharedPointer<QAtomicInt> m_current;
    int m_generation;
};

#endif // MEDIAREADER_H
