// SPDX-License-Identifier: AGPL-3.0-only
#ifndef PROXIMITYMONITOR_H
#define PROXIMITYMONITOR_H

#include <QElapsedTimer>
#include <QObject>
#include <QProximitySensor>

// The proximity sensor, for playing voice messages at the ear (like Threema for Android) and for
// a call held there: the chat and the window enable it only while that can happen, each for
// itself, so the sensor does not run all the time.
class ProximityMonitor : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool enabled READ enabled WRITE setEnabled NOTIFY enabledChanged)
    Q_PROPERTY(bool forCall READ forCall WRITE setForCall NOTIFY enabledChanged)
    // The phone was brought against something (the ear). Something already there when the sensor
    // starts doesn't count: opening the app with a hand over the sensor is no reason to play.
    Q_PROPERTY(bool covered READ covered NOTIFY coveredChanged)

public:
    explicit ProximityMonitor(QObject *parent = nullptr);

    bool enabled() const { return m_enabled; }
    void setEnabled(bool enabled);
    bool forCall() const { return m_forCall; }
    void setForCall(bool forCall);
    bool covered() const { return m_covered; }

signals:
    void enabledChanged();
    void coveredChanged();

private:
    void apply();
    void updateReading();

    QProximitySensor *m_sensor;
    bool m_enabled = false;
    bool m_forCall = false;
    bool m_running = false;
    bool m_covered = false;
    bool m_armed = false;
    QElapsedTimer m_started;
};

#endif
