// SPDX-License-Identifier: AGPL-3.0-only
#ifndef GROUPCALLMANAGER_H
#define GROUPCALLMANAGER_H

#include <QHash>
#include <QImage>
#include <QObject>
#include <QString>
#include <QTimer>
#include <QVariantList>
#include <QVariantMap>

class GroupCallEngine;
class ThreeseBackend;

// Group calls as QML sees them (`groupCalls`, ADR 0012): which groups have a running call (from
// the core's refresh steps) and the call we are in, joining the core's join/data channel
// commands with the media engine (Full only; `supported` is false without it).
class GroupCallManager : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool supported READ supported CONSTANT)
    // idle | joining | connecting | connected | ended
    Q_PROPERTY(QString state READ state NOTIFY stateChanged)
    Q_PROPERTY(QString conversation READ conversation NOTIFY stateChanged)
    // When we got into the call (ms since epoch), for the duration display.
    Q_PROPERTY(double activeSince READ activeSince NOTIFY stateChanged)
    // The other participants: `[{id, identity, nickname, authenticated}]` (identity and nickname
    // once the handshake with them is done).
    Q_PROPERTY(QVariantList participants READ participants NOTIFY participantsChanged)
    // Why the last join failed (error kind), for the UI.
    Q_PROPERTY(QString error READ error NOTIFY stateChanged)
    Q_PROPERTY(bool muted READ muted WRITE setMuted NOTIFY mutedChanged)
    // Our camera (its picture: CallVideo { manager: groupCalls; remote: false }).
    Q_PROPERTY(bool camera READ camera WRITE setCamera NOTIFY cameraChanged)
    Q_PROPERTY(bool frontCamera READ frontCamera WRITE setFrontCamera NOTIFY cameraChanged)
public:
    explicit GroupCallManager(ThreeseBackend *backend, QObject *parent = nullptr);

    bool supported() const { return m_engine != nullptr; }
    QString state() const { return m_state; }
    QString conversation() const { return m_conversation; }
    double activeSince() const { return m_activeSince; }
    QVariantList participants() const { return m_participants; }
    QString error() const { return m_error; }
    bool muted() const { return m_muted; }
    void setMuted(bool muted);
    bool camera() const { return m_camera; }
    void setCamera(bool camera);
    bool frontCamera() const { return m_frontCamera; }
    void setFrontCamera(bool front);

    // A call runs in this group (the chat shows "Join").
    Q_INVOKABLE bool running(const QString &conversation) const { return m_calls.contains(conversation); }
    // The group's running call: `{startedAt, maxParticipants, participants: [{identity, name}]}`
    // (participants from the call state the SFU keeps; empty if it has none yet).
    Q_INVOKABLE QVariantMap call(const QString &conversation) const { return m_calls.value(conversation); }
    // Peek the group's calls now (when its chat opens).
    Q_INVOKABLE void refresh(const QString &conversation);
    // `create`: start a call if none runs.
    Q_INVOKABLE void join(const QString &conversation, bool create = false);
    Q_INVOKABLE void leave();

signals:
    void stateChanged();
    // About to take the microphone: the other kind of call has to let go of it first.
    void aboutToTakeMicrophone();
    void participantsChanged();
    void mutedChanged();
    void cameraChanged();
    // Video for CallVideo items (`participant` set: that participant's).
    void localFrame(const QImage &image);
    void localVideoStopped();
    void participantFrame(quint32 participant, const QImage &image);
    // Some group's running call appeared, changed or ended.
    void runningChanged();

private slots:
    void onCoreEvent(const QString &name, const QVariantMap &data);
    void onChannelMessage(const QByteArray &data);
    void onFailed(const QString &reason);

private:
    void setState(const QString &state);
    // After a call: unmuted and without camera for the next one.
    void resetDevices();

    ThreeseBackend *m_backend;
    GroupCallEngine *m_engine = nullptr;
    QHash<QString, QVariantMap> m_calls;
    QString m_state = QStringLiteral("idle");
    QString m_conversation;
    QString m_error;
    QVariantList m_participants;
    bool m_muted = false;
    bool m_camera = false;
    bool m_frontCamera = true;
    double m_activeSince = 0;
    QTimer m_tick;
};

#endif // GROUPCALLMANAGER_H
