// SPDX-License-Identifier: AGPL-3.0-only
// GStreamer first: GLib headers use `signals` as a member name, which Qt defines as a macro.
#include <gst/gst.h>
#include <gst/app/gstappsrc.h>

#include "earpieceplayer.h"

#include <QDebug>

namespace {

// The core's Opus decoder hands over 48 kHz mono S16 (voice.rs).
const int PcmRate = 48000;
const int PcmBytesPerMs = PcmRate * 2 / 1000;
// One push into the appsrc.
const int ChunkSize = 64 * 1024;

} // namespace

EarpiecePlayer::EarpiecePlayer(QObject *parent)
    : QObject(parent)
{
    if (!gst_is_initialized()) {
        gst_init(nullptr, nullptr);
    }
    m_ticker.setInterval(100);
    connect(&m_ticker, &QTimer::timeout, this, &EarpiecePlayer::positionChanged);
    m_watchdog.setSingleShot(true);
    m_watchdog.setInterval(2000);
    connect(&m_watchdog, &QTimer::timeout, this, [this]() {
        if (m_prerolled) {
            return;
        }
        qWarning() << "earpiece pipeline: no sound route came up in time";
        release();
        setPlaying(false);
        emit failed(QStringLiteral("earpiece route"));
    });
}

EarpiecePlayer::~EarpiecePlayer()
{
    release();
}

bool EarpiecePlayer::start(const QByteArray &data, bool raw, int startMs, bool playing, double speed)
{
    release();
    if (data.isEmpty()) {
        return false;
    }
    m_data = data;
    m_raw = raw;
    m_speed = speed > 0.0 ? speed : 1.0;
    m_autoplay = playing;
    m_prerolled = false;
    m_seekMs = qMax(0, startMs);
    // Raw audio has no parser in the pipeline that could turn a byte offset into a time, so
    // the data is pushed from that offset on and the pipeline's own position starts at zero.
    m_baseMs = raw ? m_seekMs : 0;
    m_offset = raw ? qMin<qint64>(static_cast<qint64>(m_seekMs) * PcmBytesPerMs & ~1LL, m_data.size()) : 0;

    // The speed changes the pitch too in raw mode (the data is simply declared faster); the
    // encoded route has scaletempo for that, where the system has it.
    const bool stretch = !raw && gst_element_factory_find("scaletempo") != nullptr;
    const QString tempo = stretch ? QStringLiteral("scaletempo ! ") : QString();
    const QString description = raw
        ? QStringLiteral("appsrc name=src ! audioconvert ! audioresample ! %1"
                         "pulsesink name=sink stream-properties=\"props,media.role=phone\"").arg(tempo)
        : QStringLiteral("appsrc name=src ! decodebin ! audioconvert ! audioresample ! %1"
                         "pulsesink name=sink stream-properties=\"props,media.role=phone\"").arg(tempo);
    GError *error = nullptr;
    m_pipeline = gst_parse_launch(description.toUtf8().constData(), &error);
    if (error) {
        qWarning() << "earpiece pipeline:" << error->message;
        g_error_free(error);
        release();
        return false;
    }
    m_source = gst_bin_get_by_name(GST_BIN(m_pipeline), "src");
    if (!m_source) {
        release();
        return false;
    }
    g_object_set(m_source, "size", static_cast<gint64>(m_data.size()), "format", GST_FORMAT_BYTES, "stream-type",
                 raw ? 0 /* stream */ : 2 /* random access */, nullptr);
    if (raw) {
        // Declaring a higher rate plays faster, at a higher pitch (the fallback route's way).
        GstCaps *caps = gst_caps_new_simple("audio/x-raw", "format", G_TYPE_STRING, "S16LE", "rate", G_TYPE_INT,
                                            static_cast<int>(qRound(PcmRate * m_speed)), "channels", G_TYPE_INT, 1,
                                            "layout", G_TYPE_STRING, "interleaved", nullptr);
        g_object_set(m_source, "caps", caps, nullptr);
        gst_caps_unref(caps);
    }
    g_signal_connect(m_source, "need-data", G_CALLBACK(&EarpiecePlayer::onNeedData), this);
    g_signal_connect(m_source, "seek-data", G_CALLBACK(&EarpiecePlayer::onSeekData), this);

    GstBus *bus = gst_pipeline_get_bus(GST_PIPELINE(m_pipeline));
    m_busWatch = gst_bus_add_watch(bus, reinterpret_cast<GstBusFunc>(&EarpiecePlayer::onBusMessage), this);
    gst_object_unref(bus);
    const GstStateChangeReturn change = gst_element_set_state(m_pipeline, GST_STATE_PAUSED);
    if (change == GST_STATE_CHANGE_FAILURE) {
        qWarning() << "earpiece pipeline: cannot pause";
        release();
        return false;
    }
    if (change == GST_STATE_CHANGE_ASYNC) {
        // It is prerolling: the bus says when (ASYNC_DONE), and the watchdog if it never does.
        m_watchdog.start();
    } else {
        // Already there — no ASYNC_DONE will come, and waiting for one left the message
        // standing still (user report 2026-10-30: at the ear it paused instead of playing).
        prerolled();
    }
    return true;
}

// Prerolled (the decoder knows the stream): seek to where playback should start, then play.
void EarpiecePlayer::prerolled()
{
    if (m_prerolled) {
        return;
    }
    m_prerolled = true;
    m_watchdog.stop();
    // A speed other than 1 is a rate change, which is a seek as well (raw audio carries its
    // speed in the caps instead).
    if (!m_raw && (m_seekMs > 0 || !qFuzzyCompare(m_speed, 1.0))) {
        sendSeek(m_seekMs);
    }
    if (m_autoplay) {
        resume();
    }
    emit positionChanged();
}

bool EarpiecePlayer::sendSeek(int ms)
{
    if (!m_pipeline) {
        return false;
    }
    return gst_element_seek(m_pipeline, m_speed, GST_FORMAT_TIME,
                            static_cast<GstSeekFlags>(GST_SEEK_FLAG_FLUSH | GST_SEEK_FLAG_KEY_UNIT),
                            GST_SEEK_TYPE_SET, static_cast<gint64>(ms) * GST_MSECOND, GST_SEEK_TYPE_NONE,
                            GST_CLOCK_TIME_NONE)
        == TRUE;
}

void EarpiecePlayer::resume()
{
    if (!m_pipeline) {
        return;
    }
    m_autoplay = true;
    if (!m_prerolled) {
        return; // plays once prerolled
    }
    gst_element_set_state(m_pipeline, GST_STATE_PLAYING);
    setPlaying(true);
}

void EarpiecePlayer::pause()
{
    m_autoplay = false;
    if (!m_pipeline) {
        return;
    }
    gst_element_set_state(m_pipeline, GST_STATE_PAUSED);
    setPlaying(false);
}

void EarpiecePlayer::seek(int ms)
{
    if (!m_pipeline) {
        return;
    }
    if (m_raw) {
        // Push mode: start over at the new offset (the pipeline cannot seek in the appsrc).
        const QByteArray data = m_data;
        start(data, true, ms, m_playing, m_speed);
        return;
    }
    if (m_prerolled) {
        sendSeek(ms);
    } else {
        m_seekMs = ms;
    }
    emit positionChanged();
}

void EarpiecePlayer::setSpeed(double speed)
{
    const double wanted = speed > 0.0 ? speed : 1.0;
    if (qFuzzyCompare(wanted, m_speed)) {
        return;
    }
    if (!m_pipeline) {
        m_speed = wanted;
        return;
    }
    const int at = position();
    const bool wasPlaying = m_playing;
    if (m_raw) {
        const QByteArray data = m_data;
        m_speed = wanted;
        start(data, true, at, wasPlaying, wanted);
        return;
    }
    m_speed = wanted;
    if (m_prerolled) {
        sendSeek(at);
    }
}

void EarpiecePlayer::stop()
{
    release();
    setPlaying(false);
    emit positionChanged();
}

void EarpiecePlayer::setPlaying(bool playing)
{
    if (playing) {
        m_ticker.start();
    } else {
        m_ticker.stop();
    }
    if (m_playing == playing) {
        return;
    }
    m_playing = playing;
    emit playingChanged();
}

int EarpiecePlayer::position() const
{
    if (!m_pipeline) {
        return m_baseMs;
    }
    gint64 ns = 0;
    if (!gst_element_query_position(m_pipeline, GST_FORMAT_TIME, &ns) || ns < 0) {
        return m_baseMs;
    }
    const double ms = static_cast<double>(ns) / GST_MSECOND;
    // Raw mode runs the data faster than it is, so the pipeline's clock is compressed.
    return m_baseMs + static_cast<int>(m_raw ? ms * m_speed : ms);
}

int EarpiecePlayer::duration() const
{
    if (m_raw) {
        return m_data.size() / PcmBytesPerMs;
    }
    if (!m_pipeline) {
        return -1;
    }
    gint64 ns = 0;
    if (!gst_element_query_duration(m_pipeline, GST_FORMAT_TIME, &ns) || ns <= 0) {
        return -1;
    }
    return static_cast<int>(ns / GST_MSECOND);
}

void EarpiecePlayer::release()
{
    m_ticker.stop();
    m_watchdog.stop();
    if (m_busWatch) {
        g_source_remove(m_busWatch);
        m_busWatch = 0;
    }
    if (m_source) {
        g_signal_handlers_disconnect_by_data(m_source, this);
        gst_object_unref(m_source);
        m_source = nullptr;
    }
    if (m_pipeline) {
        // NULL joins the streaming threads, so nothing reads m_data afterwards.
        gst_element_set_state(m_pipeline, GST_STATE_NULL);
        gst_object_unref(m_pipeline);
        m_pipeline = nullptr;
    }
    // Shared with the caller, which wipes the bytes once nothing reads them any more.
    m_data.clear();
    m_offset = 0;
    m_prerolled = false;
    m_autoplay = false;
}

// --- GStreamer callbacks. The bus watch runs on this thread (GLib main loop = Qt's event
// loop); the appsrc's run on its streaming thread and touch nothing but m_offset and m_data,
// which stand still while the pipeline is alive. ---

int EarpiecePlayer::onBusMessage(GstBus *, GstMessage *message, void *self)
{
    EarpiecePlayer *player = static_cast<EarpiecePlayer *>(self);
    switch (GST_MESSAGE_TYPE(message)) {
    case GST_MESSAGE_ASYNC_DONE:
        player->prerolled();
        break;
    case GST_MESSAGE_EOS:
        player->release();
        player->setPlaying(false);
        emit player->finished();
        break;
    case GST_MESSAGE_ERROR: {
        GError *error = nullptr;
        gst_message_parse_error(message, &error, nullptr);
        // Threema's own error texts only: the message names elements and codecs, no content.
        const QString text = error ? QString::fromUtf8(error->message) : QString();
        qWarning() << "earpiece pipeline error:" << text; // log-ok: GStreamer's own text
        if (error) {
            g_error_free(error);
        }
        player->release();
        player->setPlaying(false);
        emit player->failed(text);
        break;
    }
    default:
        break;
    }
    return TRUE;
}

void EarpiecePlayer::onNeedData(GstElement *source, unsigned length, void *self)
{
    EarpiecePlayer *player = static_cast<EarpiecePlayer *>(self);
    const qint64 size = player->m_data.size();
    if (player->m_offset >= size) {
        gst_app_src_end_of_stream(GST_APP_SRC(source));
        return;
    }
    const qint64 wanted = qBound<qint64>(ChunkSize, length, 4 * ChunkSize);
    const qint64 count = qMin(wanted, size - player->m_offset);
    // Read-only view of the data; the pipeline is stopped before it is cleared.
    GstBuffer *buffer = gst_buffer_new_wrapped_full(
        GST_MEMORY_FLAG_READONLY, const_cast<char *>(player->m_data.constData()) + player->m_offset,
        static_cast<gsize>(count), 0, static_cast<gsize>(count), nullptr, nullptr);
    player->m_offset += count;
    if (gst_app_src_push_buffer(GST_APP_SRC(source), buffer) != GST_FLOW_OK) {
        player->m_offset = size;
    }
}

int EarpiecePlayer::onSeekData(GstElement *, quint64 offset, void *self)
{
    EarpiecePlayer *player = static_cast<EarpiecePlayer *>(self);
    if (static_cast<qint64>(offset) > player->m_data.size()) {
        return FALSE;
    }
    player->m_offset = static_cast<qint64>(offset);
    return TRUE;
}
