// SPDX-License-Identifier: AGPL-3.0-only
// GStreamer first: GLib headers use `signals` as a member name, which Qt defines as a macro.
#define GST_USE_UNSTABLE_API
#include <gst/gst.h>
#include <gst/sdp/sdp.h>
#include <gst/webrtc/webrtc.h>
#include <gst/rtp/rtp.h>
#include <gst/app/gstappsink.h>
#include <gst/app/gstappsrc.h>
#include <gst/video/video.h>
#include <nice/agent.h>
#include <openssl/hmac.h>
#include <openssl/rand.h>

#include "groupcallengine.h"
#include "aec/echocanceller.h"
#include "dtlscertificate.h"
#include "vp8filter.h"

#include <QCoreApplication>
#include <QDebug>
#include <QImage>
#include <QTransform>

#include <atomic>
#include <cstring>
#include <vector>
#include <QMutexLocker>
#include <QRegularExpression>

#include <threesea_core.h>

namespace {

// Android's SFU data channel (`DataChannelType`): label "p2s", negotiated, id 0, ordered.
const char DATA_CHANNEL_LABEL[] = "p2s";

// Frames through the crypto (logged when the call ends): encrypted, dropped; decrypted, dropped.
std::atomic<unsigned> encrypted{0}, encryptDropped{0}, decrypted{0}, decryptDropped{0};

// Codec numbers of threesea_group_call_encrypt/decrypt.
const uint8_t CODEC_OPUS = 0;
const uint8_t CODEC_VP8 = 1;

// Our video: VP8 as the SFU expects it (payload type 96), at most this much RTP payload per
// packet (room for the header, its extensions and SRTP's tag within a 1200-byte MTU).
const int VP8_PAYLOAD_TYPE = 96;
const gsize VP8_MAX_PAYLOAD = 1100;
// Our camera, and the simulcast layers sent from it like Android's (docs/research/group-calls.md
// §2.2): the RID, a fraction of the camera's size, the bitrate.
const char CAMERA_CAPS[] = "video/x-raw,format=I420,width=640,height=360,framerate=20/1";
const int CAMERA_WIDTH = 640;
const int CAMERA_HEIGHT = 360;
struct SimulcastLayer
{
    const char *rid;
    int scaleDown;
    int bitrate;
};
const SimulcastLayer SIMULCAST[GroupCallEngine::VIDEO_LAYERS] = {
    {"l", 4, 100000},
    {"m", 2, 250000},
    {"h", 1, 600000},
};

// A frame through the core's frame encryption (`participant` < 0: ours, encrypt); a new buffer
// with the same timing, or nullptr to drop the frame.
GstBuffer *transformFrame(GstBuffer *buffer, qint64 participant, uint8_t codec)
{
    GstMapInfo map;
    if (!gst_buffer_map(buffer, &map, GST_MAP_READ)) {
        return nullptr;
    }
    const gsize capacity = map.size + 22;
    GstBuffer *out = gst_buffer_new_allocate(nullptr, capacity, nullptr);
    GstMapInfo outMap;
    ptrdiff_t written = -1;
    if (gst_buffer_map(out, &outMap, GST_MAP_WRITE)) {
        written = participant < 0
            ? threesea_group_call_encrypt(codec, map.data, map.size, outMap.data, outMap.size)
            : threesea_group_call_decrypt(static_cast<uint32_t>(participant), codec, map.data, map.size,
                                          outMap.data, outMap.size);
        gst_buffer_unmap(out, &outMap);
    }
    gst_buffer_unmap(buffer, &map);
    if (written < 0) {
        ++(participant < 0 ? encryptDropped : decryptDropped);
        gst_buffer_unref(out);
        return nullptr;
    }
    ++(participant < 0 ? encrypted : decrypted);
    gst_buffer_set_size(out, written);
    gst_buffer_copy_into(out, buffer, GST_BUFFER_COPY_METADATA, 0, -1);
    return out;
}

// What the remote offer says: our microphone's and camera's MIDs (the first two m-lines) and the
// IDs of the header extensions we write or read.
struct OfferInfo
{
    QString microphoneMid;
    QString cameraMid;
    int midExtension = -1;
    int ridExtension = -1;
    int orientationExtension = -1;
};

int extensionId(const QString &sdp, const QString &uri)
{
    const QRegularExpressionMatch match =
        QRegularExpression(QStringLiteral("a=extmap:(\\d+) %1(\\s|$)").arg(QRegularExpression::escape(uri))).match(sdp);
    return match.hasMatch() ? match.captured(1).toInt() : -1;
}

OfferInfo offerInfo(const QString &sdp)
{
    OfferInfo info;
    QRegularExpressionMatchIterator mids = QRegularExpression(QStringLiteral("a=mid:(\\S+)")).globalMatch(sdp);
    if (mids.hasNext()) {
        info.microphoneMid = mids.next().captured(1);
    }
    if (mids.hasNext()) {
        info.cameraMid = mids.next().captured(1);
    }
    info.midExtension = extensionId(sdp, QStringLiteral("urn:ietf:params:rtp-hdrext:sdes:mid"));
    info.ridExtension = extensionId(sdp, QStringLiteral("urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id"));
    info.orientationExtension = extensionId(sdp, QStringLiteral("urn:3gpp:video-orientation"));
    return info;
}

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

QString sdpValue(const QString &sdp, const QString &key)
{
    const QRegularExpressionMatch match =
        QRegularExpression(QStringLiteral("a=%1:(\\S+)").arg(QRegularExpression::escape(key))).match(sdp);
    return match.hasMatch() ? match.captured(1) : QString();
}

quint32 crc32(const QByteArray &data)
{
    quint32 crc = 0xffffffff;
    for (const char byte : data) {
        crc ^= static_cast<quint8>(byte);
        for (int bit = 0; bit < 8; ++bit) {
            crc = (crc >> 1) ^ (0xedb88320 & (0 - (crc & 1)));
        }
    }
    return ~crc;
}

void appendU16(QByteArray &out, quint16 value)
{
    out.append(static_cast<char>(value >> 8));
    out.append(static_cast<char>(value & 0xff));
}

void appendAttribute(QByteArray &out, quint16 type, const QByteArray &value)
{
    appendU16(out, type);
    appendU16(out, static_cast<quint16>(value.size()));
    out.append(value);
    while (out.size() % 4) {
        out.append('\0');
    }
}

// An RFC 7675 consent check: a STUN binding request authenticated with the SFU's short-term
// credentials (USERNAME "remote:local", MESSAGE-INTEGRITY under the remote password), as the
// controlling agent, with FINGERPRINT.
QByteArray consentRequest(const QString &remoteUfrag, const QString &localUfrag, const QString &remotePassword)
{
    QByteArray message;
    appendU16(message, 0x0001); // binding request
    appendU16(message, 0);      // length, set below
    message.append("\x21\x12\xa4\x42", 4);
    unsigned char transaction[12];
    RAND_bytes(transaction, sizeof transaction);
    message.append(reinterpret_cast<const char *>(transaction), sizeof transaction);
    appendAttribute(message, 0x0006, (remoteUfrag + QLatin1Char(':') + localUfrag).toUtf8()); // USERNAME
    QByteArray tiebreaker(8, '\0');
    RAND_bytes(reinterpret_cast<unsigned char *>(tiebreaker.data()), tiebreaker.size());
    appendAttribute(message, 0x802a, tiebreaker); // ICE-CONTROLLING
    QByteArray priority(4, '\0');
    priority[0] = 0x6e;
    priority[1] = 0x7f;
    priority[2] = static_cast<char>(0xff);
    priority[3] = static_cast<char>(0xff);
    appendAttribute(message, 0x0024, priority); // PRIORITY
    // MESSAGE-INTEGRITY over everything before it, the length already counting it.
    auto setLength = [&message](int extra) {
        const int length = message.size() - 20 + extra;
        message[2] = static_cast<char>(length >> 8);
        message[3] = static_cast<char>(length & 0xff);
    };
    setLength(24);
    const QByteArray key = remotePassword.toUtf8();
    unsigned char mac[20];
    unsigned int macLength = 0;
    HMAC(EVP_sha1(), key.constData(), key.size(), reinterpret_cast<const unsigned char *>(message.constData()),
         static_cast<size_t>(message.size()), mac, &macLength);
    appendAttribute(message, 0x0008, QByteArray(reinterpret_cast<const char *>(mac), static_cast<int>(macLength)));
    setLength(8);
    const quint32 fingerprint = crc32(message) ^ 0x5354554e;
    QByteArray value(4, '\0');
    value[0] = static_cast<char>(fingerprint >> 24);
    value[1] = static_cast<char>(fingerprint >> 16);
    value[2] = static_cast<char>(fingerprint >> 8);
    value[3] = static_cast<char>(fingerprint);
    appendAttribute(message, 0x8028, value); // FINGERPRINT
    return message;
}

GstWebRTCSessionDescription *description(GstWebRTCSDPType type, const QString &sdp)
{
    GstSDPMessage *message = nullptr;
    if (gst_sdp_message_new_from_text(sdp.toUtf8().constData(), &message) != GST_SDP_OK) {
        return nullptr;
    }
    return gst_webrtc_session_description_new(type, message);
}

} // namespace

GroupCallEngine::GroupCallEngine(QObject *parent)
    : QObject(parent)
{
    m_consent.setInterval(5000);
    connect(&m_consent, &QTimer::timeout, this, &GroupCallEngine::sendConsent);
    if (!gst_is_initialized()) {
        gst_init(nullptr, nullptr);
    }
}

GroupCallEngine::~GroupCallEngine()
{
    stop();
}

QString GroupCallEngine::prepare()
{
    stop();
    const DtlsCertificate::Certificate certificate = DtlsCertificate::generate();
    m_certificate = certificate.pem;
    return QString::fromLatin1(certificate.digest.toHex());
}

void GroupCallEngine::start(const QString &offer, const QVariantList &candidates)
{
    if (m_certificate.isEmpty()) {
        emit failed(QStringLiteral("certificate"));
        return;
    }
    // Our video stream's RTP numbers start at random values (RFC 3550).
    for (int i = 0; i < VIDEO_LAYERS; ++i) {
        VideoLayer &layer = m_videoLayers[i];
        layer.engine = this;
        layer.index = i;
        RAND_bytes(reinterpret_cast<unsigned char *>(&layer.ssrc), sizeof layer.ssrc);
        RAND_bytes(reinterpret_cast<unsigned char *>(&layer.sequence), sizeof layer.sequence);
        RAND_bytes(reinterpret_cast<unsigned char *>(&layer.timestampOffset), sizeof layer.timestampOffset);
    }
    // A pipeline of its own (gst_parse_launch would return a lone webrtcbin itself).
    m_pipeline = gst_pipeline_new("groupcall");
    GstElement *webrtc = gst_element_factory_make("webrtcbin", "webrtc");
    if (!webrtc) {
        gst_object_unref(m_pipeline);
        m_pipeline = nullptr;
        emit failed(QStringLiteral("pipeline"));
        return;
    }
    g_object_set(webrtc, "bundle-policy", GST_WEBRTC_BUNDLE_POLICY_MAX_BUNDLE, "latency", 100, nullptr);
    gst_bin_add(GST_BIN(m_pipeline), webrtc);
    m_webrtc = GST_ELEMENT(gst_object_ref(webrtc));
    m_candidates = candidates;
    g_signal_connect(m_pipeline, "deep-element-added", G_CALLBACK(&GroupCallEngine::onElementAdded), this);
    g_signal_connect(m_webrtc, "notify::connection-state", G_CALLBACK(&GroupCallEngine::onConnectionStateCb), this);
    GstBus *bus = gst_pipeline_get_bus(GST_PIPELINE(m_pipeline));
    m_busWatch = gst_bus_add_watch(bus, reinterpret_cast<GstBusFunc>(&GroupCallEngine::onBusMessage), this);
    gst_object_unref(bus);
    gst_element_set_state(m_pipeline, GST_STATE_READY);
    GstStructure *options = gst_structure_new("options", "negotiated", G_TYPE_BOOLEAN, TRUE, "id", G_TYPE_INT, 0,
                                              "ordered", G_TYPE_BOOLEAN, TRUE, nullptr);
    g_signal_emit_by_name(m_webrtc, "create-data-channel", DATA_CHANNEL_LABEL, options, &m_channel);
    gst_structure_free(options);
    if (m_channel) {
        g_signal_connect(m_channel, "on-open", G_CALLBACK(&GroupCallEngine::onChannelOpenCb), this);
        g_signal_connect(m_channel, "on-message-data", G_CALLBACK(&GroupCallEngine::onChannelDataCb), this);
    }
    if (!buildAudio(offer) || !buildVideo(offer)) {
        stop();
        emit failed(QStringLiteral("pipeline"));
        return;
    }
    g_signal_connect(m_webrtc, "pad-added", G_CALLBACK(&GroupCallEngine::onPadAdded), this);
    if (gst_element_set_state(m_pipeline, GST_STATE_PLAYING) == GST_STATE_CHANGE_FAILURE) {
        stop();
        emit failed(QStringLiteral("pipeline"));
        return;
    }
    setRemote(offer);
}

// Our microphone into webrtcbin (its transceiver is the offer's first m-line, our microphone's
// MID, which every RTP packet carries for the SFU), and the mix of everyone else to the speaker.
bool GroupCallEngine::buildAudio(const QString &offer)
{
    const OfferInfo info = offerInfo(offer);
    if (info.microphoneMid.isEmpty() || info.midExtension < 0) {
        qWarning() << "group call: no microphone MID in the offer";
        return false;
    }
    // Opus as the SFU wants it (usedtx=1; no CBR, unlike 1:1 calls), encrypted per frame.
    const QString send = QStringLiteral(
        "pulsesrc stream-properties=\"props,media.role=phone\" ! audioconvert ! audioresample "
        "! audio/x-raw,format=S16LE,channels=1,rate=48000 ! audiobuffersplit output-buffer-duration=1/100 "
        "! identity name=aec ! volume name=mic ! queue "
        "! opusenc name=opus audio-type=voice bitrate=32000 frame-size=20 inband-fec=true dtx=true "
        "! rtpopuspay name=pay pt=111 ! queue "
        "! capsfilter name=rtpcaps "
        "caps=\"application/x-rtp,media=audio,encoding-name=OPUS,clock-rate=48000,payload=111,"
        "encoding-params=(string)2\"");
    const QString receive = QStringLiteral(
        // Live: it mixes whatever arrives in time and never waits for a quiet participant
        // (Opus DTX sends almost nothing while someone is silent).
        "audiomixer name=mix force-live=true ignore-inactive-pads=true latency=60000000 "
        "! audioconvert ! audioresample "
        "! audio/x-raw,format=S16LE,channels=1,rate=48000 ! audiobuffersplit output-buffer-duration=1/100 "
        "! identity name=aecplayback ! pulsesink stream-properties=\"props,media.role=phone\"");
    GError *error = nullptr;
    // The microphone joins the pipeline only when it can be linked (linkMicrophone), or it would
    // stop at once with "not-linked".
    GstElement *sendBin = gst_parse_bin_from_description(send.toUtf8().constData(), FALSE, &error);
    GstElement *bin = error ? nullptr : gst_parse_bin_from_description(receive.toUtf8().constData(), FALSE, &error);
    if (error) {
        qWarning() << "group call audio:" << error->message;
        g_error_free(error);
        if (sendBin) {
            gst_object_unref(sendBin);
        }
        return false;
    }
    gst_bin_add(GST_BIN(m_pipeline), bin);
    m_sendBin = GST_ELEMENT(gst_object_ref_sink(sendBin));
    // Linked to webrtcbin once it knows the m-lines (linkMicrophone): m-line 0 is ours.
    m_sendTail = GST_ELEMENT(gst_object_ref(sendBin));

    // The MID header extension on our packets (GStreamer's own element refuses Threema's MID
    // characters, so a probe writes it).
    m_mid = info.microphoneMid.toUtf8();
    m_midExtension = info.midExtension;
    GstElement *pay = gst_bin_get_by_name(GST_BIN(sendBin), "pay");
    GstPad *paySrc = gst_element_get_static_pad(pay, "src");
    gst_pad_add_probe(paySrc, GST_PAD_PROBE_TYPE_BUFFER,
                      reinterpret_cast<GstPadProbeCallback>(&GroupCallEngine::onAddMid), this, nullptr);
    gst_object_unref(paySrc);
    gst_object_unref(pay);
    // A ghost pad: webrtcbin is outside the microphone's bin.
    GstElement *tail = gst_bin_get_by_name(GST_BIN(sendBin), "rtpcaps");
    GstPad *tailSrc = gst_element_get_static_pad(tail, "src");
    gst_element_add_pad(sendBin, gst_ghost_pad_new("src", tailSrc));
    gst_object_unref(tailSrc);
    gst_object_unref(tail);

    // Frame encryption on the encoder's output; the echo canceller on both ends.
    GstElement *opus = gst_bin_get_by_name(GST_BIN(sendBin), "opus");
    GstPad *opusSrc = gst_element_get_static_pad(opus, "src");
    gst_pad_add_probe(opusSrc, GST_PAD_PROBE_TYPE_BUFFER,
                      reinterpret_cast<GstPadProbeCallback>(&GroupCallEngine::onEncrypt), this, nullptr);
    gst_object_unref(opusSrc);
    gst_object_unref(opus);
    m_echoCanceller.reset(new EchoCanceller);
    GstElement *aec = gst_bin_get_by_name(GST_BIN(sendBin), "aec");
    GstPad *aecPad = gst_element_get_static_pad(aec, "src");
    gst_pad_add_probe(aecPad, GST_PAD_PROBE_TYPE_BUFFER,
                      reinterpret_cast<GstPadProbeCallback>(&GroupCallEngine::onCaptureBuffer), this, nullptr);
    gst_object_unref(aecPad);
    gst_object_unref(aec);
    GstElement *playback = gst_bin_get_by_name(GST_BIN(bin), "aecplayback");
    GstPad *playbackPad = gst_element_get_static_pad(playback, "src");
    gst_pad_add_probe(playbackPad, GST_PAD_PROBE_TYPE_BUFFER,
                      reinterpret_cast<GstPadProbeCallback>(&GroupCallEngine::onPlaybackBuffer), this, nullptr);
    gst_object_unref(playbackPad);
    gst_object_unref(playback);
    m_micVolume = gst_bin_get_by_name(GST_BIN(sendBin), "mic");
    // Muted while there was no pipeline yet ("Joining…" takes seconds).
    g_object_set(m_micVolume, "mute", m_muted ? TRUE : FALSE, nullptr);
    m_mixer = gst_bin_get_by_name(GST_BIN(bin), "mix");
    return true;
}

// Our camera's way to the SFU: whatever the camera branch feeds into `vin` (nothing while it is
// off), VP8 in three sizes (simulcast; a key frame at most 3 s apart, so late subscribers see
// something soon), then the core encrypts and packetizes each frame (onEncodedFrame) into `vsrc`,
// which goes to webrtcbin like the microphone.
bool GroupCallEngine::buildVideo(const QString &offer)
{
    const OfferInfo info = offerInfo(offer);
    if (info.cameraMid.isEmpty()) {
        qWarning() << "group call: no camera MID in the offer";
        return false;
    }
    m_cameraMid = info.cameraMid.toUtf8();
    m_ridExtension = info.ridExtension;
    m_orientationExtension = info.orientationExtension;
    const QString send = QStringLiteral(
        "appsrc name=vsrc is-live=true format=time do-timestamp=false min-latency=0 max-latency=-1 "
        "caps=\"application/x-rtp,media=video,encoding-name=VP8,clock-rate=90000,payload=%1,ssrc=(uint)%2\"")
                             .arg(VP8_PAYLOAD_TYPE)
                             .arg(m_videoLayers[VIDEO_LAYERS - 1].ssrc);
    GError *error = nullptr;
    GstElement *sendBin = gst_parse_bin_from_description(send.toUtf8().constData(), FALSE, &error);
    if (error) {
        qWarning() << "group call video:" << error->message;
        g_error_free(error);
        return false;
    }
    // The input next to the camera branch in the pipeline (links don't cross bins), then one
    // encoder per simulcast layer.
    m_videoInput = gst_element_factory_make("funnel", "vin");
    GstElement *tee = gst_element_factory_make("tee", "vtee");
    gst_bin_add_many(GST_BIN(m_pipeline), m_videoInput, tee, nullptr);
    gst_object_ref(m_videoInput);
    gst_element_link(m_videoInput, tee);
    for (int i = 0; i < VIDEO_LAYERS; ++i) {
        const SimulcastLayer &layer = SIMULCAST[i];
        const QString encode = QStringLiteral(
            "queue max-size-buffers=2 leaky=downstream ! videoscale "
            "! video/x-raw,width=%1,height=%2 "
            "! vp8enc name=venc deadline=1 cpu-used=8 end-usage=cbr target-bitrate=%3 "
            "keyframe-max-dist=60 lag-in-frames=0 error-resilient=default threads=%4 "
            "! appsink name=vout emit-signals=true sync=false")
                                   .arg(CAMERA_WIDTH / layer.scaleDown)
                                   .arg(CAMERA_HEIGHT / layer.scaleDown)
                                   .arg(layer.bitrate)
                                   .arg(layer.scaleDown == 1 ? 2 : 1);
        GstElement *encodeBin = gst_parse_bin_from_description(encode.toUtf8().constData(), TRUE, &error);
        if (error) {
            qWarning() << "group call video:" << error->message;
            g_error_free(error);
            gst_object_unref(sendBin);
            return false;
        }
        gst_bin_add(GST_BIN(m_pipeline), encodeBin);
        gst_element_link(tee, encodeBin);
        m_videoLayers[i].encoder = gst_bin_get_by_name(GST_BIN(encodeBin), "venc");
        GstElement *out = gst_bin_get_by_name(GST_BIN(encodeBin), "vout");
        g_signal_connect(out, "new-sample", G_CALLBACK(&GroupCallEngine::onEncodedFrame), &m_videoLayers[i]);
        gst_object_unref(out);
    }
    m_videoSource = gst_bin_get_by_name(GST_BIN(sendBin), "vsrc");
    GstPad *sourcePad = gst_element_get_static_pad(m_videoSource, "src");
    // Someone asked for a key frame (a new subscriber, lost packets): the encoder makes one.
    gst_pad_add_probe(sourcePad, GST_PAD_PROBE_TYPE_EVENT_UPSTREAM,
                      reinterpret_cast<GstPadProbeCallback>(&GroupCallEngine::onKeyFrameRequest), this, nullptr);
    gst_element_add_pad(sendBin, gst_ghost_pad_new("src", sourcePad));
    gst_object_unref(sourcePad);
    m_videoSendBin = GST_ELEMENT(gst_object_ref_sink(sendBin));
    return true;
}

// After the first remote description: our microphone to m-line 0's transceiver and our camera
// to m-line 1's, sending only.
void GroupCallEngine::linkSenders()
{
    linkMicrophone();
    if (!m_videoSendBin || !m_webrtc) {
        return;
    }
    gst_bin_add(GST_BIN(m_pipeline), m_videoSendBin);
    GstWebRTCRTPTransceiver *transceiver = nullptr;
    g_signal_emit_by_name(m_webrtc, "get-transceiver", 1, &transceiver);
    if (transceiver) {
        g_object_set(transceiver, "direction", GST_WEBRTC_RTP_TRANSCEIVER_DIRECTION_SENDONLY, nullptr);
        gst_object_unref(transceiver);
    }
    GstPad *sink = gst_element_request_pad_simple(m_webrtc, "sink_1");
    GstPad *src = gst_element_get_static_pad(m_videoSendBin, "src");
    if (!sink || gst_pad_link(src, sink) != GST_PAD_LINK_OK) {
        qWarning() << "group call video: cannot link the camera";
    }
    gst_object_unref(src);
    if (sink) {
        gst_object_unref(sink);
    }
    gst_element_sync_state_with_parent(m_videoSendBin);
    gst_object_unref(m_videoSendBin);
    m_videoSendBin = nullptr;
    gst_bin_recalculate_latency(GST_BIN(m_pipeline));
}

// The camera branch: the camera (on a phone droidcamsrc, front or back; in the emulator, which
// has none, a test pattern) into the encoder and our own tile.
void GroupCallEngine::setCamera(bool on, bool front)
{
    if (!m_pipeline || !m_videoInput) {
        return;
    }
    if (m_camera && (!on || front != m_frontCamera)) {
        removeCamera();
    }
    if (!on || m_camera) {
        return;
    }
    m_frontCamera = front;
    GstElementFactory *droid = gst_element_factory_find("droidcamsrc");
    const QString source = droid ? QStringLiteral("droidcamsrc camera-device=%1 mode=2").arg(front ? 1 : 0)
                                 : QStringLiteral("videotestsrc is-live=true pattern=ball");
    if (droid) {
        gst_object_unref(droid);
    }
    const QString description = source + QStringLiteral(
        " ! videoconvert ! videoscale ! videorate ! capsfilter caps=%1 ! tee name=camtee "
        "camtee. ! queue max-size-buffers=2 leaky=downstream name=camout "
        "camtee. ! queue max-size-buffers=1 leaky=downstream ! videoconvert ! video/x-raw,format=BGRx "
        "! appsink name=localvideo emit-signals=true sync=false max-buffers=1 drop=true")
                                    .arg(QString::fromLatin1(CAMERA_CAPS));
    GError *error = nullptr;
    GstElement *bin = gst_parse_bin_from_description(description.toUtf8().constData(), FALSE, &error);
    if (error) {
        qWarning() << "group call camera:" << error->message;
        g_error_free(error);
        return;
    }
    GstElement *out = gst_bin_get_by_name(GST_BIN(bin), "camout");
    GstPad *outPad = gst_element_get_static_pad(out, "src");
    gst_element_add_pad(bin, gst_ghost_pad_new("src", outPad));
    gst_object_unref(outPad);
    gst_object_unref(out);
    GstElement *preview = gst_bin_get_by_name(GST_BIN(bin), "localvideo");
    g_signal_connect(preview, "new-sample", G_CALLBACK(&GroupCallEngine::onLocalSample), this);
    gst_object_unref(preview);

    gst_bin_add(GST_BIN(m_pipeline), bin);
    m_cameraPad = gst_element_request_pad_simple(m_videoInput, "sink_%u");
    GstPad *binPad = gst_element_get_static_pad(bin, "src");
    if (gst_pad_link(binPad, m_cameraPad) != GST_PAD_LINK_OK) {
        qWarning() << "group call camera: cannot link it to the encoder";
    }
    gst_object_unref(binPad);
    gst_element_sync_state_with_parent(bin);
    m_camera = bin;
}

void GroupCallEngine::removeCamera()
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
    gst_element_release_request_pad(m_videoInput, m_cameraPad);
    gst_object_unref(m_cameraPad);
    m_cameraPad = nullptr;
    gst_element_set_state(bin, GST_STATE_NULL);
    gst_bin_remove(GST_BIN(m_pipeline), bin);
    emit localVideoStopped();
}

// After the first remote description: our microphone to m-line 0's transceiver, sending only.
void GroupCallEngine::linkMicrophone()
{
    if (!m_sendTail || !m_webrtc || !m_sendBin) {
        return;
    }
    gst_bin_add(GST_BIN(m_pipeline), m_sendBin);
    GstWebRTCRTPTransceiver *transceiver = nullptr;
    g_signal_emit_by_name(m_webrtc, "get-transceiver", 0, &transceiver);
    if (transceiver) {
        g_object_set(transceiver, "direction", GST_WEBRTC_RTP_TRANSCEIVER_DIRECTION_SENDONLY, nullptr);
        gst_object_unref(transceiver);
    }
    GstPad *sink = gst_element_request_pad_simple(m_webrtc, "sink_0");
    GstPad *src = gst_element_get_static_pad(m_sendTail, "src");
    bool working = true;
    if (!sink || gst_pad_link(src, sink) != GST_PAD_LINK_OK) {
        qWarning() << "group call audio: cannot link the microphone";
        working = false;
    }
    gst_object_unref(src);
    if (sink) {
        gst_object_unref(sink);
    }
    gst_object_unref(m_sendTail);
    m_sendTail = nullptr;
    if (!gst_element_sync_state_with_parent(m_sendBin)) {
        qWarning() << "group call audio: the microphone doesn't start";
        working = false;
    }
    gst_object_unref(m_sendBin);
    m_sendBin = nullptr;
    // Without the microphone the call only seems to run; queued, this is in the middle of the setup.
    if (!working) {
        QMetaObject::invokeMethod(this, "onError", Qt::QueuedConnection, Q_ARG(QString, QStringLiteral("media")));
    }
    // The branch joined a playing pipeline: without the latency rtpsession knows no running
    // time, so it can't make sender reports and sends no RTCP at all.
    gst_bin_recalculate_latency(GST_BIN(m_pipeline));
}

void GroupCallEngine::setMuted(bool muted)
{
    m_muted = muted;
    if (m_micVolume) {
        g_object_set(m_micVolume, "mute", muted ? TRUE : FALSE, nullptr);
    }
}

void GroupCallEngine::setMicrophoneMids(const QHash<QString, quint32> &mids)
{
    QMutexLocker locker(&m_midsLock);
    m_mids = mids;
}

quint32 GroupCallEngine::participantOf(const QString &mid)
{
    QMutexLocker locker(&m_midsLock);
    return m_mids.value(mid, UINT32_MAX);
}

void GroupCallEngine::updateOffer(const QString &offer)
{
    if (!m_webrtc) {
        return;
    }
    if (m_negotiating) {
        m_nextOffer = offer;
        return;
    }
    setRemote(offer);
}

void GroupCallEngine::setRemote(const QString &offer)
{
    if (m_remoteFingerprint.isEmpty()) {
        m_remoteFingerprint = DtlsCertificate::sdpFingerprint(offer);
    }
    if (m_remoteUfrag.isEmpty()) {
        m_remoteUfrag = sdpValue(offer, QStringLiteral("ice-ufrag"));
        m_remotePassword = sdpValue(offer, QStringLiteral("ice-pwd"));
    }
    GstWebRTCSessionDescription *remote = description(GST_WEBRTC_SDP_TYPE_OFFER, offer);
    if (!remote) {
        onError(QStringLiteral("offer"));
        return;
    }
    m_negotiating = true;
    GstPromise *promise = gst_promise_new_with_change_func(&GroupCallEngine::onRemoteSetCb, this, nullptr);
    g_signal_emit_by_name(m_webrtc, "set-remote-description", remote, promise);
    gst_webrtc_session_description_free(remote);
}

void GroupCallEngine::sendChannel(const QByteArray &data)
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
        qWarning() << "group call data channel:" << (error ? error->message : "send failed");
        g_clear_error(&error);
    }
    g_bytes_unref(bytes);
}

void GroupCallEngine::stop()
{
    m_consent.stop();
    m_localUfrag.clear();
    m_remoteUfrag.clear();
    m_remotePassword.clear();
    m_pendingChannel.clear();
    m_receivedChannel.clear();
    // Our copy of this call's DTLS private key (PEM). GStreamer's own copies (the elements' `pem`
    // and the DTLS agents it keeps per PEM) are out of our reach and not cleared; the key is good
    // for this call only.
    m_certificate.fill('\0');
    m_certificate.clear();
    m_candidates.clear();
    m_candidatesAdded = false;
    m_channelOpen = false;
    m_connected = false;
    m_verified = false;
    m_remoteFingerprint.clear();
    m_negotiating = false;
    m_nextOffer.clear();
    if (!m_pipeline) {
        return;
    }
    removeCamera();
    qInfo() << "group call frames: encrypted" << encrypted.exchange(0) << "dropped" << encryptDropped.exchange(0)
            << "decrypted" << decrypted.exchange(0) << "dropped" << decryptDropped.exchange(0);
    if (m_busWatch) {
        g_source_remove(m_busWatch);
        m_busWatch = 0;
    }
    g_signal_handlers_disconnect_by_data(m_webrtc, this);
    g_signal_handlers_disconnect_by_data(m_pipeline, this);
    gst_element_set_state(m_pipeline, GST_STATE_NULL);
    // After the pipeline stopped: no probe runs any more.
    m_echoCanceller.reset();
    if (m_sendTail) {
        gst_object_unref(m_sendTail);
        m_sendTail = nullptr;
    }
    if (m_sendBin) {
        gst_element_set_state(m_sendBin, GST_STATE_NULL);
        gst_object_unref(m_sendBin);
        m_sendBin = nullptr;
    }
    if (m_micVolume) {
        gst_object_unref(m_micVolume);
        m_micVolume = nullptr;
    }
    if (m_mixer) {
        gst_object_unref(m_mixer);
        m_mixer = nullptr;
    }
    for (GstElement **element : {&m_videoInput, &m_videoSource}) {
        if (*element) {
            gst_object_unref(*element);
            *element = nullptr;
        }
    }
    for (VideoLayer &layer : m_videoLayers) {
        if (layer.encoder) {
            gst_object_unref(layer.encoder);
            layer.encoder = nullptr;
        }
    }
    if (m_videoSendBin) {
        gst_element_set_state(m_videoSendBin, GST_STATE_NULL);
        gst_object_unref(m_videoSendBin);
        m_videoSendBin = nullptr;
    }
    if (m_channel) {
        g_signal_handlers_disconnect_by_data(m_channel, this);
        g_object_unref(m_channel);
        m_channel = nullptr;
    }
    gst_object_unref(m_webrtc);
    gst_object_unref(m_pipeline);
    m_webrtc = nullptr;
    m_pipeline = nullptr;
    // The pipeline's threads are gone. What they queued belongs to this call: an error or a
    // description must not reach the next one.
    QCoreApplication::removePostedEvents(this, QEvent::MetaCall);
    QMutexLocker locker(&m_framesLock);
    m_pendingFrames.clear();
}

// --- GStreamer threads: hand everything over to this object's thread. ---

void GroupCallEngine::onRemoteSetCb(GstPromise *promise, void *self)
{
    const bool replied = gst_promise_wait(promise) == GST_PROMISE_RESULT_REPLIED;
    const GstStructure *reply = replied ? gst_promise_get_reply(promise) : nullptr;
    QString error;
    if (!replied) {
        error = QStringLiteral("no reply");
    } else if (reply && gst_structure_has_field(reply, "error")) {
        GError *gerror = nullptr;
        gst_structure_get(reply, "error", G_TYPE_ERROR, &gerror, nullptr);
        error = gerror ? QString::fromUtf8(gerror->message) : QStringLiteral("unknown");
        g_clear_error(&gerror);
    }
    gst_promise_unref(promise);
    if (error.isEmpty()) {
        QMetaObject::invokeMethod(static_cast<GroupCallEngine *>(self), "onRemoteSet", Qt::QueuedConnection);
    } else {
        QMetaObject::invokeMethod(static_cast<GroupCallEngine *>(self), "onError", Qt::QueuedConnection,
                                  Q_ARG(QString, QStringLiteral("remote description: ") + error));
    }
}

void GroupCallEngine::onAnswerCreated(GstPromise *promise, void *self)
{
    auto *engine = static_cast<GroupCallEngine *>(self);
    GstWebRTCSessionDescription *answer = nullptr;
    if (gst_promise_wait(promise) == GST_PROMISE_RESULT_REPLIED) {
        gst_structure_get(gst_promise_get_reply(promise), "answer", GST_TYPE_WEBRTC_SESSION_DESCRIPTION, &answer,
                          nullptr);
    }
    gst_promise_unref(promise);
    if (!answer) {
        QMetaObject::invokeMethod(engine, "onError", Qt::QueuedConnection, Q_ARG(QString, QStringLiteral("answer")));
        return;
    }
    // The SFU never sees our SDP: it only needs our certificate (its fingerprint was in the join).
    g_signal_emit_by_name(engine->m_webrtc, "set-local-description", answer, nullptr);
    gchar *text = gst_sdp_message_as_text(answer->sdp);
    const QString sdp = QString::fromUtf8(text);
    g_free(text);
    gst_webrtc_session_description_free(answer);
    QMetaObject::invokeMethod(engine, "onAnswer", Qt::QueuedConnection, Q_ARG(QString, sdp));
}

void GroupCallEngine::onConnectionStateCb(GstElement *webrtc, GParamSpec *, void *self)
{
    GstWebRTCPeerConnectionState state;
    g_object_get(webrtc, "connection-state", &state, nullptr);
    QMetaObject::invokeMethod(static_cast<GroupCallEngine *>(self), "onConnectionState", Qt::QueuedConnection,
                              Q_ARG(int, static_cast<int>(state)));
}

// webrtcbin's DTLS decoders take our certificate (see dtlscertificate.h).
void GroupCallEngine::onElementAdded(GstBin *, GstBin *, GstElement *element, void *self)
{
    auto *engine = static_cast<GroupCallEngine *>(self);
    GstElementFactory *factory = gst_element_get_factory(element);
    if (factory && g_strcmp0(GST_OBJECT_NAME(factory), "dtlssrtpdec") == 0) {
        g_object_set(element, "pem", engine->m_certificate.constData(), nullptr);
        // All that comes in over the transport passes here.
        GstPad *sink = gst_element_get_static_pad(element, "sink");
        if (sink) {
            gst_pad_add_probe(sink, static_cast<GstPadProbeType>(GST_PAD_PROBE_TYPE_BUFFER | GST_PAD_PROBE_TYPE_BUFFER_LIST),
                              reinterpret_cast<GstPadProbeCallback>(&GroupCallEngine::onNetworkBuffer), engine, nullptr);
            gst_object_unref(sink);
        }
    }
}

int GroupCallEngine::onNetworkBuffer(GstPad *, GstPadProbeInfo *, void *self)
{
    ++static_cast<GroupCallEngine *>(self)->m_packetsReceived;
    return GST_PAD_PROBE_OK;
}

void GroupCallEngine::onChannelOpenCb(GstWebRTCDataChannel *, void *self)
{
    QMetaObject::invokeMethod(static_cast<GroupCallEngine *>(self), "onChannelOpen", Qt::QueuedConnection);
}

void GroupCallEngine::onChannelDataCb(GstWebRTCDataChannel *, GBytes *data, void *self)
{
    gsize size = 0;
    const auto *bytes = static_cast<const char *>(g_bytes_get_data(data, &size));
    QMetaObject::invokeMethod(static_cast<GroupCallEngine *>(self), "onChannelData", Qt::QueuedConnection,
                              Q_ARG(QByteArray, QByteArray(bytes, static_cast<int>(size))));
}

// Someone's audio. webrtcbin 1.26 routes incoming streams only by SSRCs from the SDP (Threema's
// has none), so every stream arrives on some transceiver's pad: each pad is one SSRC, and the
// MID header extension in its packets says whose it is (read in onIncomingRtp, used to decrypt).
struct IncomingStream
{
    GroupCallEngine *engine;
    uint8_t codec;
    std::atomic<quint32> participant{UINT32_MAX};
    // Degrees clockwise to show the picture upright (the sender's video orientation extension:
    // Threema's apps send their camera's pictures unrotated).
    std::atomic<int> rotation{0};
    // VP8 only.
    Vp8Filter vp8;
};

void GroupCallEngine::onPadAdded(GstElement *, GstPad *pad, void *self)
{
    auto *engine = static_cast<GroupCallEngine *>(self);
    if (GST_PAD_DIRECTION(pad) != GST_PAD_SRC || !engine->m_mixer) {
        return;
    }
    GstCaps *caps = gst_pad_get_current_caps(pad);
    if (!caps) {
        caps = gst_pad_query_caps(pad, nullptr);
    }
    const gchar *encoding = gst_structure_get_string(gst_caps_get_structure(caps, 0), "encoding-name");
    const bool opus = encoding && g_ascii_strcasecmp(encoding, "OPUS") == 0;
    const bool vp8 = encoding && g_ascii_strcasecmp(encoding, "VP8") == 0;
    gst_caps_unref(caps);
    const char *description = opus
        ? "queue ! rtpopusdepay name=depay ! opusdec plc=true use-inband-fec=true ! audioconvert ! audioresample "
          "! capsfilter caps=audio/x-raw,format=S16LE,channels=1,rate=48000 ! queue max-size-time=200000000 leaky=downstream"
        : vp8
        ? "queue ! rtpvp8depay name=depay request-keyframe=true wait-for-keyframe=true ! vp8dec threads=2 "
          "! videoconvert ! video/x-raw,format=BGRx "
          "! appsink name=video emit-signals=true sync=false max-buffers=1 drop=true"
        // Retransmissions and the like.
        : "fakesink";
    GError *error = nullptr;
    GstElement *bin = gst_parse_bin_from_description(description, TRUE, &error);
    if (error) {
        qWarning() << "group call receive:" << error->message;
        g_error_free(error);
        return;
    }
    if (opus || vp8) {
        auto *stream = new IncomingStream{engine, opus ? CODEC_OPUS : CODEC_VP8};
        gst_pad_add_probe(pad, GST_PAD_PROBE_TYPE_BUFFER,
                          reinterpret_cast<GstPadProbeCallback>(&GroupCallEngine::onIncomingRtp), stream, nullptr);
        GstElement *depay = gst_bin_get_by_name(GST_BIN(bin), "depay");
        GstPad *depaySrc = gst_element_get_static_pad(depay, "src");
        // The stream lives as long as this probe (the branch).
        gst_pad_add_probe(depaySrc, GST_PAD_PROBE_TYPE_BUFFER,
                          reinterpret_cast<GstPadProbeCallback>(&GroupCallEngine::onDecrypt), stream,
                          [](gpointer data) { delete static_cast<IncomingStream *>(data); });
        gst_object_unref(depaySrc);
        gst_object_unref(depay);
        if (vp8) {
            GstElement *sink = gst_bin_get_by_name(GST_BIN(bin), "video");
            g_signal_connect(sink, "new-sample", G_CALLBACK(&GroupCallEngine::onRemoteSample), stream);
            gst_object_unref(sink);
        }
    }
    // Fully linked before it runs: a buffer reaching an unlinked pad returns "not-linked", which
    // travels upstream and stops webrtcbin's network source for good (no more media, data
    // channel or ICE).
    gst_bin_add(GST_BIN(engine->m_pipeline), bin);
    if (opus) {
        GstPad *src = gst_element_get_static_pad(bin, "src");
        GstPad *mixerSink = gst_element_request_pad_simple(engine->m_mixer, "sink_%u");
        // The mixer sits in the playback bin: ghost pads across the bin boundary.
        if (!gst_pad_link_maybe_ghosting(src, mixerSink)) {
            qWarning() << "group call: mixer link failed";
        }
        gst_object_unref(mixerSink);
        gst_object_unref(src);
    }
    gst_element_sync_state_with_parent(bin);
    GstPad *sink = gst_element_get_static_pad(bin, "sink");
    const GstPadLinkReturn linked = gst_pad_link(pad, sink);
    if (linked != GST_PAD_LINK_OK) {
        qWarning() << "group call: receive link failed" << linked;
    }
    gst_object_unref(sink);
    gst_element_post_message(engine->m_pipeline, gst_message_new_latency(GST_OBJECT(engine->m_pipeline)));
}

// The MID in a packet's header extension → the stream's participant.
int GroupCallEngine::onIncomingRtp(GstPad *, GstPadProbeInfo *info, void *data)
{
    auto *stream = static_cast<IncomingStream *>(data);
    // Nothing from the SFU before it is authenticated (onConnectionState).
    if (!stream->engine->m_verified) {
        return GST_PAD_PROBE_DROP;
    }
    GstRTPBuffer rtp = GST_RTP_BUFFER_INIT;
    if (stream->engine->m_midExtension > 0
        && gst_rtp_buffer_map(gst_pad_probe_info_get_buffer(info), GST_MAP_READ, &rtp)) {
        gpointer mid = nullptr;
        guint size = 0;
        if (gst_rtp_buffer_get_extension_onebyte_header(&rtp, static_cast<guint8>(stream->engine->m_midExtension),
                                                        0, &mid, &size)) {
            const quint32 participant =
                stream->engine->participantOf(QString::fromLatin1(static_cast<const char *>(mid),
                                                                  static_cast<int>(size)));
            if (participant != UINT32_MAX) {
                stream->participant = participant;
            }
        }
        gpointer orientation = nullptr;
        if (stream->engine->m_orientationExtension > 0
            && gst_rtp_buffer_get_extension_onebyte_header(
                &rtp, static_cast<guint8>(stream->engine->m_orientationExtension), 0, &orientation, &size)
            && size >= 1) {
            stream->rotation = (*static_cast<const guint8 *>(orientation) & 0x3) * 90;
        }
        gst_rtp_buffer_unmap(&rtp);
    }
    return GST_PAD_PROBE_OK;
}

int GroupCallEngine::onAddMid(GstPad *, GstPadProbeInfo *info, void *self)
{
    auto *engine = static_cast<GroupCallEngine *>(self);
    // Nothing to the SFU before it is authenticated (onConnectionState).
    if (!engine->m_verified) {
        return GST_PAD_PROBE_DROP;
    }
    GstBuffer *buffer = gst_buffer_make_writable(gst_pad_probe_info_get_buffer(info));
    GST_PAD_PROBE_INFO_DATA(info) = buffer;
    GstRTPBuffer rtp = GST_RTP_BUFFER_INIT;
    if (gst_rtp_buffer_map(buffer, GST_MAP_READWRITE, &rtp)) {
        gst_rtp_buffer_add_extension_onebyte_header(&rtp, static_cast<guint8>(engine->m_midExtension),
                                                    engine->m_mid.constData(),
                                                    static_cast<guint>(engine->m_mid.size()));
        gst_rtp_buffer_unmap(&rtp);
    }
    return GST_PAD_PROBE_OK;
}

int GroupCallEngine::onEncrypt(GstPad *, GstPadProbeInfo *info, void *)
{
    GstBuffer *out = transformFrame(gst_pad_probe_info_get_buffer(info), -1, CODEC_OPUS);
    if (!out) {
        return GST_PAD_PROBE_DROP;
    }
    gst_buffer_unref(GST_PAD_PROBE_INFO_BUFFER(info));
    GST_PAD_PROBE_INFO_DATA(info) = out;
    return GST_PAD_PROBE_OK;
}

int GroupCallEngine::onDecrypt(GstPad *, GstPadProbeInfo *info, void *data)
{
    auto *stream = static_cast<IncomingStream *>(data);
    const quint32 participant = stream->participant;
    if (participant == UINT32_MAX) {
        return GST_PAD_PROBE_DROP;
    }
    GstBuffer *out = transformFrame(gst_pad_probe_info_get_buffer(info), participant, stream->codec);
    if (!out) {
        return GST_PAD_PROBE_DROP;
    }
    if (stream->codec == CODEC_VP8 && !stream->vp8.passes(out)) {
        gst_buffer_unref(out);
        return GST_PAD_PROBE_DROP;
    }
    gst_buffer_unref(GST_PAD_PROBE_INFO_BUFFER(info));
    GST_PAD_PROBE_INFO_DATA(info) = out;
    return GST_PAD_PROBE_OK;
}

int GroupCallEngine::onCaptureBuffer(GstPad *, GstPadProbeInfo *info, void *self)
{
    EchoCanceller *aec = static_cast<GroupCallEngine *>(self)->m_echoCanceller.data();
    GstBuffer *buffer = gst_buffer_make_writable(gst_pad_probe_info_get_buffer(info));
    GST_PAD_PROBE_INFO_DATA(info) = buffer;
    GstMapInfo map;
    if (aec && gst_buffer_map(buffer, &map, GST_MAP_READWRITE)) {
        auto *samples = reinterpret_cast<int16_t *>(map.data);
        const gsize count = map.size / sizeof(int16_t);
        for (gsize i = 0; i + EchoCanceller::FRAME_SAMPLES <= count; i += EchoCanceller::FRAME_SAMPLES) {
            aec->capture(samples + i);
        }
        gst_buffer_unmap(buffer, &map);
    }
    return GST_PAD_PROBE_OK;
}

int GroupCallEngine::onPlaybackBuffer(GstPad *, GstPadProbeInfo *info, void *self)
{
    EchoCanceller *aec = static_cast<GroupCallEngine *>(self)->m_echoCanceller.data();
    GstBuffer *buffer = gst_pad_probe_info_get_buffer(info);
    GstMapInfo map;
    if (aec && gst_buffer_map(buffer, &map, GST_MAP_READ)) {
        const auto *samples = reinterpret_cast<const int16_t *>(map.data);
        const gsize count = map.size / sizeof(int16_t);
        for (gsize i = 0; i + EchoCanceller::FRAME_SAMPLES <= count; i += EchoCanceller::FRAME_SAMPLES) {
            aec->playback(samples + i);
        }
        gst_buffer_unmap(buffer, &map);
    }
    return GST_PAD_PROBE_OK;
}

// One of our VP8 frames of one simulcast layer (each on its encoder's thread): encrypted and
// packetized by the core, sent as that layer's RTP stream with our camera's MID and the layer's
// RID for the SFU.
int GroupCallEngine::onEncodedFrame(GstElement *sink, void *data)
{
    auto *layer = static_cast<VideoLayer *>(data);
    GroupCallEngine *engine = layer->engine;
    GstSample *sample = gst_app_sink_pull_sample(GST_APP_SINK(sink));
    if (!sample) {
        return GST_FLOW_OK;
    }
    GstBuffer *frame = gst_sample_get_buffer(sample);
    GstMapInfo map;
    if (!engine->m_verified || !engine->m_videoSource || !gst_buffer_map(frame, &map, GST_MAP_READ)) {
        gst_sample_unref(sample);
        return GST_FLOW_OK;
    }
    const gsize encryptedSize = map.size + 22;
    const gsize maxPackets = encryptedSize / (VP8_MAX_PAYLOAD - 6) + 2;
    QByteArray payloads(static_cast<int>(encryptedSize + maxPackets * 6), Qt::Uninitialized);
    std::vector<size_t> lengths(maxPackets);
    const ptrdiff_t count = threesea_group_call_vp8_packets(
        static_cast<uint8_t>(layer->index), map.data, map.size, VP8_MAX_PAYLOAD, reinterpret_cast<uint8_t *>(payloads.data()),
        static_cast<size_t>(payloads.size()), lengths.data(), maxPackets);
    gst_buffer_unmap(frame, &map);
    const GstClockTime pts = GST_BUFFER_PTS(frame);
    gst_sample_unref(sample);
    if (count <= 0) {
        ++encryptDropped;
        return GST_FLOW_OK;
    }
    ++encrypted;
    const char *rid = SIMULCAST[layer->index].rid;
    const guint32 timestamp = layer->timestampOffset
        + static_cast<guint32>(GST_CLOCK_TIME_IS_VALID(pts) ? gst_util_uint64_scale(pts, 90000, GST_SECOND) : 0);
    GstBufferList *list = gst_buffer_list_new_sized(static_cast<guint>(count));
    gsize offset = 0;
    for (ptrdiff_t i = 0; i < count; ++i) {
        const gsize length = lengths[static_cast<size_t>(i)];
        GstBuffer *packet = gst_rtp_buffer_new_allocate(static_cast<guint>(length), 0, 0);
        GstRTPBuffer rtp = GST_RTP_BUFFER_INIT;
        if (gst_rtp_buffer_map(packet, GST_MAP_READWRITE, &rtp)) {
            gst_rtp_buffer_set_ssrc(&rtp, layer->ssrc);
            gst_rtp_buffer_set_seq(&rtp, layer->sequence++);
            gst_rtp_buffer_set_timestamp(&rtp, timestamp);
            gst_rtp_buffer_set_payload_type(&rtp, VP8_PAYLOAD_TYPE);
            gst_rtp_buffer_set_marker(&rtp, i == count - 1);
            memcpy(gst_rtp_buffer_get_payload(&rtp), payloads.constData() + offset, length);
            gst_rtp_buffer_add_extension_onebyte_header(&rtp, static_cast<guint8>(engine->m_midExtension),
                                                        engine->m_cameraMid.constData(),
                                                        static_cast<guint>(engine->m_cameraMid.size()));
            if (engine->m_ridExtension > 0) {
                gst_rtp_buffer_add_extension_onebyte_header(&rtp, static_cast<guint8>(engine->m_ridExtension),
                                                            rid, static_cast<guint>(strlen(rid)));
            }
            gst_rtp_buffer_unmap(&rtp);
        }
        GST_BUFFER_PTS(packet) = pts;
        offset += length;
        gst_buffer_list_add(list, packet);
    }
    gst_app_src_push_buffer_list(GST_APP_SRC(engine->m_videoSource), list);
    return GST_FLOW_OK;
}

int GroupCallEngine::onKeyFrameRequest(GstPad *, GstPadProbeInfo *info, void *self)
{
    auto *engine = static_cast<GroupCallEngine *>(self);
    GstEvent *event = GST_PAD_PROBE_INFO_EVENT(info);
    // The request doesn't say which layer: all of them (key frames of the small ones are cheap).
    if (event && gst_video_event_is_force_key_unit(event)) {
        for (const VideoLayer &layer : engine->m_videoLayers) {
            if (layer.encoder) {
                GstPad *encoderSrc = gst_element_get_static_pad(layer.encoder, "src");
                gst_pad_send_event(encoderSrc,
                                   gst_video_event_new_upstream_force_key_unit(GST_CLOCK_TIME_NONE, TRUE, 0));
                gst_object_unref(encoderSrc);
            }
        }
    }
    return GST_PAD_PROBE_OK;
}

int GroupCallEngine::onLocalSample(GstElement *sink, void *self)
{
    const QImage image = sampleImage(GST_APP_SINK(sink));
    if (!image.isNull()) {
        QMetaObject::invokeMethod(static_cast<GroupCallEngine *>(self), "localFrame", Qt::QueuedConnection,
                                  Q_ARG(QImage, image));
    }
    return GST_FLOW_OK;
}

// A participant's decoded picture, turned upright.
int GroupCallEngine::onRemoteSample(GstElement *sink, void *data)
{
    auto *stream = static_cast<IncomingStream *>(data);
    GroupCallEngine *engine = stream->engine;
    const quint32 participant = stream->participant;
    bool wanted = participant != UINT32_MAX;
    if (wanted) {
        QMutexLocker locker(&engine->m_framesLock);
        // The GUI is behind: pictures must not pile up in its queue.
        wanted = !engine->m_pendingFrames.contains(participant);
        if (wanted) {
            engine->m_pendingFrames.insert(participant);
        }
    }
    if (!wanted) {
        GstSample *sample = gst_app_sink_pull_sample(GST_APP_SINK(sink));
        if (sample) {
            gst_sample_unref(sample);
        }
        return GST_FLOW_OK;
    }
    QImage image = sampleImage(GST_APP_SINK(sink));
    if (image.isNull()) {
        QMutexLocker locker(&engine->m_framesLock);
        engine->m_pendingFrames.remove(participant);
        return GST_FLOW_OK;
    }
    if (const int rotation = stream->rotation) {
        image = image.transformed(QTransform().rotate(rotation));
    }
    QMetaObject::invokeMethod(engine, "onParticipantFrame", Qt::QueuedConnection, Q_ARG(quint32, participant),
                              Q_ARG(QImage, image));
    return GST_FLOW_OK;
}

void GroupCallEngine::onParticipantFrame(quint32 participant, const QImage &image)
{
    {
        QMutexLocker locker(&m_framesLock);
        m_pendingFrames.remove(participant);
    }
    emit participantFrame(participant, image);
}

// Inside the microphone's or the speaker's bin (the volume and the mixer stay ours after the bins
// went into the pipeline).
bool GroupCallEngine::isSharedAudio(GstObject *source) const
{
    for (GstElement *member : {m_micVolume, m_mixer}) {
        GstObject *bin = member ? gst_object_get_parent(GST_OBJECT(member)) : nullptr;
        if (bin) {
            const bool inside = gst_object_has_as_ancestor(source, bin);
            gst_object_unref(bin);
            if (inside) {
                return true;
            }
        }
    }
    return false;
}

// On this object's thread (GLib main loop): latency changes and errors.
int GroupCallEngine::onBusMessage(GstBus *, GstMessage *message, void *self)
{
    auto *engine = static_cast<GroupCallEngine *>(self);
    switch (GST_MESSAGE_TYPE(message)) {
    case GST_MESSAGE_LATENCY:
        if (engine->m_pipeline) {
            gst_bin_recalculate_latency(GST_BIN(engine->m_pipeline));
        }
        break;
    case GST_MESSAGE_ERROR: {
        GError *error = nullptr;
        gst_message_parse_error(message, &error, nullptr);
        qWarning() << "group call pipeline error:" << (error ? error->message : "");
        g_clear_error(&error);
        // Without the microphone's or the speaker's branch the call only seems to run (a tile
        // showing the microphone on while nobody hears us). One participant's receive branch
        // failing costs only that participant, and the call goes on.
        if (engine->isSharedAudio(GST_MESSAGE_SRC(message))) {
            QMetaObject::invokeMethod(engine, "onError", Qt::QueuedConnection, Q_ARG(QString, QStringLiteral("media")));
        }
        break;
    }
    default:
        break;
    }
    return TRUE;
}

// --- This object's thread. ---

void GroupCallEngine::onRemoteSet()
{
    if (!m_webrtc) {
        return;
    }
    linkSenders();
    GstPromise *promise = gst_promise_new_with_change_func(&GroupCallEngine::onAnswerCreated, this, nullptr);
    g_signal_emit_by_name(m_webrtc, "create-answer", nullptr, promise);
}

void GroupCallEngine::onAnswer(const QString &sdp)
{
    if (!m_webrtc) {
        return;
    }
    if (m_localUfrag.isEmpty()) {
        m_localUfrag = sdpValue(sdp, QStringLiteral("ice-ufrag"));
    }
    // The SFU's candidates once, after the first answer (ICE lite: it only answers checks).
    if (!m_candidatesAdded) {
        m_candidatesAdded = true;
        for (const QVariant &candidate : m_candidates) {
            const QByteArray text = candidate.toMap().value(QStringLiteral("candidate")).toString().toUtf8();
            g_signal_emit_by_name(m_webrtc, "add-ice-candidate", 0u, text.constData());
        }
    }
    m_negotiating = false;
    if (!m_nextOffer.isEmpty()) {
        const QString next = m_nextOffer;
        m_nextOffer.clear();
        setRemote(next);
    }
}

void GroupCallEngine::onConnectionState(int state)
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
        flushChannel();
        m_consent.start();
        emit connected();
    } else if (state == GST_WEBRTC_PEER_CONNECTION_STATE_FAILED) {
        onError(QStringLiteral("connection"));
    }
}

// libnice 0.1.21 (SFOS) is older than what GStreamer needs to enable RFC 7675 consent
// freshness, so it only sends keepalive indications every 25 s; the SFU (ICE lite) drops a
// participant after about 15 s without binding requests. So we send them, every 5 s like
// libwebrtc, over the selected pair of webrtcbin's one (bundled) ICE stream.
void GroupCallEngine::sendConsent()
{
    if (!m_webrtc || m_localUfrag.isEmpty() || m_remoteUfrag.isEmpty()) {
        return;
    }
    GstWebRTCICE *ice = nullptr;
    g_object_get(m_webrtc, "ice-agent", &ice, nullptr);
    NiceAgent *agent = nullptr;
    if (ice) {
        g_object_get(ice, "agent", &agent, nullptr);
        gst_object_unref(ice);
    }
    if (!agent) {
        return;
    }
    const QByteArray request = consentRequest(m_remoteUfrag, m_localUfrag, m_remotePassword);
    // webrtcbin's first (and with max-bundle only) stream.
    const gint sent = nice_agent_send(agent, 1, NICE_COMPONENT_TYPE_RTP, static_cast<guint>(request.size()),
                                      request.constData());
    if (sent < 0) {
        qWarning() << "group call: consent check not sent";
    }
    g_object_unref(agent);
}

void GroupCallEngine::onChannelOpen()
{
    if (!m_channel) {
        return;
    }
    m_channelOpen = true;
    flushChannel();
}

void GroupCallEngine::onChannelData(const QByteArray &data)
{
    if (!m_verified) {
        m_receivedChannel << data;
        return;
    }
    emit channelMessage(data);
}

// The channel counts as open for the call once it is open and the SFU authenticated: what was
// sent or received before goes through then.
void GroupCallEngine::flushChannel()
{
    if (!m_channelOpen || !m_verified) {
        return;
    }
    const QList<QByteArray> received = m_receivedChannel;
    m_receivedChannel.clear();
    for (const QByteArray &data : received) {
        emit channelMessage(data);
    }
    const QList<QByteArray> pending = m_pendingChannel;
    m_pendingChannel.clear();
    for (const QByteArray &data : pending) {
        sendChannel(data);
    }
    emit channelOpen();
}

// The SFU's DTLS certificate is the one its join response named (pinned HTTPS to Threema).
// webrtcbin does not check this; without it anyone on the path could stand in for the SFU.
bool GroupCallEngine::remoteCertificateMatches()
{
    if (!m_remoteFingerprint.startsWith(QLatin1String("sha-256 "))) {
        qWarning() << "group call: SFU fingerprint missing or not sha-256";
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
        if (actual.compare(m_remoteFingerprint, Qt::CaseInsensitive) != 0) {
            matches = false;
        }
    }
    if (transceivers) {
        g_array_unref(transceivers);
    }
    if (!checked || !matches) {
        qWarning() << "group call: the SFU's DTLS certificate does not match its fingerprint";
    }
    return checked && matches;
}

void GroupCallEngine::onError(const QString &reason)
{
    if (!m_pipeline) {
        return;
    }
    qWarning() << "group call media failed:" << reason; // log-ok: the engine's own codes
    stop();
    emit failed(reason);
}
