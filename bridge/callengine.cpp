// SPDX-License-Identifier: AGPL-3.0-only
// GStreamer first: GLib headers use `signals` as a member name, which Qt defines as a macro.
#define GST_USE_UNSTABLE_API
#include <gst/gst.h>
#include <gst/sdp/sdp.h>
#include <gst/webrtc/webrtc.h>
#include <gst/app/gstappsink.h>
#include <gst/video/video.h>

#include "callengine.h"
#include "dtlscertificate.h"
#include "vp8filter.h"
#include "aec/echocanceller.h"

#include <QCoreApplication>
#include <QDebug>
#include <QImage>
#include <QRegularExpression>
#include <QUrl>

namespace {

// Android: SIGNALING_CHANNEL_ID, negotiated with id 0 (o2o-call.proto messages).
const char DATA_CHANNEL_LABEL[] = "3MACALLdc0";
// The payload type of our Opus offer (WebRTC's usual one); answers use the offer's.
const int OPUS_PAYLOAD_TYPE = 111;
const int VP8_PAYLOAD_TYPE = 96;
// Software VP8 on a phone's CPU: at most this much, whatever the quality profile allows.
const int MAX_VIDEO_WIDTH = 640;
const int MAX_VIDEO_HEIGHT = 480;

// "turn:host:port?transport=udp" → "turn://user:pass@host:port?transport=udp" (webrtcbin).
QString turnUri(const QString &url, const QString &user, const QString &password)
{
    const int colon = url.indexOf(QLatin1Char(':'));
    if (colon < 0) {
        return QString();
    }
    const QString scheme = url.left(colon);
    if (scheme != QLatin1String("turn") && scheme != QLatin1String("turns")) {
        return QString();
    }
    return scheme + QStringLiteral("://") + QString::fromLatin1(QUrl::toPercentEncoding(user)) + QLatin1Char(':')
        + QString::fromLatin1(QUrl::toPercentEncoding(password)) + QLatin1Char('@') + url.mid(colon + 1);
}

// The payload type of `codec` ("opus/48000", "VP8/90000") in the remote offer's `media` section
// (the answer must use the offer's), or -1.
int payloadType(const QString &sdp, const char *media, const QString &codec)
{
    GstSDPMessage *message = nullptr;
    if (gst_sdp_message_new_from_text(sdp.toUtf8().constData(), &message) != GST_SDP_OK) {
        return -1;
    }
    int result = -1;
    for (guint i = 0; i < gst_sdp_message_medias_len(message) && result < 0; ++i) {
        const GstSDPMedia *section = gst_sdp_message_get_media(message, i);
        if (g_strcmp0(gst_sdp_media_get_media(section), media) != 0) {
            continue;
        }
        for (guint j = 0; j < gst_sdp_media_attributes_len(section); ++j) {
            const GstSDPAttribute *attribute = gst_sdp_media_get_attribute(section, j);
            if (g_strcmp0(attribute->key, "rtpmap") == 0 && attribute->value
                && QString::fromUtf8(attribute->value).contains(QLatin1Char(' ') + codec, Qt::CaseInsensitive)) {
                result = atoi(attribute->value);
                break;
            }
        }
    }
    gst_sdp_message_free(message);
    return result;
}

QStringList mids(const GstSDPMessage *message)
{
    QStringList result;
    for (guint i = 0; i < gst_sdp_message_medias_len(message); ++i) {
        result << QString::fromUtf8(gst_sdp_media_get_attribute_val(gst_sdp_message_get_media(message, i), "mid"));
    }
    return result;
}

} // namespace

CallEngine::CallEngine(QObject *parent)
    : QObject(parent)
{
    if (!gst_is_initialized()) {
        gst_init(nullptr, nullptr);
    }
}

CallEngine::~CallEngine()
{
    stop();
}

bool CallEngine::build(const QVariantMap &turn, bool relayOnly, int opusPayloadType, int vp8PayloadType)
{
    // Remote candidates can arrive before the user accepts.
    const QVariantList pending = m_pendingCandidates;
    stop();
    m_pendingCandidates = pending;
    m_framesSent = 0;
    m_framesReceived = 0;
    // Microphone → mono Opus, constant bitrate (Android's SdpPatcher asks for cbr=1). The
    // pulse role "phone" lets the audio policy route it like a call.
    const QString description = QStringLiteral(
        "webrtcbin name=webrtc bundle-policy=max-bundle latency=100 "
        "pulsesrc stream-properties=\"props,media.role=phone\" ! audioconvert ! audioresample "
        "! audio/x-raw,format=S16LE,channels=1,rate=48000 ! audiobuffersplit output-buffer-duration=1/100 "
        "! identity name=aec ! volume name=mic ! queue "
        "! opusenc audio-type=voice bitrate-type=cbr bitrate=32000 frame-size=20 inband-fec=true "
        "! rtpopuspay pt=%1 ! queue "
        "! application/x-rtp,media=audio,encoding-name=OPUS,clock-rate=48000,payload=%1,"
        "encoding-params=(string)2,minptime=(string)10,useinbandfec=(string)1 "
        "! webrtc. ").arg(opusPayloadType)
        // Video calls: VP8 from whatever the camera branch feeds into `vin` (nothing while the
        // camera is off; setCamera adds and removes that branch).
        + (vp8PayloadType < 0 ? QString() : QStringLiteral(
        "funnel name=vin ! queue max-size-buffers=2 leaky=downstream "
        "! vp8enc name=venc deadline=1 cpu-used=8 end-usage=cbr target-bitrate=600000 "
        "keyframe-max-dist=60 lag-in-frames=0 error-resilient=default threads=2 "
        "! rtpvp8pay pt=%1 picture-id-mode=15-bit mtu=1200 ! queue "
        "! application/x-rtp,media=video,encoding-name=VP8,clock-rate=90000,payload=%1 "
        "! webrtc. ").arg(vp8PayloadType));
    GError *error = nullptr;
    m_pipeline = gst_parse_launch(description.toUtf8().constData(), &error);
    if (error) {
        qWarning() << "call pipeline:" << error->message;
        g_error_free(error);
        if (m_pipeline) {
            gst_object_unref(m_pipeline);
            m_pipeline = nullptr;
        }
        return false;
    }
    m_webrtc = gst_bin_get_by_name(GST_BIN(m_pipeline), "webrtc");
    m_video = vp8PayloadType >= 0;
    // Nothing of ours is sent before the peer is authenticated: DTLS (and with it SRTP) is up
    // before the fingerprint can be checked, so its encryption alone could be a stranger's.
    GstIterator *sinkPads = gst_element_iterate_sink_pads(m_webrtc);
    GValue item = G_VALUE_INIT;
    while (gst_iterator_next(sinkPads, &item) == GST_ITERATOR_OK) {
        gst_pad_add_probe(GST_PAD(g_value_get_object(&item)),
                          static_cast<GstPadProbeType>(GST_PAD_PROBE_TYPE_BUFFER | GST_PAD_PROBE_TYPE_BUFFER_LIST),
                          reinterpret_cast<GstPadProbeCallback>(&CallEngine::onMediaBuffer), this, nullptr);
        g_value_reset(&item);
    }
    g_value_unset(&item);
    gst_iterator_free(sinkPads);
    m_micVolume = gst_bin_get_by_name(GST_BIN(m_pipeline), "mic");
    // Muted while there was no pipeline yet (the TURN servers take a moment).
    g_object_set(m_micVolume, "mute", m_muted ? TRUE : FALSE, nullptr);
    m_echoCanceller.reset(new EchoCanceller);
    GstElement *aec = gst_bin_get_by_name(GST_BIN(m_pipeline), "aec");
    GstPad *aecPad = gst_element_get_static_pad(aec, "src");
    gst_pad_add_probe(aecPad, GST_PAD_PROBE_TYPE_BUFFER, reinterpret_cast<GstPadProbeCallback>(&CallEngine::onCaptureBuffer), this, nullptr);
    gst_object_unref(aecPad);
    gst_object_unref(aec);
    g_object_set(m_webrtc, "ice-transport-policy",
                 relayOnly ? GST_WEBRTC_ICE_TRANSPORT_POLICY_RELAY : GST_WEBRTC_ICE_TRANSPORT_POLICY_ALL, nullptr);

    const QString user = turn.value(QStringLiteral("username")).toString();
    const QString password = turn.value(QStringLiteral("password")).toString();
    for (const QVariant &url : turn.value(QStringLiteral("urls")).toList()) {
        const QString uri = turnUri(url.toString(), user, password);
        // TCP TURN is skipped like Android's TcpCandidatePolicy.DISABLED does for candidates.
        if (uri.isEmpty() || uri.contains(QLatin1String("transport=tcp"))) {
            continue;
        }
        gboolean added = FALSE;
        g_signal_emit_by_name(m_webrtc, "add-turn-server", uri.toUtf8().constData(), &added);
    }

    g_signal_connect(m_webrtc, "on-ice-candidate", G_CALLBACK(&CallEngine::onIceCandidateCb), this);
    g_signal_connect(m_webrtc, "pad-added", G_CALLBACK(&CallEngine::onPadAdded), this);
    g_signal_connect(m_webrtc, "notify::ice-connection-state", G_CALLBACK(&CallEngine::onIceStateCb), this);
    g_signal_connect(m_webrtc, "notify::connection-state", G_CALLBACK(&CallEngine::onConnectionStateCb), this);

    GstBus *bus = gst_pipeline_get_bus(GST_PIPELINE(m_pipeline));
    m_busWatch = gst_bus_add_watch(bus, reinterpret_cast<GstBusFunc>(&CallEngine::onBusMessage), this);
    gst_object_unref(bus);

    if (gst_element_set_state(m_pipeline, GST_STATE_READY) == GST_STATE_CHANGE_FAILURE) {
        stop();
        return false;
    }
    // Our ECDSA certificate on every DTLS transport. webrtcbin creates those while it makes the
    // offer/answer and only adds them to the pipeline when it sets the local description, so
    // the certificate goes in then (long before the handshake), and the SDP fingerprint, which
    // webrtcbin computed from its default certificate, is replaced with ours before the SDP
    // leaves the engine (descriptionCreated). webrtcbin itself does not check fingerprints.
    const DtlsCertificate::Certificate certificate = DtlsCertificate::generate();
    m_certificate = certificate.pem;
    m_fingerprint = certificate.fingerprint;
    if (m_certificate.isEmpty() || m_fingerprint.isEmpty()) {
        qWarning() << "call: no DTLS certificate";
        stop();
        return false;
    }
    g_signal_connect(m_pipeline, "deep-element-added", G_CALLBACK(&CallEngine::onElementAdded), this);
    // Negotiated (no in-band setup), so both sides create it before the offer/answer.
    GstStructure *options = gst_structure_new("options", "negotiated", G_TYPE_BOOLEAN, TRUE, "id", G_TYPE_INT, 0,
                                              "ordered", G_TYPE_BOOLEAN, TRUE, nullptr);
    g_signal_emit_by_name(m_webrtc, "create-data-channel", DATA_CHANNEL_LABEL, options, &m_channel);
    gst_structure_free(options);
    if (m_channel) {
        g_signal_connect(m_channel, "on-open", G_CALLBACK(&CallEngine::onChannelOpenCb), this);
        g_signal_connect(m_channel, "on-message-data", G_CALLBACK(&CallEngine::onChannelDataCb), this);
    }
    return gst_element_set_state(m_pipeline, GST_STATE_PLAYING) != GST_STATE_CHANGE_FAILURE;
}

void CallEngine::startOffer(const QVariantMap &turn, bool relayOnly, bool video)
{
    if (!build(turn, relayOnly, OPUS_PAYLOAD_TYPE, video ? VP8_PAYLOAD_TYPE : -1)) {
        emit failed(QStringLiteral("pipeline"));
        return;
    }
    GstPromise *promise = gst_promise_new_with_change_func(&CallEngine::onOfferCreated, this, nullptr);
    g_signal_emit_by_name(m_webrtc, "create-offer", nullptr, promise);
}

void CallEngine::startAnswer(const QString &remoteOffer, const QVariantMap &turn, bool relayOnly, bool video)
{
    const QString remoteFingerprint = DtlsCertificate::sdpFingerprint(remoteOffer);
    const int payloadType = ::payloadType(remoteOffer, "audio", QStringLiteral("opus/48000"));
    // Video only if asked for and the offer has VP8 (else we only receive what comes).
    const int vp8 = video ? ::payloadType(remoteOffer, "video", QStringLiteral("VP8/90000")) : -1;
    GstSDPMessage *message = nullptr;
    if (payloadType < 0 || gst_sdp_message_new_from_text(remoteOffer.toUtf8().constData(), &message) != GST_SDP_OK) {
        emit failed(QStringLiteral("offer"));
        return;
    }
    if (!build(turn, relayOnly, payloadType, vp8)) {
        gst_sdp_message_free(message);
        emit failed(QStringLiteral("pipeline"));
        return;
    }
    m_remoteFingerprint = remoteFingerprint;
    GstWebRTCSessionDescription *offer = gst_webrtc_session_description_new(GST_WEBRTC_SDP_TYPE_OFFER, message);
    GstPromise *promise = gst_promise_new_with_change_func(&CallEngine::onRemoteSet, this, nullptr);
    g_signal_emit_by_name(m_webrtc, "set-remote-description", offer, promise);
    gst_webrtc_session_description_free(offer);
}

void CallEngine::setRemoteAnswer(const QString &sdp)
{
    m_remoteFingerprint = DtlsCertificate::sdpFingerprint(sdp);
    GstSDPMessage *message = nullptr;
    if (!m_webrtc || gst_sdp_message_new_from_text(sdp.toUtf8().constData(), &message) != GST_SDP_OK) {
        emit failed(QStringLiteral("answer"));
        return;
    }
    GstWebRTCSessionDescription *answer = gst_webrtc_session_description_new(GST_WEBRTC_SDP_TYPE_ANSWER, message);
    GstPromise *promise = gst_promise_new_with_change_func(&CallEngine::onRemoteSet, this, nullptr);
    g_signal_emit_by_name(m_webrtc, "set-remote-description", answer, promise);
    gst_webrtc_session_description_free(answer);
}

void CallEngine::addRemoteCandidates(const QVariantList &candidates)
{
    for (const QVariant &candidate : candidates) {
        if (m_remoteSet) {
            addCandidate(candidate.toMap());
        } else {
            m_pendingCandidates << candidate;
        }
    }
}

void CallEngine::addCandidate(const QVariantMap &candidate)
{
    const QString text = candidate.value(QStringLiteral("candidate")).toString();
    if (!m_webrtc || text.isEmpty()) {
        return;
    }
    int mline = candidate.value(QStringLiteral("sdpMLineIndex"), -1).toInt();
    if (mline < 0) {
        mline = m_mids.indexOf(candidate.value(QStringLiteral("sdpMid")).toString());
    }
    // With max-bundle every m-line shares one transport: the first is as good as any.
    g_signal_emit_by_name(m_webrtc, "add-ice-candidate", static_cast<guint>(qMax(mline, 0)), text.toUtf8().constData());
}

void CallEngine::setMuted(bool muted)
{
    m_muted = muted;
    if (m_micVolume) {
        g_object_set(m_micVolume, "mute", muted ? TRUE : FALSE, nullptr);
    }
}

void CallEngine::sendChannel(const QByteArray &data)
{
    if (!m_channel) {
        return;
    }
    if (!m_channelOpen || !m_verified) {
        m_pendingChannel << data;
        return;
    }
    GBytes *bytes = g_bytes_new(data.constData(), static_cast<gsize>(data.size()));
    GError *error = nullptr;
    if (!gst_webrtc_data_channel_send_data_full(m_channel, bytes, &error)) {
        qWarning() << "call data channel:" << (error ? error->message : "send failed");
        g_clear_error(&error);
    }
    g_bytes_unref(bytes);
}

void CallEngine::stop()
{
    m_pendingCandidates.clear();
    m_pendingChannel.clear();
    m_receivedChannel.clear();
    m_channelOpen = false;
    // Our copy of this call's DTLS private key (PEM). GStreamer's own copies (the elements' `pem`
    // and the DTLS agents it keeps per PEM) are out of our reach and not cleared; the key is good
    // for this call only.
    m_certificate.fill('\0');
    m_certificate.clear();
    if (!m_pipeline) {
        return;
    }
    if (m_busWatch) {
        g_source_remove(m_busWatch);
        m_busWatch = 0;
    }
    qInfo() << "call audio frames: sent" << m_framesSent << "received" << m_framesReceived;
    removeCamera();
    m_video = false;
    g_signal_handlers_disconnect_by_data(m_webrtc, this);
    gst_element_set_state(m_pipeline, GST_STATE_NULL);
    if (m_channel) {
        g_signal_handlers_disconnect_by_data(m_channel, this);
        g_object_unref(m_channel);
        m_channel = nullptr;
    }
    gst_object_unref(m_micVolume);
    gst_object_unref(m_webrtc);
    gst_object_unref(m_pipeline);
    // After the pipeline stopped: no probe runs any more.
    m_echoCanceller.reset();
    m_micVolume = nullptr;
    m_webrtc = nullptr;
    m_pipeline = nullptr;
    m_remoteSet = false;
    m_connected = false;
    m_verified = false;
    m_pendingCandidates.clear();
    m_mids.clear();
    m_remoteFingerprint.clear();
    // The pipeline's threads are gone. What they queued belongs to this call: an offer or error
    // must not reach the next one, nor the manager after the user hung up.
    QCoreApplication::removePostedEvents(this, QEvent::MetaCall);
    m_remoteFramePending = false;
}

// --- GStreamer threads: hand everything over to this object's thread. ---

void CallEngine::onOfferCreated(GstPromise *promise, void *self)
{
    static_cast<CallEngine *>(self)->descriptionCreated(promise, "offer", QStringLiteral("offer"));
}

void CallEngine::onAnswerCreated(GstPromise *promise, void *self)
{
    static_cast<CallEngine *>(self)->descriptionCreated(promise, "answer", QStringLiteral("answer"));
}

void CallEngine::descriptionCreated(GstPromise *promise, const char *field, const QString &type)
{
    GstWebRTCSessionDescription *description = nullptr;
    if (gst_promise_wait(promise) == GST_PROMISE_RESULT_REPLIED) {
        gst_structure_get(gst_promise_get_reply(promise), field, GST_TYPE_WEBRTC_SESSION_DESCRIPTION, &description,
                          nullptr);
    }
    gst_promise_unref(promise);
    if (!description) {
        QMetaObject::invokeMethod(this, "onError", Qt::QueuedConnection, Q_ARG(QString, type));
        return;
    }
    if (!m_webrtc) {
        gst_webrtc_session_description_free(description);
        return;
    }
    g_signal_emit_by_name(m_webrtc, "set-local-description", description, nullptr);
    gchar *text = gst_sdp_message_as_text(description->sdp);
    QString sdp = QString::fromUtf8(text);
    sdp.replace(QRegularExpression(QStringLiteral("a=fingerprint:[^\r\n]*")),
                QStringLiteral("a=fingerprint:") + m_fingerprint);
    g_free(text);
    gst_webrtc_session_description_free(description);
    QMetaObject::invokeMethod(this, "onLocalDescription", Qt::QueuedConnection, Q_ARG(QString, type),
                              Q_ARG(QString, sdp));
}

void CallEngine::onRemoteSet(GstPromise *promise, void *self)
{
    auto *engine = static_cast<CallEngine *>(self);
    // Success replies without a structure; failures carry an "error".
    const bool replied = gst_promise_wait(promise) == GST_PROMISE_RESULT_REPLIED;
    const GstStructure *reply = replied ? gst_promise_get_reply(promise) : nullptr;
    QString error;
    if (!replied) {
        error = QStringLiteral("no reply");
    } else if (reply && gst_structure_has_field(reply, "error")) {
        GError *gerror = nullptr;
        gst_structure_get(reply, "error", G_TYPE_ERROR, &gerror, nullptr);
        error = gerror ? QString::fromUtf8(gerror->message) : QStringLiteral("unknown");
        if (gerror) {
            g_error_free(gerror);
        }
    }
    gst_promise_unref(promise);
    if (error.isEmpty()) {
        QMetaObject::invokeMethod(engine, "onRemoteDescriptionSet", Qt::QueuedConnection);
    } else {
        QMetaObject::invokeMethod(engine, "onError", Qt::QueuedConnection,
                                  Q_ARG(QString, QStringLiteral("remote description: ") + error));
    }
}

void CallEngine::onChannelOpenCb(GstWebRTCDataChannel *, void *self)
{
    QMetaObject::invokeMethod(static_cast<CallEngine *>(self), "onChannelOpen", Qt::QueuedConnection);
}

void CallEngine::onChannelDataCb(GstWebRTCDataChannel *, GBytes *data, void *self)
{
    gsize size = 0;
    const auto *bytes = static_cast<const char *>(g_bytes_get_data(data, &size));
    const QByteArray message(bytes, static_cast<int>(size));
    QMetaObject::invokeMethod(static_cast<CallEngine *>(self), "onChannelData", Qt::QueuedConnection,
                              Q_ARG(QByteArray, message));
}

void CallEngine::onIceCandidateCb(GstElement *, unsigned mline, char *candidate, void *self)
{
    QMetaObject::invokeMethod(static_cast<CallEngine *>(self), "onIceCandidate", Qt::QueuedConnection,
                              Q_ARG(uint, mline), Q_ARG(QString, QString::fromUtf8(candidate)));
}

void CallEngine::onIceStateCb(GstElement *webrtc, GParamSpec *, void *self)
{
    GstWebRTCICEConnectionState state;
    g_object_get(webrtc, "ice-connection-state", &state, nullptr);
    QMetaObject::invokeMethod(static_cast<CallEngine *>(self), "onIceState", Qt::QueuedConnection,
                              Q_ARG(int, static_cast<int>(state)));
}

void CallEngine::onConnectionStateCb(GstElement *webrtc, GParamSpec *, void *self)
{
    GstWebRTCPeerConnectionState state;
    g_object_get(webrtc, "connection-state", &state, nullptr);
    QMetaObject::invokeMethod(static_cast<CallEngine *>(self), "onConnectionState", Qt::QueuedConnection,
                              Q_ARG(int, static_cast<int>(state)));
}

void CallEngine::onPadAdded(GstElement *, GstPad *pad, void *self)
{
    auto *engine = static_cast<CallEngine *>(self);
    if (GST_PAD_DIRECTION(pad) != GST_PAD_SRC) {
        return;
    }
    // Received media only once the peer is authenticated (see build()).
    gst_pad_add_probe(pad, static_cast<GstPadProbeType>(GST_PAD_PROBE_TYPE_BUFFER | GST_PAD_PROBE_TYPE_BUFFER_LIST),
                      reinterpret_cast<GstPadProbeCallback>(&CallEngine::onMediaBuffer), engine, nullptr);
    GstCaps *caps = gst_pad_get_current_caps(pad);
    if (!caps) {
        caps = gst_pad_query_caps(pad, nullptr);
    }
    const GstStructure *structure = gst_caps_get_structure(caps, 0);
    const bool opus = g_strcmp0(gst_structure_get_string(structure, "media"), "audio") == 0
        && g_ascii_strcasecmp(gst_structure_get_string(structure, "encoding-name"), "OPUS") == 0;
    const bool vp8 = g_strcmp0(gst_structure_get_string(structure, "media"), "video") == 0
        && g_ascii_strcasecmp(gst_structure_get_string(structure, "encoding-name"), "VP8") == 0;
    gst_caps_unref(caps);
    // Received audio → speaker/earpiece; VP8 video → frames for the call page; anything else is
    // dropped.
    const char *description = opus
        ? "queue ! rtpopusdepay ! opusdec plc=true use-inband-fec=true ! audioconvert ! audioresample "
          "! audio/x-raw,format=S16LE,channels=1,rate=48000 ! audiobuffersplit output-buffer-duration=1/100 "
          "! identity name=aecplayback ! pulsesink stream-properties=\"props,media.role=phone\""
        : vp8 ? "queue ! rtpvp8depay name=depay ! vp8dec threads=2 ! videoconvert ! video/x-raw,format=BGRx "
                "! appsink name=remotevideo emit-signals=true sync=false max-buffers=1 drop=true"
              : "fakesink";
    GError *error = nullptr;
    GstElement *bin = gst_parse_bin_from_description(description, TRUE, &error);
    if (error) {
        qWarning() << "call receive pipeline:" << error->message;
        g_error_free(error);
        return;
    }
    if (vp8) {
        GstElement *sink = gst_bin_get_by_name(GST_BIN(bin), "remotevideo");
        g_signal_connect(sink, "new-sample", G_CALLBACK(&CallEngine::onRemoteSample), engine);
        gst_object_unref(sink);
        GstElement *depay = gst_bin_get_by_name(GST_BIN(bin), "depay");
        GstPad *depaySrc = gst_element_get_static_pad(depay, "src");
        gst_pad_add_probe(depaySrc, GST_PAD_PROBE_TYPE_BUFFER, reinterpret_cast<GstPadProbeCallback>(&CallEngine::onVp8Frame),
                          new Vp8Filter, [](gpointer filter) { delete static_cast<Vp8Filter *>(filter); });
        gst_object_unref(depaySrc);
        gst_object_unref(depay);
    }
    if (opus) {
        GstElement *playback = gst_bin_get_by_name(GST_BIN(bin), "aecplayback");
        GstPad *playbackPad = gst_element_get_static_pad(playback, "src");
        gst_pad_add_probe(playbackPad, GST_PAD_PROBE_TYPE_BUFFER, reinterpret_cast<GstPadProbeCallback>(&CallEngine::onPlaybackBuffer), engine, nullptr);
        gst_object_unref(playbackPad);
        gst_object_unref(playback);
    }
    gst_bin_add(GST_BIN(engine->m_pipeline), bin);
    gst_element_sync_state_with_parent(bin);
    GstPad *sink = gst_element_get_static_pad(bin, "sink");
    gst_pad_link(pad, sink);
    gst_object_unref(sink);
}

// The microphone's 10 ms buffers, cleaned of the far end's echo (and noise) in place.
int CallEngine::onCaptureBuffer(GstPad *, GstPadProbeInfo *info, void *self)
{
    EchoCanceller *aec = static_cast<CallEngine *>(self)->m_echoCanceller.data();
    GstBuffer *buffer = gst_buffer_make_writable(gst_pad_probe_info_get_buffer(info));
    GST_PAD_PROBE_INFO_DATA(info) = buffer;
    GstMapInfo map;
    if (aec && gst_buffer_map(buffer, &map, GST_MAP_READWRITE)) {
        auto *samples = reinterpret_cast<int16_t *>(map.data);
        const gsize count = map.size / sizeof(int16_t);
        for (gsize i = 0; i + EchoCanceller::FRAME_SAMPLES <= count; i += EchoCanceller::FRAME_SAMPLES) {
            aec->capture(samples + i);
            ++static_cast<CallEngine *>(self)->m_framesSent;
        }
        gst_buffer_unmap(buffer, &map);
    }
    return GST_PAD_PROBE_OK;
}

// Media into and out of webrtcbin: dropped until the peer's certificate matched (dropped, not
// blocked: the handshake goes on meanwhile).
int CallEngine::onMediaBuffer(GstPad *, GstPadProbeInfo *, void *self)
{
    return static_cast<CallEngine *>(self)->m_verified ? GST_PAD_PROBE_OK : GST_PAD_PROBE_DROP;
}

int CallEngine::onVp8Frame(GstPad *, GstPadProbeInfo *info, void *filter)
{
    return static_cast<Vp8Filter *>(filter)->passes(gst_pad_probe_info_get_buffer(info)) ? GST_PAD_PROBE_OK
                                                                                         : GST_PAD_PROBE_DROP;
}

// What is about to be played: the reference the echo is removed against.
int CallEngine::onPlaybackBuffer(GstPad *, GstPadProbeInfo *info, void *self)
{
    EchoCanceller *aec = static_cast<CallEngine *>(self)->m_echoCanceller.data();
    GstBuffer *buffer = gst_pad_probe_info_get_buffer(info);
    GstMapInfo map;
    if (aec && gst_buffer_map(buffer, &map, GST_MAP_READ)) {
        const auto *samples = reinterpret_cast<const int16_t *>(map.data);
        const gsize count = map.size / sizeof(int16_t);
        for (gsize i = 0; i + EchoCanceller::FRAME_SAMPLES <= count; i += EchoCanceller::FRAME_SAMPLES) {
            aec->playback(samples + i);
            ++static_cast<CallEngine *>(self)->m_framesReceived;
        }
        gst_buffer_unmap(buffer, &map);
    }
    return GST_PAD_PROBE_OK;
}

namespace {

// A BGRx video sample as an image (a copy: the buffer goes back to GStreamer).
QImage sampleImage(GstAppSink *sink)
{
    GstSample *sample = gst_app_sink_pull_sample(sink);
    if (!sample) {
        return QImage();
    }
    QImage image;
    GstVideoInfo info;
    GstMapInfo map;
    GstBuffer *buffer = gst_sample_get_buffer(sample);
    if (gst_video_info_from_caps(&info, gst_sample_get_caps(sample))
        && gst_buffer_map(buffer, &map, GST_MAP_READ)) {
        image = QImage(map.data, GST_VIDEO_INFO_WIDTH(&info), GST_VIDEO_INFO_HEIGHT(&info),
                       GST_VIDEO_INFO_PLANE_STRIDE(&info, 0), QImage::Format_RGB32)
                    .copy();
        gst_buffer_unmap(buffer, &map);
    }
    gst_sample_unref(sample);
    return image;
}

} // namespace

int CallEngine::onRemoteSample(GstElement *sink, void *self)
{
    auto *engine = static_cast<CallEngine *>(self);
    if (engine->m_remoteFramePending.exchange(true)) {
        // The GUI is behind: pictures must not pile up in its queue.
        GstSample *sample = gst_app_sink_pull_sample(GST_APP_SINK(sink));
        if (sample) {
            gst_sample_unref(sample);
        }
        return GST_FLOW_OK;
    }
    const QImage image = sampleImage(GST_APP_SINK(sink));
    if (image.isNull()) {
        engine->m_remoteFramePending = false;
        return GST_FLOW_OK;
    }
    QMetaObject::invokeMethod(engine, "onRemoteFrame", Qt::QueuedConnection, Q_ARG(QImage, image));
    return GST_FLOW_OK;
}

int CallEngine::onLocalSample(GstElement *sink, void *self)
{
    const QImage image = sampleImage(GST_APP_SINK(sink));
    if (!image.isNull()) {
        QMetaObject::invokeMethod(static_cast<CallEngine *>(self), "localFrame", Qt::QueuedConnection,
                                  Q_ARG(QImage, image));
    }
    return GST_FLOW_OK;
}

// The camera branch: the camera (on a phone droidcamsrc, front or back; in the emulator, which
// has none, a test pattern) scaled to what the profile allows, into the encoder and the preview.
void CallEngine::setCamera(bool on, bool front)
{
    if (!m_pipeline || !m_video || on == (m_camera != nullptr)) {
        return;
    }
    if (!on) {
        removeCamera();
        return;
    }
    GstElementFactory *droid = gst_element_factory_find("droidcamsrc");
    const QString source = droid ? QStringLiteral("droidcamsrc camera-device=%1 mode=2").arg(front ? 1 : 0)
                                 : QStringLiteral("videotestsrc is-live=true pattern=ball");
    if (droid) {
        gst_object_unref(droid);
    }
    const QString description = source + QStringLiteral(
        " ! videoconvert ! videoscale ! videorate ! capsfilter name=camcaps caps=%1 ! tee name=camtee "
        "camtee. ! queue max-size-buffers=2 leaky=downstream name=camout "
        "camtee. ! queue max-size-buffers=1 leaky=downstream ! videoconvert ! video/x-raw,format=BGRx "
        "! appsink name=localvideo emit-signals=true sync=false max-buffers=1 drop=true")
                                    .arg(cameraCaps());
    GError *error = nullptr;
    GstElement *bin = gst_parse_bin_from_description(description.toUtf8().constData(), FALSE, &error);
    if (error) {
        qWarning() << "call camera:" << error->message;
        g_error_free(error);
        return;
    }
    GstElement *out = gst_bin_get_by_name(GST_BIN(bin), "camout");
    GstPad *outPad = gst_element_get_static_pad(out, "src");
    gst_element_add_pad(bin, gst_ghost_pad_new("src", outPad));
    gst_object_unref(outPad);
    gst_object_unref(out);
    GstElement *preview = gst_bin_get_by_name(GST_BIN(bin), "localvideo");
    g_signal_connect(preview, "new-sample", G_CALLBACK(&CallEngine::onLocalSample), this);
    gst_object_unref(preview);

    gst_bin_add(GST_BIN(m_pipeline), bin);
    GstElement *funnel = gst_bin_get_by_name(GST_BIN(m_pipeline), "vin");
    m_cameraPad = gst_element_request_pad_simple(funnel, "sink_%u");
    gst_object_unref(funnel);
    GstPad *binPad = gst_element_get_static_pad(bin, "src");
    gst_pad_link(binPad, m_cameraPad);
    gst_object_unref(binPad);
    gst_element_sync_state_with_parent(bin);
    m_camera = bin;
}

void CallEngine::removeCamera()
{
    if (!m_camera) {
        return;
    }
    GstElement *bin = m_camera;
    m_camera = nullptr;
    // No EOS (it would end the encoder's stream for good): unlink, then stop the branch.
    GstPad *binPad = gst_element_get_static_pad(bin, "src");
    gst_pad_unlink(binPad, m_cameraPad);
    gst_object_unref(binPad);
    if (m_pipeline) {
        GstElement *funnel = gst_bin_get_by_name(GST_BIN(m_pipeline), "vin");
        gst_element_release_request_pad(funnel, m_cameraPad);
        gst_object_unref(funnel);
    }
    gst_object_unref(m_cameraPad);
    m_cameraPad = nullptr;
    gst_element_set_state(bin, GST_STATE_NULL);
    if (m_pipeline) {
        gst_bin_remove(GST_BIN(m_pipeline), bin);
    }
}

// What the encoder may send (Android's quality profiles, capped for software VP8).
void CallEngine::setVideoParams(int bitrateKbps, int width, int height, int fps)
{
    m_videoWidth = qMin(width, MAX_VIDEO_WIDTH);
    m_videoHeight = qMin(height, MAX_VIDEO_HEIGHT);
    m_videoFps = fps;
    if (!m_pipeline || !m_video) {
        return;
    }
    GstElement *encoder = gst_bin_get_by_name(GST_BIN(m_pipeline), "venc");
    if (encoder) {
        g_object_set(encoder, "target-bitrate", bitrateKbps * 1000, nullptr);
        gst_object_unref(encoder);
    }
    if (m_camera) {
        GstElement *filter = gst_bin_get_by_name(GST_BIN(m_camera), "camcaps");
        GstCaps *caps = gst_caps_from_string(cameraCaps().toUtf8().constData());
        g_object_set(filter, "caps", caps, nullptr);
        gst_caps_unref(caps);
        gst_object_unref(filter);
    }
}

QString CallEngine::cameraCaps() const
{
    return QStringLiteral("video/x-raw,format=I420,width=%1,height=%2,framerate=%3/1")
        .arg(m_videoWidth)
        .arg(m_videoHeight)
        .arg(m_videoFps);
}

// A DTLS decoder (its transport's certificate) gets our certificate; it can arrive inside a
// bin that is added as a whole.
void CallEngine::onElementAdded(GstBin *, GstBin *, GstElement *element, void *self)
{
    auto *engine = static_cast<CallEngine *>(self);
    auto configure = [engine](GstElement *candidate) {
        GstElementFactory *factory = gst_element_get_factory(candidate);
        if (factory && g_strcmp0(GST_OBJECT_NAME(factory), "dtlssrtpdec") == 0
            && !g_object_get_data(G_OBJECT(candidate), "threesea-certificate")) {
            g_object_set(candidate, "pem", engine->m_certificate.constData(), nullptr);
            g_object_set_data(G_OBJECT(candidate), "threesea-certificate", GINT_TO_POINTER(1));
        }
    };
    configure(element);
    if (GST_IS_BIN(element)) {
        GstIterator *children = gst_bin_iterate_recurse(GST_BIN(element));
        GValue item = G_VALUE_INIT;
        while (gst_iterator_next(children, &item) == GST_ITERATOR_OK) {
            configure(GST_ELEMENT(g_value_get_object(&item)));
            g_value_reset(&item);
        }
        g_value_unset(&item);
        gst_iterator_free(children);
    }
}

int CallEngine::onBusMessage(GstBus *, GstMessage *message, void *self)
{
    if (GST_MESSAGE_TYPE(message) == GST_MESSAGE_ERROR) {
        GError *error = nullptr;
        gst_message_parse_error(message, &error, nullptr);
        qWarning() << "call pipeline error:" << (error ? error->message : "");
        if (error) {
            g_error_free(error);
        }
        static_cast<CallEngine *>(self)->onError(QStringLiteral("media"));
    }
    return TRUE;
}

// --- This object's thread. ---

void CallEngine::onLocalDescription(const QString &type, const QString &sdp)
{
    GstSDPMessage *message = nullptr;
    if (gst_sdp_message_new_from_text(sdp.toUtf8().constData(), &message) == GST_SDP_OK) {
        m_mids = mids(message);
        gst_sdp_message_free(message);
    }
    emit localDescription(type, sdp);
}

void CallEngine::onIceCandidate(uint mline, const QString &candidate)
{
    if (candidate.isEmpty()) {
        return;
    }
    // libnice also offers TCP candidates; Android disables them (bad with retransmissions).
    if (candidate.contains(QLatin1String(" tcp "), Qt::CaseInsensitive)) {
        return;
    }
    QVariantMap result;
    result.insert(QStringLiteral("candidate"), candidate.startsWith(QLatin1String("a="))
                                                    ? candidate.mid(2) : candidate);
    result.insert(QStringLiteral("sdpMid"), m_mids.value(static_cast<int>(mline)));
    result.insert(QStringLiteral("sdpMLineIndex"), mline);
    const int ufrag = candidate.indexOf(QLatin1String(" ufrag "));
    if (ufrag >= 0) {
        result.insert(QStringLiteral("ufrag"), candidate.mid(ufrag + 7).section(QLatin1Char(' '), 0, 0));
    }
    emit localCandidate(result);
}

void CallEngine::onRemoteDescriptionSet()
{
    if (!m_webrtc) {
        return;
    }
    m_remoteSet = true;
    const QVariantList pending = m_pendingCandidates;
    m_pendingCandidates.clear();
    for (const QVariant &candidate : pending) {
        addCandidate(candidate.toMap());
    }
    GstWebRTCSessionDescription *remote = nullptr;
    g_object_get(m_webrtc, "remote-description", &remote, nullptr);
    const bool offer = remote && remote->type == GST_WEBRTC_SDP_TYPE_OFFER;
    if (remote) {
        gst_webrtc_session_description_free(remote);
    }
    if (offer) {
        GstPromise *promise = gst_promise_new_with_change_func(&CallEngine::onAnswerCreated, this, nullptr);
        g_signal_emit_by_name(m_webrtc, "create-answer", nullptr, promise);
    }
}

void CallEngine::onIceState(int state)
{
    if (!m_pipeline) {
        return;
    }
    if (state == GST_WEBRTC_ICE_CONNECTION_STATE_FAILED) {
        onError(QStringLiteral("ice"));
    }
}

void CallEngine::onConnectionState(int state)
{
    if (!m_pipeline) {
        return;
    }
    if (state == GST_WEBRTC_PEER_CONNECTION_STATE_CONNECTED && !m_connected) {
        if (!remoteCertificateMatches()) {
            onError(QStringLiteral("certificate"));
            return;
        }
        m_connected = true;
        m_verified = true;
        // Channel messages held back until now.
        flushChannel();
        emit connected();
    } else if (state == GST_WEBRTC_PEER_CONNECTION_STATE_FAILED) {
        onError(QStringLiteral("connection"));
    }
}

// The peer's DTLS certificate is the one its (end-to-end encrypted) SDP named. webrtcbin does
// not check this; without it anyone on the media path could take the call over (libwebrtc,
// and so Android, checks it).
bool CallEngine::remoteCertificateMatches()
{
    if (m_remoteFingerprint.isEmpty() || !m_remoteFingerprint.startsWith(QLatin1String("sha-256 "))) {
        qWarning() << "call: remote fingerprint missing or not sha-256";
        return false;
    }
    bool checked = false;
    bool matches = true;
    GArray *transceivers = nullptr;
    g_signal_emit_by_name(m_webrtc, "get-transceivers", &transceivers);
    for (guint i = 0; transceivers && i < transceivers->len; ++i) {
        GstWebRTCRTPTransceiver *transceiver = g_array_index(transceivers, GstWebRTCRTPTransceiver *, i);
        GstWebRTCRTPSender *sender = nullptr;
        g_object_get(transceiver, "sender", &sender, nullptr);
        GstWebRTCDTLSTransport *transport = nullptr;
        if (sender) {
            g_object_get(sender, "transport", &transport, nullptr);
            gst_object_unref(sender);
        }
        if (!transport) {
            continue;
        }
        gchar *pem = nullptr;
        g_object_get(transport, "remote-certificate", &pem, nullptr);
        gst_object_unref(transport);
        if (!pem) {
            matches = false;
            continue;
        }
        const QString actual = DtlsCertificate::fingerprintOfPem(QByteArray(pem));
        g_free(pem);
        checked = true;
        if (actual != m_remoteFingerprint) {
            matches = false;
        }
    }
    if (transceivers) {
        g_array_unref(transceivers);
    }
    if (!checked || !matches) {
        qWarning() << "call: the peer's DTLS certificate does not match its fingerprint";
    }
    return checked && matches;
}

void CallEngine::onChannelOpen()
{
    if (!m_channel) {
        return;
    }
    m_channelOpen = true;
    flushChannel();
}

void CallEngine::onChannelData(const QByteArray &data)
{
    if (!m_verified) {
        m_receivedChannel << data;
        return;
    }
    emit channelMessage(data);
}

// What was sent or received before the channel was open and the peer authenticated.
void CallEngine::flushChannel()
{
    if (m_verified) {
        const QList<QByteArray> received = m_receivedChannel;
        m_receivedChannel.clear();
        for (const QByteArray &data : received) {
            emit channelMessage(data);
        }
    }
    if (!m_channelOpen || !m_verified) {
        return;
    }
    const QList<QByteArray> pending = m_pendingChannel;
    m_pendingChannel.clear();
    for (const QByteArray &data : pending) {
        sendChannel(data);
    }
}

void CallEngine::onRemoteFrame(const QImage &image)
{
    m_remoteFramePending = false;
    emit remoteFrame(image);
}

void CallEngine::onError(const QString &reason)
{
    if (!m_pipeline) {
        return;
    }
    stop();
    emit failed(reason);
}
