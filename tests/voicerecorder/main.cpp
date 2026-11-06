// SPDX-License-Identifier: AGPL-3.0-only
// What ends a voice recording, without a microphone: only a tap sends. The length limit stops it
// and holds it for an answer; leaving the app throws it away, running or held, and what's under a
// second or thrown away is gone from memory. Pausing doesn't count as time and ends the same ways; listening never gets in
// the way of what happens to the recording. Exit 0: passed.
#include "voicerecorder.h"

#include <QDir>
#include <QElapsedTimer>
#include <QFile>
#include <QFileInfo>
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
    // A recording on the platform's path, with what its backend reports played by the test: the SDK
    // has no microphone for the real one.
    void startPlatform(QObject *owner, int recordedMs)
    {
        m_recorder.m_codec = QStringLiteral("aac");
        m_recorder.setOwner(owner);
        m_recorder.m_recording = true;
        m_recorder.m_stop = VoiceRecorder::Stop::ByItself;
        m_recorder.m_recordedBefore = recordedMs;
        m_recorder.m_clock.start();
    }
    void writeFile(const QByteArray &data)
    {
        QDir().mkpath(QFileInfo(m_recorder.outputPath()).path());
        QFile file(m_recorder.outputPath());
        file.open(QIODevice::WriteOnly);
        file.write(data);
    }
    bool fileGone() const { return !QFile::exists(m_recorder.outputPath()); }
    void reportStopped(QMediaRecorder::Status status) { m_recorder.recorderStopped(status); }
    void reportStatus(QMediaRecorder::Status status) { m_recorder.recorderStatus(status); }
    void finalizeWithin(int ms) { m_recorder.m_finalizeWait.setInterval(ms); }
    void reportTime(qint64 position) { m_recorder.pauseSample(position); }
    // The real timer stays out of it: the test says when the time is looked at.
    void noPauseTimer() { m_recorder.m_pauseWait.stop(); }
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

void aHeldOneGoesWhenLeaving()
{
    Run run;
    run.test.limitAfter(1100);
    run.test.startWithWords(&run.owner);
    waitFor(1500);
    check(run.recorder.held(), "held: waits in front");
    setActive(false);
    check(!run.recorder.held() && run.test.wordsGone(), "held, then away: gone");
    check(run.discarded == 1 && run.finished == 0, "held, then away: discarded, and said so");
    check(!run.recorder.owner(), "held, then away: nobody's any more");
    setActive(true);
    check(!run.recorder.held() && !run.recorder.recording(), "held, away and back: nothing waits");
}

void leavingStopsAndDiscards()
{
    Run run;
    run.test.startWithWords(&run.owner);
    waitFor(1100);
    setActive(false);
    check(!run.recorder.recording(), "leaving: the microphone is free");
    check(run.finished == 0, "leaving: not sent");
    check(run.discarded == 1, "leaving: discarded, and said so");
    check(!run.recorder.held() && run.test.wordsGone(), "leaving: nothing kept");
    check(!run.recorder.owner(), "leaving: nobody's any more");
    setActive(true);
    check(!run.recorder.recording() && !run.recorder.held(), "coming back: nothing starts or waits");
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

void aPauseIsntRecordedTime()
{
    Run run;
    run.test.limitAfter(1800);
    run.test.startWithWords(&run.owner);
    waitFor(600);
    run.recorder.pause();
    check(run.recorder.paused() && run.recorder.recording(), "pause: still its recording");
    const int atPause = run.recorder.duration();
    check(atPause >= 500 && atPause < 1000, "pause: the time so far");
    // Longer than the limit, which only counts what is recorded.
    waitFor(1500);
    check(run.recorder.duration() == atPause, "pause: the time stands");
    check(run.recorder.recording() && !run.recorder.held(), "pause: the limit isn't reached");
    run.recorder.resume();
    check(!run.recorder.paused(), "resume: going on");
    waitFor(500);
    check(run.recorder.duration() >= atPause + 400 && run.recorder.duration() < atPause + 900, "resume: the time goes on");
    bool pcm = false;
    QByteArray taken;
    int sentDuration = 0;
    QObject::connect(&run.recorder, &VoiceRecorder::finished, [&](int duration) {
        sentDuration = duration;
        taken = run.recorder.takeRecording(&pcm);
    });
    run.recorder.stop();
    check(taken == "words", "resume: all of it goes");
    check(sentDuration >= atPause + 400 && sentDuration < atPause + 900, "resume: without the pause's time");
    check(!run.recorder.paused(), "resume: over");
}

void aPausedOneEndsLikeARunningOne()
{
    {
        Run run;
        run.test.startWithWords(&run.owner);
        waitFor(1100);
        run.recorder.pause();
        bool pcm = false;
        QByteArray taken;
        QObject::connect(&run.recorder, &VoiceRecorder::finished, [&]() { taken = run.recorder.takeRecording(&pcm); });
        run.recorder.stop();
        check(run.finished == 1 && taken == "words", "paused: a tap sends");
    }
    {
        Run run;
        run.test.startWithWords(&run.owner);
        waitFor(1100);
        run.recorder.pause();
        run.recorder.cancel();
        check(!run.recorder.owner() && !run.recorder.paused() && run.test.wordsGone(), "paused: discarded");
        check(run.finished == 0, "paused: discarding sends nothing");
    }
    {
        Run run;
        run.test.startWithWords(&run.owner);
        waitFor(1100);
        run.recorder.pause();
        setActive(false);
        check(!run.recorder.recording() && !run.recorder.held(), "paused, then away: the microphone is free, nothing held");
        check(!run.recorder.paused() && run.test.wordsGone(), "paused, then away: gone");
        check(run.discarded == 1 && run.finished == 0, "paused, then away: discarded, and said so");
        setActive(true);
    }
    {
        Run run;
        run.test.startWithWords(&run.owner);
        waitFor(1100);
        run.recorder.hold();
        run.recorder.resume();
        check(!run.recorder.recording() && run.recorder.held(), "held: can't go on recording");
        run.recorder.cancel();
    }
}

void listeningAsksForAHeldOne()
{
    Run run;
    run.recorder.startPreview();
    check(!run.recorder.previewing(), "listen: nothing recorded");
    run.test.startWithWords(&run.owner);
    waitFor(1100);
    run.recorder.startPreview();
    check(!run.recorder.previewing(), "listen: not while it records");
    run.recorder.hold();
    check(run.recorder.held() && run.finished == 0, "hold: held, not sent");
    check(run.recorder.owner() == &run.owner, "hold: still its owner's");
    bool pcm = false;
    check(run.recorder.takeRecording(&pcm) == "words", "hold: the words are there");

    // Under a second: nothing to listen to.
    Run slip;
    slip.test.startWithWords(&slip.owner);
    slip.recorder.hold();
    check(!slip.recorder.held() && !slip.recorder.owner() && slip.test.wordsGone(), "hold: a slip leaves nothing");
    check(slip.discarded == 0, "hold: a slip isn't reported");
}

// The SDK has no sound: whether it sounds is for a device; here, that what goes on doesn't depend
// on it, and that listening doesn't touch the words.
void listeningLeavesTheWordsAlone()
{
    {
        Run run;
        run.test.startWithWords(&run.owner);
        waitFor(1100);
        run.recorder.hold();
        run.recorder.startPreview();
        run.recorder.seekPreview(500);
        run.recorder.pausePreview();
        check(run.recorder.held() && !run.recorder.previewing(), "listen: still held after pausing");
        check(!run.test.wordsGone(), "listen: the words are still there");
        bool pcm = false;
        run.recorder.startPreview();
        check(run.recorder.takeRecording(&pcm) == "words", "listen: all of it can be sent");
        check(!run.recorder.previewing() && !run.recorder.held(), "listen: over once taken");
    }
    {
        Run run;
        run.test.startWithWords(&run.owner);
        waitFor(1100);
        run.recorder.hold();
        run.recorder.startPreview();
        run.recorder.cancel();
        check(!run.recorder.previewing() && run.test.wordsGone(), "listen: discarding ends it, nothing kept");
        check(run.recorder.previewPosition() == 0, "listen: back at the start");
    }
}

// Qt says "stopped" while the file is still being written: nothing may use it before it is done.
void aFileIsUsedOnlyOnceFinished()
{
    {
        Run run;
        run.test.startPlatform(&run.owner, 2000);
        bool pcm = true;
        QByteArray taken;
        QObject::connect(&run.recorder, &VoiceRecorder::finished, [&]() { taken = run.recorder.takeRecording(&pcm); });
        run.recorder.stop();
        run.test.reportStopped(QMediaRecorder::FinalizingStatus);
        check(run.finished == 0 && run.recorder.recording(), "file, send: waits while it is written");
        run.test.reportStatus(QMediaRecorder::FinalizingStatus);
        check(run.finished == 0, "file, send: still written");
        run.test.writeFile("words");
        run.test.reportStatus(QMediaRecorder::LoadedStatus);
        check(run.finished == 1 && taken == "words" && !pcm, "file, send: all of it, then");
        check(run.test.fileGone(), "file, send: gone once taken");
    }
    {
        Run run;
        run.test.startPlatform(&run.owner, 2000);
        run.recorder.hold();
        run.test.reportStopped(QMediaRecorder::FinalizingStatus);
        check(!run.recorder.held(), "file, hold: not held while it is written");
        run.test.writeFile("words");
        run.test.reportStatus(QMediaRecorder::LoadedStatus);
        check(run.recorder.held() && !run.recorder.recording(), "file, hold: held once written");
        run.recorder.cancel();
        check(run.test.fileGone(), "file, hold: discarded");
    }
    {
        // By itself, and in the state that needs no waiting.
        Run run;
        run.test.startPlatform(&run.owner, 2000);
        run.test.writeFile("words");
        run.test.reportStopped(QMediaRecorder::LoadedStatus);
        check(run.recorder.held() && run.finished == 0, "file, by itself: held, not sent");
    }
    {
        // Nothing written after all: gone, and said so.
        Run run;
        run.test.startPlatform(&run.owner, 2000);
        run.test.writeFile("");
        run.recorder.hold();
        run.test.reportStopped(QMediaRecorder::FinalizingStatus);
        run.test.reportStatus(QMediaRecorder::LoadedStatus);
        check(!run.recorder.held() && run.discarded == 1, "file, empty: nothing to hold");
    }
    {
        // The status never comes: nobody stays stuck, and what may be cut off doesn't go out.
        Run run;
        run.test.finalizeWithin(50);
        run.test.startPlatform(&run.owner, 2000);
        run.test.writeFile("wor");
        run.recorder.stop();
        run.test.reportStopped(QMediaRecorder::FinalizingStatus);
        waitFor(150);
        check(!run.recorder.recording(), "file, no status: not stuck");
        check(run.finished == 0 && run.recorder.held(), "file, no status: held, not sent");
        run.recorder.cancel();
    }
    {
        // Thrown away while it is written.
        Run run;
        run.test.startPlatform(&run.owner, 2000);
        run.recorder.stop();
        run.test.reportStopped(QMediaRecorder::FinalizingStatus);
        run.recorder.cancel();
        run.test.writeFile("words");
        run.test.reportStatus(QMediaRecorder::LoadedStatus);
        check(run.finished == 0 && !run.recorder.held() && run.test.fileGone(), "file, discarded meanwhile: gone");
    }
}

// Leaving while the platform still writes the file: what a tap sent goes on, nothing else waits.
void leavingWhileTheFileIsWritten()
{
    {
        Run run;
        run.test.startPlatform(&run.owner, 2000);
        bool pcm = true;
        QByteArray taken;
        QObject::connect(&run.recorder, &VoiceRecorder::finished, [&]() { taken = run.recorder.takeRecording(&pcm); });
        run.recorder.stop();
        run.test.reportStopped(QMediaRecorder::FinalizingStatus);
        setActive(false);
        run.test.writeFile("words");
        run.test.reportStatus(QMediaRecorder::LoadedStatus);
        check(run.finished == 1 && taken == "words" && run.discarded == 0, "file, sent, then away: it goes");
        setActive(true);
    }
    {
        // Sent, but the platform never says it is done: it would be held, and nobody is there.
        Run run;
        run.test.finalizeWithin(50);
        run.test.startPlatform(&run.owner, 2000);
        run.test.writeFile("wor");
        run.recorder.stop();
        run.test.reportStopped(QMediaRecorder::FinalizingStatus);
        setActive(false);
        waitFor(150);
        check(!run.recorder.recording() && !run.recorder.held(), "file, no status, away: not held");
        check(run.finished == 0 && run.discarded == 1 && run.test.fileGone(), "file, no status, away: gone, and said so");
        setActive(true);
    }
    {
        // Being stopped to listen to: thrown away like a running one.
        Run run;
        run.test.startPlatform(&run.owner, 2000);
        run.recorder.hold();
        run.test.reportStopped(QMediaRecorder::FinalizingStatus);
        setActive(false);
        run.test.writeFile("words");
        run.test.reportStatus(QMediaRecorder::LoadedStatus);
        check(!run.recorder.held() && run.test.fileGone(), "file, holding, away: gone");
        check(run.discarded == 1 && run.finished == 0, "file, holding, away: said once");
        setActive(true);
    }
}

// Qt says "paused" at once: only the recorder's time tells.
void aPauseIsJudgedByTheRecordersTime()
{
    {
        Run run;
        run.test.startPlatform(&run.owner, 0);
        waitFor(600);
        run.recorder.pause();
        run.test.noPauseTimer();
        run.test.reportTime(600);
        run.test.noPauseTimer();
        run.test.reportTime(620);
        check(run.recorder.paused() && run.recorder.recording(), "time stands: paused");
        run.recorder.resume();
        check(!run.recorder.paused() && run.recorder.recording(), "time stands: goes on");
    }
    {
        Run run;
        run.test.startPlatform(&run.owner, 2000);
        waitFor(600);
        run.recorder.pause();
        run.test.noPauseTimer();
        run.test.writeFile("words");
        run.test.reportTime(600);
        run.test.noPauseTimer();
        run.test.reportTime(1100);
        run.test.reportStopped(QMediaRecorder::LoadedStatus);
        check(run.recorder.held() && !run.recorder.recording(), "time goes on: stopped and held");
        run.recorder.cancel();
    }
    {
        // A backend that tells no time can't show it has paused.
        Run run;
        run.test.startPlatform(&run.owner, 2000);
        waitFor(600);
        run.recorder.pause();
        run.test.noPauseTimer();
        run.test.writeFile("words");
        run.test.reportTime(0);
        run.test.noPauseTimer();
        run.test.reportTime(0);
        run.test.reportStopped(QMediaRecorder::LoadedStatus);
        check(run.recorder.held(), "no time: stopped and held");
        run.recorder.cancel();
    }
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
    aHeldOneGoesWhenLeaving();
    leavingStopsAndDiscards();
    leavingAfterASlipLeavesNothing();
    aPauseIsntRecordedTime();
    aPausedOneEndsLikeARunningOne();
    listeningAsksForAHeldOne();
    listeningLeavesTheWordsAlone();
    aFileIsUsedOnlyOnceFinished();
    leavingWhileTheFileIsWritten();
    aPauseIsJudgedByTheRecordersTime();
    itGoesWithItsOwner();
    if (failures) {
        std::printf("voice recorder: %d failed\n", failures);
        return 1;
    }
    std::printf("voice recorder: passed\n");
    return 0;
}
