// SPDX-License-Identifier: AGPL-3.0-only
// What ends a voice recording, without a microphone: only a tap sends. The length limit and
// leaving the app stop it and hold it for an answer, and what's under a second or thrown away is
// gone from memory. Exit 0: passed.
#include "voicerecorder.h"

#include <QElapsedTimer>
#include <QGuiApplication>
#include <QTemporaryDir>
#include <cstdio>
#include <memory>
#include <qpa/qwindowsysteminterface.h>

class VoiceRecorderTest
{
public:
    // A recording on the path without the platform's encoder, with words written in place of what
    // the microphone (which the SDK doesn't have) would give.
    explicit VoiceRecorderTest(VoiceRecorder &recorder)
        : m_recorder(recorder)
    {
        m_recorder.m_codec.clear();
        m_recorder.m_pcmAvailable = true;
    }
    void startWithWords(QObject *owner)
    {
        m_recorder.start(owner);
        m_recorder.m_pcmBuffer.write("words");
    }
    void limitAfter(int ms) { m_recorder.m_maxDuration = ms; }
    bool wordsGone() const { return m_recorder.m_pcm.isEmpty(); }

private:
    VoiceRecorder &m_recorder;
};

namespace {

int failures = 0;

void check(bool ok, const char *what)
{
    if (!ok) {
        std::printf("FAILED: %s\n", what);
        ++failures;
    }
}

void setActive(bool active)
{
    QWindowSystemInterface::handleApplicationStateChanged(active ? Qt::ApplicationActive : Qt::ApplicationInactive);
    QWindowSystemInterface::flushWindowSystemEvents();
}

void waitFor(int ms)
{
    QElapsedTimer clock;
    clock.start();
    while (clock.elapsed() < ms) {
        QCoreApplication::processEvents(QEventLoop::AllEvents, 20);
    }
}

// One recording with its own recorder, counting what it reports.
struct Run {
    VoiceRecorder recorder;
    VoiceRecorderTest test;
    QObject owner;
    int finished = 0;
    int discarded = 0;
    int failed = 0;

    Run()
        : test(recorder)
    {
        QObject::connect(&recorder, &VoiceRecorder::finished, [this](int) { ++finished; });
        QObject::connect(&recorder, &VoiceRecorder::discarded, [this]() { ++discarded; });
        QObject::connect(&recorder, &VoiceRecorder::failed, [this](const QString &) { ++failed; });
    }
};

void aTapSends()
{
    Run run;
    run.test.startWithWords(&run.owner);
    check(run.recorder.recording(), "tap: recording");
    waitFor(1100);
    bool pcm = false;
    QByteArray taken;
    QObject::connect(&run.recorder, &VoiceRecorder::finished,
                     [&]() { taken = run.recorder.takeRecording(&pcm); });
    run.recorder.stop();
    check(run.finished == 1 && run.failed == 0, "tap: finished once");
    check(taken == "words" && pcm, "tap: the words can be taken");
    check(!run.recorder.owner(), "tap: nobody owns it any more");
}

void underASecondIsGone()
{
    Run run;
    run.test.startWithWords(&run.owner);
    run.recorder.stop();
    check(run.finished == 0 && run.discarded == 0, "short: nothing to report");
    check(run.test.wordsGone(), "short: nothing kept");
}

void theLimitHoldsInFront()
{
    Run run;
    run.test.limitAfter(1100);
    run.test.startWithWords(&run.owner);
    waitFor(1500);
    check(!run.recorder.recording(), "limit: stopped");
    check(run.finished == 0, "limit: not sent by itself");
    check(run.recorder.held(), "limit: held for an answer");
    check(run.recorder.owner() == &run.owner, "limit: still its owner's");
    // Then: thrown away by the answer.
    run.recorder.cancel();
    check(!run.recorder.held() && run.test.wordsGone(), "limit: gone when discarded");
    check(run.finished == 0, "limit: discarding sends nothing");
}

void aHeldOneStaysWhenLeaving()
{
    Run run;
    run.test.limitAfter(1100);
    run.test.startWithWords(&run.owner);
    waitFor(1500);
    setActive(false);
    check(run.recorder.held(), "held, then away: still held");
    check(run.discarded == 0 && run.finished == 0, "held, then away: nothing happened to it");
    check(run.recorder.owner() == &run.owner, "held, then away: still its owner's");
    setActive(true);
    check(run.recorder.held(), "held, away and back: still held");
    run.recorder.cancel();
    setActive(false);
    setActive(true);
    check(!run.recorder.held() && run.test.wordsGone(), "held, discarded: stays gone");
}

void leavingStopsAndHolds()
{
    Run run;
    run.test.startWithWords(&run.owner);
    waitFor(1100);
    setActive(false);
    check(!run.recorder.recording(), "leaving: the microphone is free");
    check(run.finished == 0, "leaving: not sent");
    check(run.discarded == 0, "leaving: nothing lost");
    check(run.recorder.held(), "leaving: held for an answer");
    check(run.recorder.owner() == &run.owner, "leaving: still its owner's");
    setActive(true);
    check(!run.recorder.recording(), "coming back: it doesn't start again");
    check(run.recorder.held() && run.finished == 0, "coming back: still waiting");
    // The answer: only now it goes.
    bool pcm = false;
    const QByteArray taken = run.recorder.takeRecording(&pcm);
    check(taken == "words", "coming back: what was said can be sent");
    check(!run.recorder.held() && !run.recorder.owner(), "coming back: over once taken");
}

void leavingAfterASlipLeavesNothing()
{
    Run run;
    run.test.startWithWords(&run.owner);
    setActive(false);
    check(!run.recorder.recording() && !run.recorder.held(), "slip: nothing running or held");
    check(run.discarded == 0 && run.finished == 0, "slip: nothing to report");
    check(run.test.wordsGone(), "slip: nothing kept");
    setActive(true);
}

void itGoesWithItsOwner()
{
    Run run;
    {
        QObject page;
        run.test.startWithWords(&page);
        check(run.recorder.owner() == &page, "owner: the page");
    }
    check(!run.recorder.recording() && !run.recorder.owner(), "owner: gone with the page");
    check(run.finished == 0, "owner: not sent");
}

} // namespace

int main(int argc, char **argv)
{
    QTemporaryDir home;
    qputenv("XDG_CACHE_HOME", home.path().toUtf8());
    QGuiApplication app(argc, argv);
    setActive(true);
    aTapSends();
    underASecondIsGone();
    theLimitHoldsInFront();
    aHeldOneStaysWhenLeaving();
    leavingStopsAndHolds();
    leavingAfterASlipLeavesNothing();
    itGoesWithItsOwner();
    if (failures) {
        std::printf("voice recorder: %d failed\n", failures);
        return 1;
    }
    std::printf("voice recorder: passed\n");
    return 0;
}
