// SPDX-License-Identifier: AGPL-3.0-only
#include "callmanager.h"

#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusPendingCallWatcher>
#include <QDBusPendingReply>
#include <QDateTime>
#include <QDebug>
#include <QStringList>

#include "threesebackend.h"
#ifdef THREESEA_FULL
#include "callengine.h"
#endif

CallManager::CallManager(ThreeseBackend *backend, QObject *parent)
    : QObject(parent)
    , m_backend(backend)
{
#ifdef THREESEA_FULL
    m_engine = new CallEngine(this);
    connect(m_engine, &CallEngine::localDescription, this, &CallManager::onLocalDescription);
    connect(m_engine, &CallEngine::localCandidate, this, &CallManager::onLocalCandidate);
    connect(m_engine, &CallEngine::connected, this, &CallManager::onConnected);
    connect(m_engine, &CallEngine::failed, this, &CallManager::onFailed);
    connect(m_engine, &CallEngine::channelMessage, this, &CallManager::onChannelMessage);
    connect(m_engine, &CallEngine::remoteFrame, this, &CallManager::remoteFrame);
    connect(m_engine, &CallEngine::localFrame, this, &CallManager::localFrame);
#endif
    connect(backend, &ThreeseBackend::coreEvent, this, &CallManager::onCoreEvent);
    m_candidateTimer.setSingleShot(true);
    m_candidateTimer.setInterval(100);
    connect(&m_candidateTimer, &QTimer::timeout, this, &CallManager::sendCandidates);
    m_ringTimer.setSingleShot(true);
    m_ringTimer.setInterval(60000);
    connect(&m_ringTimer, &QTimer::timeout, this,
            [this] { m_backend->request(QStringLiteral("callTimeout"), QVariantMap()); });
    // The end screen stays a moment ("Call ended"), like Jolla's phone app.
    m_endedTimer.setSingleShot(true);
    m_endedTimer.setInterval(2500);
    connect(&m_endedTimer, &QTimer::timeout, this, [this] { setState(QStringLiteral("idle")); });
}

bool CallManager::supported() const
{
    return m_engine != nullptr;
}

void CallManager::setMuted(bool muted)
{
    if (muted == m_muted) {
        return;
    }
    m_muted = muted;
#ifdef THREESEA_FULL
    m_engine->setMuted(muted);
#endif
    // The peer shows it (Android sends its microphone state on the data channel too).
    if (m_state == QLatin1String("active") || m_state == QLatin1String("connecting")) {
        QVariantMap args;
        args.insert(QStringLiteral("device"), QStringLiteral("microphone"));
        args.insert(QStringLiteral("on"), !muted);
        m_backend->request(QStringLiteral("callCapture"), args);
    }
    emit mutedChanged();
}

namespace {

// ngfd lives on the system bus (like libngf-qt, which Jolla's apps use).
QDBusMessage ngfCall(const QString &method)
{
    return QDBusMessage::createMethodCall(QStringLiteral("com.nokia.NonGraphicFeedback1.Backend"),
                                          QStringLiteral("/com/nokia/NonGraphicFeedback1"),
                                          QStringLiteral("com.nokia.NonGraphicFeedback1"), method);
}

} // namespace

// The call's tone through ngfd: incoming, the system's VoIP ringtone (profile tone, vibration,
// volume keys); outgoing while the other side rings, the tone generator's ringback (`ring`).
// At most one at a time.
void CallManager::setTone(Tone tone)
{
    if (tone == m_tone) {
        return;
    }
    m_tone = tone;
    stopTone();
    if (tone == Tone::None) {
        return;
    }
    QDBusMessage play = ngfCall(QStringLiteral("Play"));
    QVariantMap properties;
    if (tone == Tone::Ringback) {
        properties.insert(QStringLiteral("tonegen.pattern"), QStringLiteral("ring"));
    }
    play << (tone == Tone::Ringtone ? QStringLiteral("voip_ringtone") : QStringLiteral("indicator")) << properties;
    auto *watcher = new QDBusPendingCallWatcher(QDBusConnection::systemBus().asyncCall(play), this);
    connect(watcher, &QDBusPendingCallWatcher::finished, this, [this, tone](QDBusPendingCallWatcher *call) {
        QDBusPendingReply<quint32> reply = *call;
        call->deleteLater();
        if (!reply.isValid()) {
            return;
        }
        if (m_tone != tone) { // answered, ended or changed meanwhile
            QDBusMessage stop = ngfCall(QStringLiteral("Stop"));
            stop << reply.value();
            QDBusConnection::systemBus().asyncCall(stop);
            return;
        }
        m_toneId = reply.value();
    });
}

void CallManager::stopTone()
{
    if (m_toneId) {
        QDBusMessage stop = ngfCall(QStringLiteral("Stop"));
        stop << m_toneId;
        QDBusConnection::systemBus().asyncCall(stop);
        m_toneId = 0;
    }
}

void CallManager::setState(const QString &state)
{
    if (state == m_state) {
        return;
    }
    m_state = state;
    setTone(state == QLatin1String("incoming") ? Tone::Ringtone
            : state == QLatin1String("ringing")  ? Tone::Ringback
                                                 : Tone::None);
    if (state == QLatin1String("idle")) {
        m_identity.clear();
        m_result.clear();
        m_remoteOffer.clear();
        m_activeSince = 0;
        setMuted(false);
        setCamera(false);
        m_video = false;
        emit remoteVideoStopped();
        if (m_remoteMuted || m_remoteCamera) {
            m_remoteMuted = false;
            m_remoteCamera = false;
            emit remoteChanged();
        }
    }
    emit stateChanged();
}

void CallManager::withTurnServers(std::function<void(const QVariantMap &)> then)
{
    QVariantMap args;
    // The core decides per call whether it goes only through TURN (unverified peers do).
    args.insert(QStringLiteral("identity"), m_identity);
    m_backend->request(QStringLiteral("callTurnServers"), args,
                       [this, then](bool ok, const QVariantMap &turn, const QString &) {
                           if (ok) {
                               m_relayOnly = turn.value(QStringLiteral("relayOnly")).toBool();
                               then(turn);
                           } else {
                               // No TURN (offline?): the call can't be set up reliably.
                               m_backend->request(QStringLiteral("callHangUp"), QVariantMap());
                               end(QStringLiteral("failed"));
                           }
                       });
}

void CallManager::start(const QString &identity, bool video)
{
    if (!m_engine || (m_state != QLatin1String("idle") && m_state != QLatin1String("ended"))) {
        return;
    }
    emit aboutToTakeMicrophone();
    m_endedTimer.stop();
    m_identity = identity;
    m_outgoing = true;
    m_video = video;
    m_result.clear();
    setState(QStringLiteral("starting"));
#ifdef THREESEA_FULL
    withTurnServers([this](const QVariantMap &turn) {
        if (m_state == QLatin1String("starting")) {
            m_engine->startOffer(turn, turn.value(QStringLiteral("relayOnly")).toBool(), m_video);
        }
    });
#endif
}

void CallManager::accept()
{
    if (m_state != QLatin1String("incoming")) {
        return;
    }
    emit aboutToTakeMicrophone();
    m_ringTimer.stop();
    setState(QStringLiteral("connecting"));
#ifdef THREESEA_FULL
    withTurnServers([this](const QVariantMap &turn) {
        if (m_state == QLatin1String("connecting")) {
            m_engine->startAnswer(m_remoteOffer, turn, turn.value(QStringLiteral("relayOnly")).toBool(), m_video);
        }
    });
#endif
}

void CallManager::reject()
{
    if (m_state == QLatin1String("incoming")) {
        m_backend->request(QStringLiteral("callReject"), QVariantMap());
    }
}

void CallManager::hangUp()
{
    if (m_state == QLatin1String("idle") || m_state == QLatin1String("ended")) {
        return;
    }
    if (m_state == QLatin1String("starting")) {
        // Nothing was sent yet.
#ifdef THREESEA_FULL
        m_engine->stop();
#endif
        end(QStringLiteral("aborted"));
        return;
    }
    m_backend->request(QStringLiteral("callHangUp"), QVariantMap());
}

void CallManager::endCallWithMicrophone()
{
    if (m_state == QLatin1String("incoming")) {
        return;
    }
    hangUp();
#ifdef THREESEA_FULL
    // At once, not when the core's answer comes: nothing may reach this call from now on.
    m_engine->stop();
#endif
}

void CallManager::end(const QString &result)
{
#ifdef THREESEA_FULL
    m_engine->stop();
#endif
    m_ringTimer.stop();
    m_candidateTimer.stop();
    m_candidates.clear();
    m_descriptionSent = false;
    m_result = result;
    setState(QStringLiteral("ended"));
    m_endedTimer.start();
}

void CallManager::onCoreEvent(const QString &name, const QVariantMap &data)
{
    if (!name.startsWith(QLatin1String("call"))) {
        return;
    }
    if (name == QLatin1String("callIncoming")) {
        m_endedTimer.stop();
        m_identity = data.value(QStringLiteral("identity")).toString();
        m_remoteOffer = data.value(QStringLiteral("sdp")).toString();
        m_outgoing = false;
        // The core already said no video if we don't take video calls.
        m_video = data.value(QStringLiteral("video")).toBool();
        m_result.clear();
        setState(QStringLiteral("incoming"));
        m_ringTimer.start();
        emit incoming(m_identity);
    } else if (name == QLatin1String("callRinging")) {
        if (m_state == QLatin1String("calling")) {
            setState(QStringLiteral("ringing"));
        }
    } else if (name == QLatin1String("callAnswer")) {
        m_ringTimer.stop();
        setState(QStringLiteral("connecting"));
#ifdef THREESEA_FULL
        m_engine->setRemoteAnswer(data.value(QStringLiteral("sdp")).toString());
#endif
    } else if (name == QLatin1String("callCandidates")) {
#ifdef THREESEA_FULL
        m_engine->addRemoteCandidates(data.value(QStringLiteral("candidates")).toList());
#endif
    } else if (name == QLatin1String("callChannelSend")) {
#ifdef THREESEA_FULL
        m_engine->sendChannel(QByteArray::fromBase64(data.value(QStringLiteral("data")).toByteArray()));
#endif
    } else if (name == QLatin1String("callVideoParams")) {
#ifdef THREESEA_FULL
        m_engine->setVideoParams(data.value(QStringLiteral("bitrateKbps")).toInt(),
                                 data.value(QStringLiteral("width")).toInt(),
                                 data.value(QStringLiteral("height")).toInt(),
                                 data.value(QStringLiteral("fps")).toInt());
#endif
    } else if (name == QLatin1String("callRemoteCapture")) {
        const bool on = data.value(QStringLiteral("on")).toBool();
        if (data.value(QStringLiteral("device")).toString() == QLatin1String("camera")) {
            m_remoteCamera = on;
            if (!on) {
                emit remoteVideoStopped();
            }
        } else {
            m_remoteMuted = !on;
        }
        emit remoteChanged();
    } else if (name == QLatin1String("callEnded")) {
        end(data.value(QStringLiteral("record")).toMap().value(QStringLiteral("result")).toString());
    }
}

void CallManager::onLocalDescription(const QString &type, const QString &sdp)
{
    QVariantMap args;
    args.insert(QStringLiteral("sdp"), sdp);
    m_descriptionSent = true;
    if (type == QLatin1String("offer")) {
        args.insert(QStringLiteral("identity"), m_identity);
        args.insert(QStringLiteral("video"), m_video);
        m_backend->request(QStringLiteral("callStart"), args, [this](bool ok, const QVariantMap &, const QString &) {
            if (ok) {
                setState(QStringLiteral("calling"));
                m_ringTimer.start();
            } else {
                end(QStringLiteral("failed"));
            }
        });
    } else {
        args.insert(QStringLiteral("video"), m_video);
        m_backend->request(QStringLiteral("callAccept"), args, [this](bool ok, const QVariantMap &, const QString &) {
            if (!ok) {
                hangUp();
            }
        });
    }
    sendCandidates();
}

void CallManager::onLocalCandidate(const QVariantMap &candidate)
{
    m_candidates << candidate;
    if (!m_candidateTimer.isActive()) {
        m_candidateTimer.start();
    }
}

void CallManager::sendCandidates()
{
    // Only after our offer/answer (requests reach the core in order; the peer drops candidates
    // of calls it doesn't know).
    if (!m_descriptionSent || m_candidates.isEmpty()) {
        return;
    }
    // Which kinds were found (host, srflx, relay), never the addresses.
    QStringList kinds;
    for (const QVariant &candidate : m_candidates) {
        kinds << candidate.toMap().value(QStringLiteral("candidate")).toString().section(QLatin1String(" typ "), 1).section(QLatin1Char(' '), 0, 0);
    }
    qInfo() << "call candidates:" << kinds.join(QLatin1Char(' '));
    QVariantMap args;
    args.insert(QStringLiteral("candidates"), m_candidates);
    m_candidates.clear();
    m_backend->request(QStringLiteral("callCandidates"), args);
}

void CallManager::setCamera(bool on)
{
    if (on == m_camera || (on && !m_video)) {
        return;
    }
    m_camera = on;
#ifdef THREESEA_FULL
    m_engine->setCamera(on, m_frontCamera);
#endif
    if (!on) {
        emit localVideoStopped();
    }
    // The peer shows our video only while we say the camera is on (like Android).
    if (m_state != QLatin1String("idle") && m_state != QLatin1String("ended")) {
        QVariantMap args;
        args.insert(QStringLiteral("device"), QStringLiteral("camera"));
        args.insert(QStringLiteral("on"), on);
        m_backend->request(QStringLiteral("callCapture"), args);
    }
    emit cameraChanged();
}

void CallManager::setFrontCamera(bool front)
{
    if (front == m_frontCamera) {
        return;
    }
    m_frontCamera = front;
#ifdef THREESEA_FULL
    // A new camera branch with the other camera.
    if (m_camera) {
        m_engine->setCamera(false, front);
        m_engine->setCamera(true, front);
    }
#endif
    emit cameraChanged();
}

void CallManager::onChannelMessage(const QByteArray &data)
{
    QVariantMap args;
    args.insert(QStringLiteral("data"), QString::fromLatin1(data.toBase64()));
    m_backend->request(QStringLiteral("callChannel"), args);
}

void CallManager::onConnected()
{
    // Outgoing video calls start with the camera on (Android's video call button).
    if (m_video && m_outgoing && !m_camera) {
        setCamera(true);
    }
    QVariantMap args;
    args.insert(QStringLiteral("relayed"), m_relayOnly);
    m_backend->request(QStringLiteral("callConnected"), args);
    if (m_muted) {
        QVariantMap capture;
        capture.insert(QStringLiteral("device"), QStringLiteral("microphone"));
        capture.insert(QStringLiteral("on"), false);
        m_backend->request(QStringLiteral("callCapture"), capture);
    }
    m_activeSince = static_cast<double>(QDateTime::currentMSecsSinceEpoch());
    setState(QStringLiteral("active"));
}

void CallManager::onFailed(const QString &reason)
{
    qWarning() << "call media failed:" << reason; // log-ok: the engine's own codes
    if (m_state == QLatin1String("starting")) {
        end(QStringLiteral("failed"));
    } else {
        m_backend->request(QStringLiteral("callHangUp"), QVariantMap());
    }
}
