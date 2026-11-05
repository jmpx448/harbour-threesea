// SPDX-License-Identifier: AGPL-3.0-only
#include "callroute.h"

#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusPendingCallWatcher>
#include <QDBusPendingReply>
#include <QStringList>

namespace {

// Sailjail's Audio permission (part of Microphone) lets us talk to it.
const char *const Service = "org.nemomobile.Route.Manager";
const char *const Path = "/org/nemomobile/Route/Manager";
const char *const Interface = "org.nemomobile.Route.Manager";
const char *const SpeakerFeature = "speaker";
const char *const Earpiece = "earpiece";

QDBusMessage routeCall(const char *method)
{
    return QDBusMessage::createMethodCall(QLatin1String(Service), QLatin1String(Path), QLatin1String(Interface),
                                          QLatin1String(method));
}

} // namespace

CallRoute::CallRoute(QObject *parent)
    : QObject(parent)
{
}

CallRoute::~CallRoute()
{
    // Waits: the process is ending, and the loudspeaker must not stay on for the next phone call.
    if (m_ours) {
        QDBusMessage disable = routeCall("Disable");
        disable << QLatin1String(SpeakerFeature);
        QDBusConnection::systemBus().call(disable, QDBus::Block, 1000);
    }
}

void CallRoute::listen(bool on)
{
    QDBusConnection bus = QDBusConnection::systemBus();
    const QString service = QLatin1String(Service), path = QLatin1String(Path), iface = QLatin1String(Interface);
    if (on) {
        bus.connect(service, path, iface, QStringLiteral("AudioFeatureChanged"), this,
                    SLOT(onFeatureChanged(QString,uint,uint)));
        bus.connect(service, path, iface, QStringLiteral("AudioRouteChanged"), this,
                    SLOT(onRouteChanged(QString,uint)));
    } else {
        bus.disconnect(service, path, iface, QStringLiteral("AudioFeatureChanged"), this,
                       SLOT(onFeatureChanged(QString,uint,uint)));
        bus.disconnect(service, path, iface, QStringLiteral("AudioRouteChanged"), this,
                       SLOT(onRouteChanged(QString,uint)));
    }
}

void CallRoute::setCall(bool running, bool preferSpeaker)
{
    if (running == m_running) {
        return;
    }
    m_running = running;
    if (running) {
        m_preferSpeaker = preferSpeaker;
        m_chosen = false;
        listen(true);
        query();
        return;
    }
    listen(false);
    if (m_ours) {
        sendSpeaker(false);
    }
    m_ours = false;
    m_output.clear();
    if (m_available || m_speaker) {
        m_available = false;
        m_speaker = false;
        emit changed();
    }
}

void CallRoute::query()
{
    QDBusConnection bus = QDBusConnection::systemBus();
    auto *allowed = new QDBusPendingCallWatcher(bus.asyncCall(routeCall("FeaturesAllowed")), this);
    connect(allowed, &QDBusPendingCallWatcher::finished, this, [this](QDBusPendingCallWatcher *call) {
        QDBusPendingReply<QStringList> reply = *call;
        call->deleteLater();
        if (!reply.isValid()) {
            qWarning("threesea: audio route manager gave no answer");
            return;
        }
        if (!m_running) {
            return;
        }
        const bool available = reply.value().contains(QLatin1String(SpeakerFeature));
        if (available != m_available) {
            m_available = available;
            emit changed();
        }
        applyPreference();
    });
    auto *routes = new QDBusPendingCallWatcher(bus.asyncCall(routeCall("ActiveRoutes")), this);
    connect(routes, &QDBusPendingCallWatcher::finished, this, [this](QDBusPendingCallWatcher *call) {
        QDBusPendingReply<QString, uint, QString, uint> reply = *call;
        call->deleteLater();
        if (reply.isValid() && m_running) {
            m_output = reply.argumentAt<0>();
            applyPreference();
        }
    });
}

// Once both answers are in. Only from the earpiece: a headset stays as it is.
void CallRoute::applyPreference()
{
    if (m_preferSpeaker && !m_chosen && m_available && !m_speaker && m_output == QLatin1String(Earpiece)) {
        m_preferSpeaker = false;
        m_speaker = true;
        m_ours = true;
        sendSpeaker(true);
        emit changed();
    }
}

void CallRoute::setSpeaker(bool on)
{
    if (!m_running || !m_available || on == m_speaker) {
        return;
    }
    m_chosen = true;
    m_speaker = on;
    m_ours = on;
    sendSpeaker(on);
    emit changed();
}

void CallRoute::sendSpeaker(bool on)
{
    QDBusMessage message = routeCall(on ? "Enable" : "Disable");
    message << QLatin1String(SpeakerFeature);
    QDBusConnection::systemBus().asyncCall(message);
}

void CallRoute::onFeatureChanged(const QString &name, uint allowed, uint enabled)
{
    if (!m_running || name != QLatin1String(SpeakerFeature)) {
        return;
    }
    const bool available = allowed != 0, speaker = enabled != 0;
    if (available != m_available || speaker != m_speaker) {
        m_available = available;
        m_speaker = speaker;
        emit changed();
        applyPreference();
    }
}

void CallRoute::onRouteChanged(const QString &device, uint)
{
    if (m_running) {
        m_output = device;
        applyPreference();
    }
}
