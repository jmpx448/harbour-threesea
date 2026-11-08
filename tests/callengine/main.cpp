// SPDX-License-Identifier: AGPL-3.0-only
// Two engines call each other through host candidates: offer → answer → ICE → DTLS. Exits 0
// once both report `connected` and audio arrived, 1 on failure or after 30 s.
// THREESEA_TEST_BAD_FINGERPRINT=1: the callee gets a wrong fingerprint for the caller and must
// refuse the call without any audio passing either way, also while its check is delayed (the
// event loop is kept busy meanwhile): exits 0 then.
// THREESEA_TEST_MUTED=1: muted before the pipeline exists, the pipeline's microphone starts muted
// and follows later changes: exits 0 right after building it, without a call.
#include "callengine.h"

#include <gst/gst.h>

#include <QCoreApplication>
#include <QImage>
#include <QFile>
#include <QRegularExpression>
#include <QThread>
#include <QTimer>
#include <cstdio>

class CallEngineTest
{
public:
    static bool hasPipeline(const CallEngine &engine) { return engine.m_micVolume != nullptr; }
    // What the pipeline's microphone is really set to; false without a pipeline.
    static bool microphoneMuted(const CallEngine &engine)
    {
        gboolean muted = FALSE;
        if (engine.m_micVolume) {
            g_object_get(engine.m_micVolume, "mute", &muted, nullptr);
        }
        return muted;
    }
};

int main(int argc, char **argv)
{
    // QImage needs QtGui linked, not a GUI application.
    QCoreApplication app(argc, argv);
    // THREESEA_TEST_VIDEO=1: a video call; both cameras (test patterns) go on once connected and
    // each side must receive the other's frames.
    const bool video = !qgetenv("THREESEA_TEST_VIDEO").isEmpty();
    const bool badFingerprint = !qgetenv("THREESEA_TEST_BAD_FINGERPRINT").isEmpty();
    int callerFrames = 0;
    int calleeFrames = 0;
    CallEngine caller;
    CallEngine callee;
    int connected = 0;

    QObject::connect(&caller, &CallEngine::localDescription, [&](const QString &type, const QString &sdp) {
        std::printf("caller %s:\n%s\n", qPrintable(type), qPrintable(sdp));
        QString offer = sdp;
        if (badFingerprint) {
            offer.replace(QRegularExpression(QStringLiteral("(a=fingerprint:sha-256 )..")), QStringLiteral("\\1FF"));
        }
        callee.startAnswer(offer, QVariantMap(), false, video);
    });
    QObject::connect(&callee, &CallEngine::localDescription, [&](const QString &type, const QString &sdp) {
        std::printf("callee %s:\n%s\n", qPrintable(type), qPrintable(sdp));
        caller.setRemoteAnswer(sdp);
    });
    QObject::connect(&caller, &CallEngine::localCandidate,
                     [&](const QVariantMap &candidate) { callee.addRemoteCandidates({candidate}); });
    QObject::connect(&callee, &CallEngine::localCandidate,
                     [&](const QVariantMap &candidate) { caller.addRemoteCandidates({candidate}); });
    // The data channel: the caller says hello once connected, the callee must receive it.
    bool channelOk = false;
    QObject::connect(&callee, &CallEngine::channelMessage, [&](const QByteArray &data) {
        std::printf("channel: %s\n", data.constData());
        channelOk = data == "hello";
    });
    QObject::connect(&caller, &CallEngine::remoteFrame, [&](const QImage &) { ++callerFrames; });
    QObject::connect(&callee, &CallEngine::remoteFrame, [&](const QImage &) { ++calleeFrames; });
    auto onConnected = [&] {
        if (++connected == 2) {
            std::printf("connected\n");
            caller.sendChannel("hello");
            if (video) {
                caller.setCamera(true, true);
                callee.setCamera(true, true);
            }
            // Let audio flow for a moment (pipeline errors would show up now).
            QTimer::singleShot(video ? 6000 : 3000, &app, [&] {
                if (!channelOk) {
                    std::printf("no data channel message\n");
                }
                const bool framesOk = !video || (callerFrames > 0 && calleeFrames > 0);
                if (video) {
                    std::printf("video frames: caller received %d, callee received %d\n", callerFrames,
                                calleeFrames);
                    // Off again: the camera branch must go away cleanly.
                    caller.setCamera(false, true);
                }
                // The bridge's watch on a running call relies on this count.
                const bool packetsOk = caller.packetsReceived() > 0 && callee.packetsReceived() > 0;
                std::printf("packets received: caller %u, callee %u\n", caller.packetsReceived(),
                            callee.packetsReceived());
                app.exit(channelOk && framesOk && packetsOk ? 0 : 1);
            });
        }
    };
    QObject::connect(&caller, &CallEngine::connected, onConnected);
    QObject::connect(&callee, &CallEngine::connected, onConnected);
    auto onFailed = [&](const QString &reason) {
        std::printf("failed: %s\n", qPrintable(reason));
        if (!badFingerprint) {
            app.exit(1);
            return;
        }
        // Whatever was under way arrives within a moment.
        QTimer::singleShot(1000, &app, [&, reason] {
            const unsigned toCaller = caller.audioFramesReceived();
            const unsigned toCallee = callee.audioFramesReceived();
            std::printf("audio frames received: caller %u, callee %u\n", toCaller, toCallee);
            app.exit(reason == QLatin1String("certificate") && toCaller == 0 && toCallee == 0 ? 0 : 1);
        });
    };
    // A slow device: the check (queued on this thread) comes late, media threads go on.
    QTimer busy;
    if (badFingerprint) {
        QObject::connect(&busy, &QTimer::timeout, [] { QThread::msleep(300); });
        busy.start(10);
    }
    QObject::connect(&caller, &CallEngine::failed, onFailed);
    QObject::connect(&callee, &CallEngine::failed, onFailed);
    QTimer::singleShot(30000, &app, [&] {
        std::printf("timeout\n");
        app.exit(1);
    });

    // THREESEA_TEST_OFFER=file: only answer that offer (e.g. tests/callengine/android-offer.sdp)
    // and print the answer: shows how webrtcbin answers another app's offer.
    const QByteArray offerFile = qgetenv("THREESEA_TEST_OFFER");
    if (!offerFile.isEmpty()) {
        QFile file(QString::fromUtf8(offerFile));
        if (!file.open(QIODevice::ReadOnly)) {
            std::printf("no offer file\n");
            return 1;
        }
        CallEngine answerer;
        QObject::connect(&answerer, &CallEngine::localDescription, [&](const QString &type, const QString &sdp) {
            std::printf("%s:\n%s\n", qPrintable(type), qPrintable(sdp));
            app.exit(0);
        });
        QObject::connect(&answerer, &CallEngine::failed, onFailed);
        answerer.startAnswer(QString::fromUtf8(file.readAll()), QVariantMap(), false, video);
        const int result = app.exec();
        answerer.stop();
        return result;
    }

    // THREESEA_TEST_TURN=turn:host:port?transport=udp: the caller also uses that TURN server
    // (dummy credentials: checks resolving and contacting it, not relaying).
    QVariantMap turn;
    const QByteArray testTurn = qgetenv("THREESEA_TEST_TURN");
    if (!testTurn.isEmpty()) {
        turn.insert(QStringLiteral("urls"), QVariantList() << QString::fromUtf8(testTurn));
        turn.insert(QStringLiteral("username"), QStringLiteral("test"));
        turn.insert(QStringLiteral("password"), QStringLiteral("test"));
    }
    if (!qgetenv("THREESEA_TEST_MUTED").isEmpty()) {
        CallEngine muted;
        muted.setMuted(true);
        muted.startOffer(turn, false, false);
        const bool startedMuted = CallEngineTest::microphoneMuted(muted);
        muted.setMuted(false);
        const bool followed = CallEngineTest::hasPipeline(muted) && !CallEngineTest::microphoneMuted(muted);
        std::printf("pipeline: %d, started muted: %d, followed: %d\n", CallEngineTest::hasPipeline(muted), startedMuted,
                    followed);
        muted.stop();
        return startedMuted && followed ? 0 : 1;
    }
    caller.startOffer(turn, false, video);
    const int result = app.exec();
    caller.stop();
    callee.stop();
    return result;
}
