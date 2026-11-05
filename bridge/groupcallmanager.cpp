// SPDX-License-Identifier: AGPL-3.0-only
#include "groupcallmanager.h"

#include <QStringList>
#include <QDateTime>
#include <QDebug>

#include "threesebackend.h"
#ifdef THREESEA_FULL
#include "groupcallengine.h"
#endif

namespace {

// Nothing at all has come in for this long: the connection counts as lost. Longer than a 1:1
// call's limit: with everyone muted only the data channel's heartbeats (about every 30 s) come.
const int MEDIA_SILENT_LIMIT_SECONDS = 60;

} // namespace

GroupCallManager::GroupCallManager(ThreeseBackend *backend, QObject *parent)
    : QObject(parent)
    , m_backend(backend)
{
#ifdef THREESEA_FULL
    m_engine = new GroupCallEngine(this);
    connect(m_engine, &GroupCallEngine::channelMessage, this, &GroupCallManager::onChannelMessage);
    connect(m_engine, &GroupCallEngine::failed, this, &GroupCallManager::onFailed);
    connect(m_engine, &GroupCallEngine::connected, this, [this] {
        m_activeSince = static_cast<double>(QDateTime::currentMSecsSinceEpoch());
        setState(QStringLiteral("connected"));
    });
    connect(m_engine, &GroupCallEngine::localFrame, this, &GroupCallManager::localFrame);
    connect(m_engine, &GroupCallEngine::localVideoStopped, this, &GroupCallManager::localVideoStopped);
    connect(m_engine, &GroupCallEngine::participantFrame, this, &GroupCallManager::participantFrame);
    // The core's timers (rekeys, call state updates, announcing a new call) run on this tick.
    m_tick.setInterval(1000);
    connect(&m_tick, &QTimer::timeout, this, [this] {
        m_backend->request(QStringLiteral("groupCallTick"), QVariantMap());
        checkMedia();
    });
    m_connectTimer.setSingleShot(true);
    m_connectTimer.setInterval(20000);
    connect(&m_connectTimer, &QTimer::timeout, this, [this] { onFailed(QStringLiteral("timeout")); });
    connect(this, &GroupCallManager::stateChanged, this, [this] {
        if (m_state == QLatin1String("connected")) {
            m_lastPackets = m_engine->packetsReceived();
            m_silentSeconds = 0;
            m_tick.start();
        } else {
            m_tick.stop();
        }
        if (m_state == QLatin1String("connecting")) {
            m_connectTimer.start();
        } else {
            m_connectTimer.stop();
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
    emit aboutToTakeMicrophone();
    m_conversation = conversation;
    m_error.clear();
    m_errorMessage.clear();
    m_participants.clear();
    emit participantsChanged();
    setState(QStringLiteral("joining"));
    QVariantMap args;
    args.insert(QStringLiteral("conversation"), conversation);
    args.insert(QStringLiteral("fingerprint"), m_engine->prepare());
    args.insert(QStringLiteral("create"), create);
    const unsigned serial = ++m_joinSerial;
    m_backend->request(QStringLiteral("groupCallJoin"), args,
                       [this, serial](bool ok, const QVariantMap &result, const QString &kind) {
                           if (serial != m_joinSerial || m_state != QLatin1String("joining")) {
                               // Joined after all with nobody here for it: the core lets go. A newer
                               // join stays out of this, as a leave would end it in the core.
                               if (ok && m_state == QLatin1String("idle")) {
                                   m_backend->request(QStringLiteral("groupCallLeave"),
                                                      QVariantMap());
                               }
                               return;
                           }
                           if (!ok) {
                               m_error = kind;
                               m_errorMessage = result.value(QStringLiteral("message")).toString();
                               // Wipes our copy of the certificate made for this join.
                               m_engine->stop();
                               resetDevices();
                               setState(QStringLiteral("ended"));
                               setState(QStringLiteral("idle"));
                               return;
                           }
                           // Muted before the core knew of this call: it has to say so too.
                           if (m_muted) {
                               QVariantMap mute;
                               mute.insert(QStringLiteral("muted"), true);
                               m_backend->request(QStringLiteral("groupCallMute"), mute);
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
    resetDevices();
    setState(QStringLiteral("ended"));
    setState(QStringLiteral("idle"));
}

void GroupCallManager::forget()
{
    leave();
    if (!m_calls.isEmpty()) {
        m_calls.clear();
        emit runningChanged();
    }
}

void GroupCallManager::resetDevices()
{
    if (m_muted) {
        m_muted = false;
#ifdef THREESEA_FULL
        m_engine->setMuted(false);
#endif
        emit mutedChanged();
    }
    if (m_camera) {
        m_camera = false;
        emit cameraChanged();
    }
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
    } else if (name == QLatin1String("groupCallsLocked")) {
        const QStringList keep = data.value(QStringLiteral("keep")).toStringList();
        bool changed = false;
        for (auto it = m_calls.begin(); it != m_calls.end();) {
            if (keep.contains(it.key())) {
                ++it;
            } else {
                it = m_calls.erase(it);
                changed = true;
            }
        }
        if (changed) {
            emit runningChanged();
        }
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
    } else if (name == QLatin1String("groupCallMicrophoneOff")) {
        // A big call starts muted; the core has already noted that.
        if (!m_muted) {
            m_muted = true;
#ifdef THREESEA_FULL
            m_engine->setMuted(true);
#endif
            emit mutedChanged();
        }
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
    qWarning() << "group call failed:" << reason; // log-ok: the engine's own codes
    // ("media": the engine's microphone or speaker; everything else is about getting connected.)
    m_error = reason == QLatin1String("media") ? QStringLiteral("groupCallAudio")
                                               : QStringLiteral("groupCallConnection");
    m_errorMessage.clear();
    leave();
}

// libnice (SFOS) doesn't notice a dead path by itself, see GroupCallEngine::sendConsent. Off if
// nothing was ever counted.
void GroupCallManager::checkMedia()
{
#ifdef THREESEA_FULL
    const unsigned packets = m_engine->packetsReceived();
    if (packets != m_lastPackets) {
        m_lastPackets = packets;
        m_silentSeconds = 0;
    } else if (packets > 0 && ++m_silentSeconds >= MEDIA_SILENT_LIMIT_SECONDS) {
        onFailed(QStringLiteral("silence"));
    }
#endif
}
