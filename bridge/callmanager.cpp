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

namespace {

// Nothing at all has come in (speech, silence and control packets all count) for this long: the
// page says so, and then the connection counts as lost.
const int MEDIA_SILENT_NOTICE_SECONDS = 10;
const int MEDIA_SILENT_LIMIT_SECONDS = 30;
// How long the end screen stays: a failure's text takes longer to read.
const int END_SCREEN_MS = 2500;
const int FAILURE_SCREEN_MS = 6000;

} // namespace

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
            [this] { m_backend->request(QStringLiteral("callTimeout"), callArgs()); });
    m_connectTimer.setSingleShot(true);
    m_connectTimer.setInterval(60000);
    connect(&m_connectTimer, &QTimer::timeout, this, [this] { fail(QStringLiteral("connection")); });
    m_mediaTimer.setInterval(1000);
    connect(&m_mediaTimer, &QTimer::timeout, this, &CallManager::checkMedia);
    // The end screen stays a moment ("Call ended"), like Jolla's phone app.
    m_endedTimer.setSingleShot(true);
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
    if (state == QLatin1String("starting") || state == QLatin1String("connecting")) {
        m_connectTimer.start();
    } else {
        m_connectTimer.stop();
    }
    if (state == QLatin1String("idle")) {
        m_identity.clear();
        resetCall();
    }
    emit stateChanged();
}

void CallManager::resetCall()
{
    m_result.clear();
    m_failure.clear();
    m_remoteOffer.clear();
    m_activeSince = 0;
    m_callId = -1;
    setInterrupted(false);
    // Not through the setters: nobody is to be told, the call they would tell is over.
    if (m_muted) {
        m_muted = false;
#ifdef THREESEA_FULL
        m_engine->setMuted(false);
#endif
        emit mutedChanged();
    }
    if (m_camera) {
        m_camera = false;
        emit localVideoStopped();
        emit cameraChanged();
    }
    m_video = false;
    emit remoteVideoStopped();
    if (m_remoteMuted || m_remoteCamera) {
        m_remoteMuted = false;
        m_remoteCamera = false;
        emit remoteChanged();
    }
}

void CallManager::setInterrupted(bool interrupted)
{
    if (interrupted != m_interrupted) {
        m_interrupted = interrupted;
        emit interruptedChanged();
    }
}

void CallManager::stopMedia()
{
#ifdef THREESEA_FULL
    m_engine->stop();
#endif
    m_ringTimer.stop();
    m_connectTimer.stop();
    m_mediaTimer.stop();
    m_candidateTimer.stop();
    m_candidates.clear();
    m_descriptionSent = false;
    ++m_serial;
}

QVariantMap CallManager::callArgs() const
{
    QVariantMap args;
    args.insert(QStringLiteral("callId"), m_callId);
    return args;
}

void CallManager::withTurnServers(std::function<void(const QVariantMap &)> then)
{
    QVariantMap args;
    // The core decides per call whether it goes only through TURN (unverified peers do).
    args.insert(QStringLiteral("identity"), m_identity);
    const unsigned serial = m_serial;
    m_backend->request(QStringLiteral("callTurnServers"), args,
                       [this, then, serial](bool ok, const QVariantMap &turn, const QString &) {
                           if (serial != m_serial) {
                               return;
                           }
                           if (ok) {
                               m_relayOnly = turn.value(QStringLiteral("relayOnly")).toBool();
                               then(turn);
                           } else {
                               // No TURN (offline?): the call can't be set up reliably.
                               fail(QStringLiteral("connection"));
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
    resetCall();
    m_identity = identity;
    m_outgoing = true;
    m_video = video;
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
        m_backend->request(QStringLiteral("callReject"), callArgs());
    }
}

void CallManager::hangUp()
{
    if (m_state == QLatin1String("idle") || m_state == QLatin1String("ended")) {
        return;
    }
    if (m_state == QLatin1String("starting")) {
        // Nothing was sent yet, or the offer is on its way to the core: the reply to it says
        // which call to end (onLocalDescription), so the peer is told right after.
        end(QStringLiteral("aborted"));
        return;
    }
    m_backend->request(QStringLiteral("callHangUp"), callArgs());
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

void CallManager::forget()
{
    stopMedia();
    m_endedTimer.stop();
    setState(QStringLiteral("idle"));
}

void CallManager::end(const QString &result)
{
    stopMedia();
    m_callId = -1;
    setInterrupted(false);
    m_result = result;
    m_endedTimer.setInterval(m_failure.isEmpty() ? END_SCREEN_MS : FAILURE_SCREEN_MS);
    setState(QStringLiteral("ended"));
    m_endedTimer.start();
}

void CallManager::fail(const QString &failure)
{
    if (m_state == QLatin1String("idle") || m_state == QLatin1String("ended")) {
        return;
    }
    if (m_callId >= 0) {
        QVariantMap args = callArgs();
        args.insert(QStringLiteral("reason"), failure);
        m_backend->request(QStringLiteral("callFailed"), args);
    } else if (m_descriptionSent) {
        // The offer is on its way to the core: the reply to it says which call to end.
        m_startFailure = failure;
    }
    m_failure = failure;
    end(QStringLiteral("failed"));
}

void CallManager::endStarted(qint64 callId, const QString &failure)
{
    QVariantMap args;
    args.insert(QStringLiteral("callId"), callId);
    if (failure.isEmpty()) {
        m_backend->request(QStringLiteral("callHangUp"), args);
    } else {
        args.insert(QStringLiteral("reason"), failure);
        m_backend->request(QStringLiteral("callFailed"), args);
    }
}

// Mid-call: libnice (SFOS) doesn't notice a dead path by itself (see GroupCallEngine::sendConsent),
// so the picture and the time would just go on. Off if nothing was ever counted.
void CallManager::checkMedia()
{
#ifdef THREESEA_FULL
    const unsigned packets = m_engine->packetsReceived();
    if (packets != m_lastPackets) {
        m_lastPackets = packets;
        m_silentSeconds = 0;
        setInterrupted(false);
        return;
    }
    if (packets == 0) {
        return;
    }
    ++m_silentSeconds;
    if (m_silentSeconds >= MEDIA_SILENT_LIMIT_SECONDS) {
        qWarning() << "call: nothing received for" << m_silentSeconds << "s";
        fail(QStringLiteral("connection"));
    } else if (m_silentSeconds >= MEDIA_SILENT_NOTICE_SECONDS) {
        setInterrupted(true);
    }
#endif
}

void CallManager::onCoreEvent(const QString &name, const QVariantMap &data)
{
    if (!name.startsWith(QLatin1String("call"))) {
        return;
    }
    if (name == QLatin1String("callIncoming")) {
        // The core has no call while we set one up: the incoming one takes over from that.
        stopMedia();
        m_endedTimer.stop();
        resetCall();
        m_callId = data.value(QStringLiteral("callId")).toLongLong();
        m_identity = data.value(QStringLiteral("identity")).toString();
        m_remoteOffer = data.value(QStringLiteral("sdp")).toString();
        m_outgoing = false;
        // The core already said no video if we don't take video calls.
        m_video = data.value(QStringLiteral("video")).toBool();
        setState(QStringLiteral("incoming"));
        m_ringTimer.start();
        emit incoming(m_identity);
    } else if (name == QLatin1String("callRinging")) {
        if (m_state == QLatin1String("calling")) {
            setState(QStringLiteral("ringing"));
        }
    } else if (name == QLatin1String("callAnswer")) {
        if (m_state != QLatin1String("calling") && m_state != QLatin1String("ringing")) {
            return;
        }
        m_ringTimer.stop();
        // The other side may have answered without video: an audio call then, without our camera.
        const bool video = data.value(QStringLiteral("video")).toBool();
        if (!video) {
            setCamera(false);
        }
        m_video = video;
        setState(QStringLiteral("connecting"));
#ifdef THREESEA_FULL
        m_engine->setRemoteAnswer(data.value(QStringLiteral("sdp")).toString());
#endif
    } else if (name == QLatin1String("callCandidates")) {
#ifdef THREESEA_FULL
        // (The engine keeps what comes before its pipeline for the call; not for the next one.)
        if (m_state != QLatin1String("idle") && m_state != QLatin1String("ended")) {
            m_engine->addRemoteCandidates(data.value(QStringLiteral("candidates")).toList());
        }
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
        // A call we let go of already, or one that never showed (its offer was unusable).
        const QVariantMap record = data.value(QStringLiteral("record")).toMap();
        if (record.value(QStringLiteral("callId")).toLongLong() == m_callId) {
            end(record.value(QStringLiteral("result")).toString());
        }
    }
}

void CallManager::onLocalDescription(const QString &type, const QString &sdp)
{
    // (What the engine queued before it stopped never comes here.)
    const bool offer = type == QLatin1String("offer");
    if (m_state != (offer ? QLatin1String("starting") : QLatin1String("connecting"))) {
        return;
    }
    QVariantMap args;
    args.insert(QStringLiteral("sdp"), sdp);
    m_descriptionSent = true;
    args.insert(QStringLiteral("video"), m_video);
    const unsigned serial = m_serial;
    if (offer) {
        args.insert(QStringLiteral("identity"), m_identity);
        m_backend->request(QStringLiteral("callStart"), args,
                           [this, serial](bool ok, const QVariantMap &result, const QString &kind) {
                               const QString startFailure = m_startFailure;
                               m_startFailure.clear();
                               if (serial != m_serial) {
                                   // Given up meanwhile: now the core's call has an id to end it
                                   // by, and nothing else can be hit.
                                   if (ok) {
                                       endStarted(result.value(QStringLiteral("callId")).toLongLong(),
                                                  startFailure);
                                   }
                                   return;
                               }
                               if (ok) {
                                   m_callId = result.value(QStringLiteral("callId")).toLongLong();
                                   setState(QStringLiteral("calling"));
                                   m_ringTimer.start();
                                   sendCandidates();
                               } else {
                                   // The core took no call: this contact can't be called (the end
                                   // screen says why), or, usually, another call came in
                                   // (`callIncoming` may still be on its way). It has nothing of
                                   // ours to end, and that other call stays untouched.
                                   m_failure = kind.startsWith(QLatin1String("contact"))
                                               ? kind : QStringLiteral("other");
                                   end(QStringLiteral("failed"));
                               }
                           });
    } else {
        args.insert(QStringLiteral("callId"), m_callId);
        m_backend->request(QStringLiteral("callAccept"), args,
                           [this, serial](bool ok, const QVariantMap &, const QString &) {
                               if (!ok && serial == m_serial) {
                                   fail(QStringLiteral("other"));
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
    // of calls it doesn't know), and once the core said which call this is: until `callStart`'s
    // reply the core may have another one, whose caller must not get these addresses.
    if (!m_descriptionSent || m_callId < 0 || m_candidates.isEmpty()) {
        return;
    }
    // Which kinds were found (host, srflx, relay), never the addresses.
    QStringList kinds;
    for (const QVariant &candidate : m_candidates) {
        kinds << candidate.toMap().value(QStringLiteral("candidate")).toString().section(QLatin1String(" typ "), 1).section(QLatin1Char(' '), 0, 0);
    }
    qInfo() << "call candidates:" << kinds.join(QLatin1Char(' '));
    QVariantMap args = callArgs();
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
    if (m_state != QLatin1String("connecting")) {
        return;
    }
    // Outgoing video calls start with the camera on (Android's video call button).
    if (m_video && m_outgoing && !m_camera) {
        setCamera(true);
    }
#ifdef THREESEA_FULL
    // Switched on before the pipeline existed.
    if (m_camera) {
        m_engine->setCamera(true, m_frontCamera);
    }
    m_lastPackets = m_engine->packetsReceived();
    m_silentSeconds = 0;
    m_mediaTimer.start();
#endif
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
    if (reason == QLatin1String("certificate")) {
        fail(reason);
    } else if (reason == QLatin1String("ice") || reason == QLatin1String("connection")) {
        fail(QStringLiteral("connection"));
    } else {
        fail(QStringLiteral("other"));
    }
}
