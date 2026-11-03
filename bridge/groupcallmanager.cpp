// SPDX-License-Identifier: AGPL-3.0-only
#include "groupcallmanager.h"

#include <QDebug>

#include "threesebackend.h"
#ifdef THREESEA_FULL
#include "groupcallengine.h"
#endif

GroupCallManager::GroupCallManager(ThreeseBackend *backend, QObject *parent)
    : QObject(parent)
    , m_backend(backend)
{
#ifdef THREESEA_FULL
    m_engine = new GroupCallEngine(this);
    connect(m_engine, &GroupCallEngine::channelMessage, this, &GroupCallManager::onChannelMessage);
    connect(m_engine, &GroupCallEngine::failed, this, &GroupCallManager::onFailed);
    connect(m_engine, &GroupCallEngine::connected, this, [this] { setState(QStringLiteral("connected")); });
    connect(m_engine, &GroupCallEngine::localFrame, this, &GroupCallManager::localFrame);
    connect(m_engine, &GroupCallEngine::localVideoStopped, this, &GroupCallManager::localVideoStopped);
    connect(m_engine, &GroupCallEngine::participantFrame, this, &GroupCallManager::participantFrame);
    // The core's timers (rekeys, call state updates, announcing a new call) run on this tick.
    m_tick.setInterval(1000);
    connect(&m_tick, &QTimer::timeout, this, [this] { m_backend->request(QStringLiteral("groupCallTick"), QVariantMap()); });
    connect(this, &GroupCallManager::stateChanged, this, [this] {
        if (m_state == QLatin1String("connected")) {
            m_tick.start();
        } else {
            m_tick.stop();
        }
    });
#endif
    connect(backend, &ThreeseBackend::coreEvent, this, &GroupCallManager::onCoreEvent);
}

void GroupCallManager::setState(const QString &state)
{
    if (state == m_state) {
        return;
    }
    m_state = state;
    emit stateChanged();
}

void GroupCallManager::refresh(const QString &conversation)
{
    QVariantMap args;
    args.insert(QStringLiteral("conversation"), conversation);
    m_backend->request(QStringLiteral("groupCallRefresh"), args);
}

void GroupCallManager::join(const QString &conversation, bool create)
{
    if (!m_engine || m_state == QLatin1String("joining") || m_state == QLatin1String("connecting")
        || m_state == QLatin1String("connected")) {
        return;
    }
#ifdef THREESEA_FULL
    m_conversation = conversation;
    m_error.clear();
    m_participants.clear();
    emit participantsChanged();
    setState(QStringLiteral("joining"));
    QVariantMap args;
    args.insert(QStringLiteral("conversation"), conversation);
    args.insert(QStringLiteral("fingerprint"), m_engine->prepare());
    args.insert(QStringLiteral("create"), create);
    m_backend->request(QStringLiteral("groupCallJoin"), args,
                       [this](bool ok, const QVariantMap &result, const QString &kind) {
                           if (m_state != QLatin1String("joining")) {
                               return;
                           }
                           if (!ok) {
                               m_error = kind;
                               setState(QStringLiteral("ended"));
                               setState(QStringLiteral("idle"));
                               return;
                           }
                           setState(QStringLiteral("connecting"));
                           m_engine->start(result.value(QStringLiteral("offer")).toString(),
                                           result.value(QStringLiteral("candidates")).toList());
                       });
#else
    Q_UNUSED(conversation)
    Q_UNUSED(create)
#endif
}

void GroupCallManager::setMuted(bool muted)
{
    if (muted == m_muted) {
        return;
    }
    m_muted = muted;
#ifdef THREESEA_FULL
    m_engine->setMuted(muted);
#endif
    QVariantMap args;
    args.insert(QStringLiteral("muted"), muted);
    m_backend->request(QStringLiteral("groupCallMute"), args);
    emit mutedChanged();
}

void GroupCallManager::setCamera(bool camera)
{
    if (camera == m_camera) {
        return;
    }
    m_camera = camera;
#ifdef THREESEA_FULL
    m_engine->setCamera(camera, m_frontCamera);
#endif
    QVariantMap args;
    args.insert(QStringLiteral("on"), camera);
    m_backend->request(QStringLiteral("groupCallCamera"), args);
    emit cameraChanged();
}

void GroupCallManager::setFrontCamera(bool front)
{
    if (front == m_frontCamera) {
        return;
    }
    m_frontCamera = front;
#ifdef THREESEA_FULL
    m_engine->setCamera(m_camera, front);
#endif
    emit cameraChanged();
}

void GroupCallManager::leave()
{
    if (m_state == QLatin1String("idle")) {
        return;
    }
#ifdef THREESEA_FULL
    m_engine->stop();
#endif
    m_backend->request(QStringLiteral("groupCallLeave"), QVariantMap());
    if (m_muted) {
        m_muted = false;
        emit mutedChanged();
    }
    if (m_camera) {
        m_camera = false;
        emit cameraChanged();
    }
    setState(QStringLiteral("ended"));
    setState(QStringLiteral("idle"));
}

void GroupCallManager::onCoreEvent(const QString &name, const QVariantMap &data)
{
    if (name == QLatin1String("groupCall")) {
        const QString conversation = data.value(QStringLiteral("conversation")).toString();
        const QVariant call = data.value(QStringLiteral("call"));
        if (call.isValid() && !call.isNull()) {
            m_calls.insert(conversation, call.toMap());
        } else if (!m_calls.remove(conversation)) {
            return;
        }
        emit runningChanged();
    } else if (name == QLatin1String("groupCallOffer")) {
#ifdef THREESEA_FULL
        m_engine->updateOffer(data.value(QStringLiteral("sdp")).toString());
#endif
    } else if (name == QLatin1String("groupCallParticipants")) {
        m_participants = data.value(QStringLiteral("participants")).toList();
#ifdef THREESEA_FULL
        QHash<QString, quint32> mids;
        for (const QVariant &participant : m_participants) {
            const QVariantMap map = participant.toMap();
            mids.insert(map.value(QStringLiteral("microphoneMid")).toString(),
                        map.value(QStringLiteral("id")).toUInt());
            mids.insert(map.value(QStringLiteral("cameraMid")).toString(), map.value(QStringLiteral("id")).toUInt());
        }
        m_engine->setMicrophoneMids(mids);
#endif
        emit participantsChanged();
    } else if (name == QLatin1String("groupCallAlone")) {
        leave();
    } else if (name == QLatin1String("groupCallChannelSend")) {
#ifdef THREESEA_FULL
        m_engine->sendChannel(QByteArray::fromBase64(data.value(QStringLiteral("data")).toByteArray()));
#endif
    }
}

void GroupCallManager::onChannelMessage(const QByteArray &data)
{
    QVariantMap args;
    args.insert(QStringLiteral("data"), QString::fromLatin1(data.toBase64()));
    m_backend->request(QStringLiteral("groupCallChannel"), args);
}

void GroupCallManager::onFailed(const QString &reason)
{
    m_error = reason;
    leave();
}
