// SPDX-License-Identifier: AGPL-3.0-only
#ifndef CALLMANAGER_H
#define CALLMANAGER_H

#include <functional>

#include <QImage>
#include <QObject>
#include <QString>
#include <QTimer>
#include <QVariantList>
#include <QVariantMap>

class CallEngine;
class ThreeseBackend;

// The call as QML sees it (`calls`): connects the core's call commands/events (docs/ffi.md) with
// the media engine (Full only, ADR 0012). Without an engine (Lite) `supported` is false and
// nothing else happens; the core rejects incoming calls itself there.
class CallManager : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool supported READ supported CONSTANT)
    // idle | starting | calling | ringing | incoming | connecting | active | ended
    Q_PROPERTY(QString state READ state NOTIFY stateChanged)
    Q_PROPERTY(QString identity READ identity NOTIFY stateChanged)
    // For `ended`: finished | missed | rejected | busy | unavailable | disabled | failed …
    Q_PROPERTY(QString result READ result NOTIFY stateChanged)
    // For result `failed` on this device: connection | certificate | other, or the core's error
    // kind (contact…) when it refused to call this contact.
    Q_PROPERTY(QString failure READ failure NOTIFY stateChanged)
    // In an active call nothing has come in over the network for a while.
    Q_PROPERTY(bool interrupted READ interrupted NOTIFY interruptedChanged)
    Q_PROPERTY(bool outgoing READ outgoing NOTIFY stateChanged)
    // With a private chat: what is shown outside the app names nobody.
    Q_PROPERTY(bool privateChat READ privateChat NOTIFY stateChanged)
    // Start of the active call (ms since epoch), for the duration display.
    Q_PROPERTY(double activeSince READ activeSince NOTIFY stateChanged)
    Q_PROPERTY(bool muted READ muted WRITE setMuted NOTIFY mutedChanged)
    // The peer muted its microphone / has its camera on (from the call's data channel).
    Q_PROPERTY(bool remoteMuted READ remoteMuted NOTIFY remoteChanged)
    Q_PROPERTY(bool remoteCamera READ remoteCamera NOTIFY remoteChanged)
    // A video call (both sides take video); our camera on/off, front or back.
    Q_PROPERTY(bool video READ video NOTIFY stateChanged)
    Q_PROPERTY(bool camera READ camera WRITE setCamera NOTIFY cameraChanged)
    Q_PROPERTY(bool frontCamera READ frontCamera WRITE setFrontCamera NOTIFY cameraChanged)
public:
    explicit CallManager(ThreeseBackend *backend, QObject *parent = nullptr);

    bool supported() const;
    QString state() const { return m_state; }
    QString identity() const { return m_identity; }
    QString result() const { return m_result; }
    QString failure() const { return m_failure; }
    bool interrupted() const { return m_interrupted; }
    bool outgoing() const { return m_outgoing; }
    bool privateChat() const { return m_privateChat; }
    double activeSince() const { return m_activeSince; }
    bool muted() const { return m_muted; }
    bool remoteMuted() const { return m_remoteMuted; }
    bool remoteCamera() const { return m_remoteCamera; }
    bool video() const { return m_video; }
    bool camera() const { return m_camera; }
    void setCamera(bool on);
    bool frontCamera() const { return m_frontCamera; }
    void setFrontCamera(bool front);
    void setMuted(bool muted);

    // `video`: a video call (the camera starts on, like Android's video call button).
    Q_INVOKABLE void start(const QString &identity, bool video = false);
    Q_INVOKABLE void accept();
    Q_INVOKABLE void reject();
    Q_INVOKABLE void hangUp();
    // The microphone goes to someone else (a group call) or the window closed: end a call that
    // has it or is about to. An incoming call that only rings keeps ringing.
    void endCallWithMicrophone();
    // The ID was removed: the call and its media are gone at once, nobody is left to ask.
    void forget();

signals:
    void stateChanged();
    // About to take the microphone: the other kind of call has to let go of it first.
    void aboutToTakeMicrophone();
    void interruptedChanged();
    void mutedChanged();
    void remoteChanged();
    void cameraChanged();
    // Video frames for CallVideo items; *Stopped: that stream ended (the item clears).
    void remoteFrame(const QImage &image);
    void localFrame(const QImage &image);
    void remoteVideoStopped();
    void localVideoStopped();
    // A call comes in: the UI shows itself and rings.
    void incoming(const QString &identity);

private slots:
    void onCoreEvent(const QString &name, const QVariantMap &data);
    void onLocalDescription(const QString &type, const QString &sdp);
    void onLocalCandidate(const QVariantMap &candidate);
    void sendCandidates();
    void onConnected();
    void onChannelMessage(const QByteArray &data);
    void onFailed(const QString &reason);

private:
    void setState(const QString &state);
    enum class Tone { None, Ringtone, Ringback };
    void setTone(Tone tone);
    void stopTone();
    void end(const QString &result);
    // The call can't go on (`failure`: see the property): the core tells the peer if it knows of
    // the call, and we go to the end screen at once.
    void fail(const QString &failure);
    // The core took an offer this object has given up on since (`failure` empty: hung up).
    void endStarted(qint64 callId, const QString &failure);
    // The arguments naming this call: a request must not hit another call that took its place
    // in the core before this object heard of the end.
    QVariantMap callArgs() const;
    // Engine, timers and what is queued for this call: gone, and late answers to its requests
    // are recognised (m_serial).
    void stopMedia();
    // A new call starts clean: not muted, no camera, nothing known about the peer.
    void resetCall();
    void checkMedia();
    void setInterrupted(bool interrupted);
    // TURN servers, then `then(turn)` (or the call ends).
    void withTurnServers(std::function<void(const QVariantMap &)> then);

    ThreeseBackend *m_backend;
    CallEngine *m_engine = nullptr;
    QString m_state = QStringLiteral("idle");
    QString m_identity;
    QString m_result;
    QString m_failure;
    // Why the call whose offer is on its way to the core failed, for the reply to that offer.
    QString m_startFailure;
    QString m_remoteOffer;
    // The core's id of the call this object follows, -1: none (yet); events of others don't count.
    qint64 m_callId = -1;
    // Changes when a call's media go away; a reply to a request of an earlier value is late.
    unsigned m_serial = 0;
    bool m_outgoing = false;
    bool m_privateChat = false;
    bool m_muted = false;
    bool m_remoteMuted = false;
    bool m_remoteCamera = false;
    bool m_video = false;
    bool m_camera = false;
    bool m_frontCamera = true;
    // This call goes only through TURN (no MAX video then).
    bool m_relayOnly = false;
    double m_activeSince = 0;
    // Our ICE candidates go out in batches (like Android), not one message each.
    QVariantList m_candidates;
    bool m_descriptionSent = false;
    QTimer m_candidateTimer;
    // Unanswered calls end after a minute (Android's ring time), both directions.
    QTimer m_ringTimer;
    // No media in time (Android: a minute): the call failed, it doesn't wait forever.
    QTimer m_connectTimer;
    // Once it runs: is anything still coming in?
    QTimer m_mediaTimer;
    unsigned m_lastPackets = 0;
    int m_silentSeconds = 0;
    bool m_interrupted = false;
    QTimer m_endedTimer;
    Tone m_tone = Tone::None;
    quint32 m_toneId = 0;
};

#endif // CALLMANAGER_H
