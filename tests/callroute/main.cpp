// SPDX-License-Identifier: AGPL-3.0-only
// What the loudspeaker button says has to be what the audio policy did: a wrong "on" takes the
// ear guard away with the sound still at the ear. And a voice call stays off the loudspeaker.
// Exit 0: passed.
#include "callroute.h"

#include <QCoreApplication>
#include <cstdio>

namespace {

int failures = 0;

void check(bool ok, const char *what)
{
    if (!ok) {
        std::printf("FAILED: %s\n", what);
        ++failures;
    }
}

// The manager as the phone's would answer, by hand.
class Route : public CallRoute
{
public:
    QList<bool> sent;
    int failed = 0;
    Route()
    {
        connect(this, &CallRoute::failed, [this]() { ++failed; });
    }
    using CallRoute::answeredAllowed;
    using CallRoute::answeredRoute;
    using CallRoute::answeredSend;
    using CallRoute::confirmTimedOut;
    using CallRoute::onFeatureChanged;
    using CallRoute::onRouteChanged;

    void started(bool prefer = false)
    {
        setCall(true, prefer);
        answeredAllowed({ QStringLiteral("speaker") });
        answeredRoute(QStringLiteral("earpiece"));
    }
    void feature(bool on) { onFeatureChanged(QStringLiteral("speaker"), 1, on ? 1 : 0); }
    void route(const char *name) { onRouteChanged(QLatin1String(name), 0); }

protected:
    void busListen(bool) override {}
    void busQuery() override {}
    void busSend(bool on) override { sent.append(on); }
};

void testLightsOnlyWhenConfirmed()
{
    Route r;
    r.started();
    check(r.available(), "the manager offers it");
    r.setSpeaker(true);
    check(!r.speaker(), "not lit on asking");
    check(r.sent == QList<bool>{ true }, "asked for");
    r.answeredSend(true, true);
    check(!r.speaker(), "not lit on a reply alone");
    r.feature(true);
    check(!r.speaker(), "the feature alone, sound still on the earpiece");
    r.route("ihf");
    check(r.speaker(), "lit once the route moved");

    r.setSpeaker(false);
    check(r.speaker(), "stays lit until the way back is confirmed");
    r.feature(false);
    check(!r.speaker(), "off confirmed");
}

void testRefused()
{
    Route r;
    r.started();
    r.setSpeaker(true);
    r.answeredSend(false, true);
    check(!r.speaker(), "refused: not lit");
    check(r.failed == 1, "refused: the user hears of it");
    check(r.sent.last() == false, "refused: nothing left on");
}

void testPolicyDoesNotMoveIt()
{
    Route r;
    r.started();
    r.setSpeaker(true);
    r.answeredSend(true, true);
    r.feature(true);
    r.confirmTimedOut();
    check(!r.speaker(), "still on the earpiece: not lit");
    check(r.failed == 1, "still on the earpiece: the user hears of it");
    check(r.sent == (QList<bool>{ true, false }), "still on the earpiece: the feature goes off again");
    r.feature(false);
    check(!r.speaker(), "and stays off");
}

void testUnknownEarpieceName()
{
    // The device is called something else: the feature's word is all there is.
    Route r;
    r.setCall(true, false);
    r.answeredAllowed({ QStringLiteral("speaker") });
    r.answeredRoute(QStringLiteral("handset"));
    r.setSpeaker(true);
    r.feature(true);
    check(r.speaker(), "feature confirmed, on no earpiece we know of");
}

void testNotOffered()
{
    Route r;
    r.setCall(true, true);
    r.answeredAllowed({ QStringLiteral("bluetooth_override") });
    r.answeredRoute(QStringLiteral("earpiece"));
    check(!r.available(), "not offered: no button");
    r.setSpeaker(true);
    check(r.sent.isEmpty(), "not offered: nothing sent, also not for a camera");
}

void testPreference()
{
    Route voice;
    voice.started(false);
    voice.setCall(true, false);
    check(voice.sent.isEmpty(), "a voice call stays on the earpiece");

    voice.setCall(true, true);
    check(voice.sent == QList<bool>{ true }, "a camera comes on: loudspeaker");
    voice.route("ihf");
    voice.feature(true);
    check(voice.speaker(), "confirmed");
    voice.setCall(true, false);
    check(voice.speaker(), "the camera going off changes nothing");
    voice.setSpeaker(false);
    voice.feature(false);
    voice.route("earpiece");
    voice.setCall(true, true);
    check(voice.sent.size() == 2, "after the user's choice a camera doesn't move it");

    Route headset;
    headset.setCall(true, true);
    headset.answeredAllowed({ QStringLiteral("speaker") });
    headset.answeredRoute(QStringLiteral("wiredheadset"));
    check(headset.sent.isEmpty(), "a headset stays as it is");

    Route failing;
    failing.started(true);
    failing.answeredSend(false, true);
    check(failing.failed == 0, "nobody asked: no notice");
    failing.route("earpiece");
    check(failing.sent.size() == 2, "and no second try by itself");
}

void testEnd()
{
    Route r;
    r.started();
    r.setSpeaker(true);
    r.feature(true);
    r.route("ihf");
    r.setCall(false, false);
    check(r.sent.last() == false, "off with the call");
    check(!r.speaker() && !r.available(), "nothing shown after it");

    Route waiting;
    waiting.started();
    waiting.setSpeaker(true);
    waiting.setCall(false, false);
    check(waiting.sent.last() == false, "off with the call also while unconfirmed");
}

} // namespace

int main(int argc, char **argv)
{
    QCoreApplication app(argc, argv);
    testLightsOnlyWhenConfirmed();
    testRefused();
    testPolicyDoesNotMoveIt();
    testUnknownEarpieceName();
    testNotOffered();
    testPreference();
    testEnd();
    if (failures == 0) {
        std::printf("passed\n");
    }
    return failures == 0 ? 0 : 1;
}
