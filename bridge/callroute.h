// SPDX-License-Identifier: AGPL-3.0-only
#ifndef CALLROUTE_H
#define CALLROUTE_H

#include <QObject>
#include <QString>
#include <QStringList>
#include <QTimer>

// Earpiece or loudspeaker during a call (`callRoute`), through the audio policy's route manager,
// where Jolla's own apps go too: the call's streams keep their role, so the echo canceller stays
// in their path. The manager says at runtime whether the loudspeaker can be chosen. It routes a
// connected headset by itself. Only talks to the bus during a call, so never in Lite.
//
// `speaker` says what the manager confirmed, not what was asked for: the button, and whether a
// call at the ear goes dark, depend on it.
class CallRoute : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool available READ available NOTIFY changed)
    Q_PROPERTY(bool speaker READ speaker WRITE setSpeaker NOTIFY changed)
public:
    explicit CallRoute(QObject *parent = nullptr);
    ~CallRoute() override;

    bool available() const { return m_available; }
    bool speaker() const { return m_speaker; }
    void setSpeaker(bool on);
    // A call runs or not. `preferSpeaker`: it is held away from the ear now (a camera is on). Each
    // time that begins the call goes to the loudspeaker if it would be on the earpiece, unless
    // the user chose in this call.
    void setCall(bool running, bool preferSpeaker);

signals:
    void changed();
    // What the user asked for didn't happen: the sound stays where it was.
    void failed();

protected:
    // The bus, apart from the logic.
    virtual void busListen(bool on);
    virtual void busQuery();
    virtual void busSend(bool on);

    void answeredAllowed(const QStringList &features);
    void answeredEnabled(const QStringList &features);
    void answeredRoute(const QString &device);
    void answeredSend(bool ok, bool on);
    void confirmTimedOut();

protected slots:
    void onFeatureChanged(const QString &name, uint allowed, uint enabled);
    void onRouteChanged(const QString &device, uint type);

private:
    void applyPreference();
    void request(bool on, bool byUser);
    void settle();
    void giveUp();
    void setShown(bool available, bool speaker);

    bool m_running = false;
    bool m_lastPrefer = false;
    bool m_preferSpeaker = false;
    // The user chose: nothing else changes the route of this call any more.
    bool m_chosen = false;
    bool m_available = false;
    bool m_speaker = false;
    // We turned it on, so it is ours to turn off when the call ends.
    bool m_ours = false;
    // What the manager last said of the feature.
    bool m_featureOn = false;
    // A request waits for the manager to confirm it.
    bool m_pending = false;
    bool m_wanted = false;
    bool m_byUser = false;
    QString m_output;
    QString m_outputAtRequest;
    QTimer m_confirm;
};

#endif // CALLROUTE_H
