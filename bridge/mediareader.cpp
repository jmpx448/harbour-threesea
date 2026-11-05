// SPDX-License-Identifier: AGPL-3.0-only
#include "mediareader.h"

#include <QThreadPool>

namespace {

// The only reference to these bytes, so the wipe happens where they lie.
void wipeAndDelete(QByteArray *bytes)
{
    bytes->fill('\0');
    delete bytes;
}

} // namespace

MediaReader::MediaReader(QThreadPool *pool, QObject *parent)
    : QObject(parent)
    , m_pool(pool)
    , m_current(new QAtomicInt(0))
{
}

void MediaReader::start(const Read &read, const Done &done)
{
    const int generation = m_current->fetchAndAddOrdered(1) + 1;
    const QSharedPointer<QByteArray> result(new QByteArray, wipeAndDelete);
    auto *task = new MediaReadTask(read, result, m_current, generation);
    // Queued to this object: dropped, with the bytes, if it is gone by then.
    connect(task, &MediaReadTask::finished, this, [this, done, result, generation]() {
        if (generation == m_current->load()) {
            done(*result);
        }
    }, Qt::QueuedConnection);
    m_pool->start(task);
}

void MediaReader::cancel()
{
    m_current->fetchAndAddOrdered(1);
}

MediaReadTask::MediaReadTask(const MediaReader::Read &read, const QSharedPointer<QByteArray> &result,
                             const QSharedPointer<QAtomicInt> &current, int generation)
    : m_read(read)
    , m_result(result)
    , m_current(current)
    , m_generation(generation)
{
}

void MediaReadTask::run()
{
    if (m_generation == m_current->load()) {
        *m_result = m_read();
    }
    emit finished();
}
