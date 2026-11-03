// SPDX-License-Identifier: AGPL-3.0-only
#include "proximitymonitor.h"

namespace {
// A "close" reading this soon after the start is how things already were, not a gesture.
const int InitialReadingMs = 1000;
} // namespace

ProximityMonitor::ProximityMonitor(QObject *parent)
    : QObject(parent)
    , m_sensor(new QProximitySensor(this))
{
    connect(m_sensor, &QProximitySensor::readingChanged, this, &ProximityMonitor::updateReading);
}

void ProximityMonitor::setEnabled(bool enabled)
{
    if (m_enabled == enabled) {
        return;
    }
    m_enabled = enabled;
    if (enabled) {
        m_armed = false;
        m_started.start();
        m_sensor->start();
    } else {
        m_sensor->stop();
        if (m_covered) {
            m_covered = false;
            emit coveredChanged();
        }
    }
    emit enabledChanged();
}

void ProximityMonitor::updateReading()
{
    const QProximityReading *reading = m_sensor->reading();
    const bool covered = reading && reading->close();
    if (!m_armed) {
        if (covered && m_started.elapsed() < InitialReadingMs) {
            return;
        }
        m_armed = true;
    }
    if (covered != m_covered) {
        m_covered = covered;
        emit coveredChanged();
    }
}
