// SPDX-License-Identifier: AGPL-3.0-only
#include "callroute.h"

#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusPendingCallWatcher>
#include <QDBusPendingReply>

namespace {

// Sailjail's Audio permission (part of Microphone) lets us talk to it. The interface is in the
// SDK; the two names below aren't (the policy's rules are on the phone), see the device checklist.
const char *const Service = "org.nemomobile.Route.Manager";
const char *const Path = "/org/nemomobile/Route/Manager";
const char *const Interface = "org.nemomobile.Route.Manager";
const char *const SpeakerFeature = "speaker";
const char *const Earpiece = "earpiece";
// The manager's signals come at once if it does what it was asked.
const int ConfirmMs = 3000;

QDBusMessage routeCall(const char *method)
{
    return QDBusMessage::createMethodCall(QLatin1String(Service), QLatin1String(Path), QLatin1String(Interface),
                                          QLatin1String(method));
}

} // namespace

CallRoute::CallRoute(QObject *parent)
    : QObject(parent)
{
    m_confirm.setSingleShot(true);
    m_confirm.setInterval(ConfirmMs);
    connect(&m_confirm, &QTimer::timeout, this, &CallRoute::confirmTimedOut);
}

CallRoute::~CallRoute()
{
    // Waits: the process is ending, and the loudspeaker must not stay on for the next phone call.
    if (m_ours || (m_pending && m_wanted)) {
        QDBusMessage disable = routeCall("Disable");
        disable << QLatin1String(SpeakerFeature);
        QDBusConnection::systemBus().call(disable, QDBus::Block, 1000);
    }
}

void CallRoute::busListen(bool on)
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

void CallRoute::busQuery()
{
    QDBusConnection bus = QDBusConnection::systemBus();
    const auto features = [this, &bus](const char *method, void (CallRoute::*answered)(const QStringList &)) {
        auto *watcher = new QDBusPendingCallWatcher(bus.asyncCall(routeCall(method)), this);
        connect(watcher, &QDBusPendingCallWatcher::finished, this, [this, answered](QDBusPendingCallWatcher *call) {
            QDBusPendingReply<QStringList> reply = *call;
            call->deleteLater();
            if (reply.isValid()) {
                (this->*answered)(reply.value());
            } else {
                qWarning("threesea: audio route manager gave no answer");
            }
        });
    };
    features("FeaturesAllowed", &CallRoute::answeredAllowed);
    features("FeaturesEnabled", &CallRoute::answeredEnabled);
    auto *routes = new QDBusPendingCallWatcher(bus.asyncCall(routeCall("ActiveRoutes")), this);
    connect(routes, &QDBusPendingCallWatcher::finished, this, [this](QDBusPendingCallWatcher *call) {
        QDBusPendingReply<QString, uint, QString, uint> reply = *call;
        call->deleteLater();
        if (reply.isValid()) {
            answeredRoute(reply.argumentAt<0>());
        }
    });
}

void CallRoute::busSend(bool on)
{
    QDBusMessage message = routeCall(on ? "Enable" : "Disable");
    message << QLatin1String(SpeakerFeature);
    auto *watcher = new QDBusPendingCallWatcher(QDBusConnection::systemBus().asyncCall(message), this);
    connect(watcher, &QDBusPendingCallWatcher::finished, this, [this, on](QDBusPendingCallWatcher *call) {
        const bool ok = !call->isError();
        call->deleteLater();
        answeredSend(ok, on);
    });
}

void CallRoute::setShown(bool available, bool speaker)
{
    if (available != m_available || speaker != m_speaker) {
        m_available = available;
        m_speaker = speaker;
        emit changed();
    }
}

void CallRoute::setCall(bool running, bool preferSpeaker)
{
    if (running != m_running) {
        m_running = running;
        if (!running) {
            busListen(false);
            if (m_ours || (m_pending && m_wanted)) {
                busSend(false);
            }
            m_confirm.stop();
            m_pending = false;
            m_ours = false;
            m_featureOn = false;
            m_output.clear();
            m_lastPrefer = false;
            m_preferSpeaker = false;
            setShown(false, false);
            return;
        }
        m_chosen = false;
        busListen(true);
        busQuery();
    }
    if (!running) {
        return;
    }
    if (preferSpeaker != m_lastPrefer) {
        m_lastPrefer = preferSpeaker;
        m_preferSpeaker = preferSpeaker;
        applyPreference();
    }
}

void CallRoute::answeredAllowed(const QStringList &features)
{
    if (m_running) {
        setShown(features.contains(QLatin1String(SpeakerFeature)), m_speaker);
        applyPreference();
    }
}

void CallRoute::answeredEnabled(const QStringList &features)
{
    if (m_running && !m_pending) {
        m_featureOn = features.contains(QLatin1String(SpeakerFeature));
        setShown(m_available, m_featureOn);
    }
}

void CallRoute::answeredRoute(const QString &device)
{
    if (m_running) {
        m_output = device;
        settle();
        applyPreference();
    }
}

// Only from the earpiece: a headset stays as it is.
void CallRoute::applyPreference()
{
    if (m_preferSpeaker && !m_chosen && m_available && !m_speaker && !m_pending
        && m_output == QLatin1String(Earpiece)) {
        m_preferSpeaker = false;
        request(true, false);
    }
}

void CallRoute::setSpeaker(bool on)
{
    if (!m_running || !m_available || m_pending || on == m_speaker) {
        return;
    }
    m_chosen = true;
    request(on, true);
}

void CallRoute::request(bool on, bool byUser)
{
    m_pending = true;
    m_wanted = on;
    m_byUser = byUser;
    m_outputAtRequest = m_output;
    m_confirm.start();
    busSend(on);
}

// The manager's feature follows, and the sound is not on the earpiece; or the route itself moved.
void CallRoute::settle()
{
    if (!m_pending) {
        return;
    }
    const bool moved = m_output != m_outputAtRequest;
    const bool onEarpiece = m_output == QLatin1String(Earpiece);
    const bool done = m_wanted ? (m_featureOn || moved) && !onEarpiece : !m_featureOn || moved;
    if (!done) {
        return;
    }
    m_confirm.stop();
    m_pending = false;
    m_ours = m_wanted;
    m_featureOn = m_wanted;
    setShown(m_available, m_wanted);
}

void CallRoute::answeredSend(bool ok, bool on)
{
    if (!ok && m_pending && m_wanted == on) {
        qWarning("threesea: audio route manager refused the loudspeaker change");
        giveUp();
    }
}

void CallRoute::confirmTimedOut()
{
    if (m_pending) {
        qWarning("threesea: audio route manager did not confirm the loudspeaker change");
        giveUp();
    }
}

// Back to what it was. The feature may be on with the sound still on the earpiece.
void CallRoute::giveUp()
{
    m_confirm.stop();
    m_pending = false;
    if (m_wanted) {
        m_ours = false;
        busSend(false);
    }
    if (m_byUser) {
        emit failed();
    }
}

void CallRoute::onFeatureChanged(const QString &name, uint allowed, uint enabled)
{
    if (!m_running || name != QLatin1String(SpeakerFeature)) {
        return;
    }
    m_featureOn = enabled != 0;
    if (m_pending) {
        setShown(allowed != 0, m_speaker);
        settle();
    } else {
        setShown(allowed != 0, m_featureOn);
        applyPreference();
    }
}

void CallRoute::onRouteChanged(const QString &device, uint)
{
    if (m_running) {
        m_output = device;
        settle();
        applyPreference();
    }
}
