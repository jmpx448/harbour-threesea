// SPDX-License-Identifier: AGPL-3.0-only
#ifndef PROXIMITYMONITOR_H
#define PROXIMITYMONITOR_H

#include <QElapsedTimer>
#include <QObject>
#include <QProximitySensor>

// The proximity sensor, for playing voice messages at the ear (like Threema for Android):
// the chat enables it only while that can happen, so the sensor does not run all the time.
class ProximityMonitor : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool enabled READ enabled WRITE setEnabled NOTIFY enabledChanged)
    // The phone was brought against something (the ear). Something already there when the sensor
    // starts doesn't count: opening the app with a hand over the sensor is no reason to play.
    Q_PROPERTY(bool covered READ covered NOTIFY coveredChanged)

public:
    explicit ProximityMonitor(QObject *parent = nullptr);

    bool enabled() const { return m_enabled; }
    void setEnabled(bool enabled);
    bool covered() const { return m_covered; }

signals:
    void enabledChanged();
    void coveredChanged();

private:
    void updateReading();

    QProximitySensor *m_sensor;
    bool m_enabled = false;
    bool m_covered = false;
    bool m_armed = false;
    QElapsedTimer m_started;
};

#endif
