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
    apply();
    emit enabledChanged();
}

void ProximityMonitor::setForCall(bool forCall)
{
    if (m_forCall == forCall) {
        return;
    }
    m_forCall = forCall;
    apply();
    emit enabledChanged();
}

void ProximityMonitor::apply()
{
    const bool run = m_enabled || m_forCall;
    if (run == m_running) {
        return;
    }
    m_running = run;
    if (run) {
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
