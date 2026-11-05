// SPDX-License-Identifier: AGPL-3.0-only
#ifndef CALLROUTE_H
#define CALLROUTE_H

#include <QObject>
#include <QString>

// Earpiece or loudspeaker during a call (`callRoute`), through the audio policy's route manager,
// where Jolla's own apps go too: the call's streams keep their role, so the echo canceller stays
// in their path. The manager says at runtime whether the loudspeaker can be chosen. It routes a
// connected headset by itself. Only talks to the bus during a call, so never in Lite.
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
    // A call began or ended. `preferSpeaker`: it is held away from the ear, so it starts on the
    // loudspeaker if it would otherwise go to the earpiece.
    void setCall(bool running, bool preferSpeaker);

signals:
    void changed();

private slots:
    void onFeatureChanged(const QString &name, uint allowed, uint enabled);
    void onRouteChanged(const QString &device, uint type);

private:
    void listen(bool on);
    void query();
    void applyPreference();
    void sendSpeaker(bool on);

    bool m_running = false;
    bool m_preferSpeaker = false;
    // The user chose: nothing else changes the route of this call any more.
    bool m_chosen = false;
    bool m_available = false;
    bool m_speaker = false;
    // We turned it on, so it is ours to turn off when the call ends.
    bool m_ours = false;
    QString m_output;
};

#endif // CALLROUTE_H
