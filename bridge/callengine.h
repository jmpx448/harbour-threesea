// SPDX-License-Identifier: AGPL-3.0-only
#ifndef CALLENGINE_H
#define CALLENGINE_H

#include <atomic>

#include <QObject>
#include <QScopedPointer>
#include <QStringList>
#include <QVariantList>
#include <QVariantMap>

class QImage;

typedef struct _GstElement GstElement;
typedef struct _GstPad GstPad;
typedef struct _GstPromise GstPromise;
typedef struct _GstBus GstBus;
typedef struct _GstBin GstBin;
typedef struct _GstMessage GstMessage;
typedef struct _GParamSpec GParamSpec;
typedef struct _GstWebRTCSessionDescription GstWebRTCSessionDescription;
typedef struct _GstWebRTCDataChannel GstWebRTCDataChannel;
typedef struct _GstPadProbeInfo GstPadProbeInfo;
class EchoCanceller;

// The media side of a 1:1 call (ADR 0012, Full only): a GStreamer webrtcbin pipeline set up like
// Android's PeerConnectionClient (max-bundle, rtcp-mux, Opus mono, the negotiated data channel
// "3MACALLdc0" with id 0, TURN from the core). It only makes and takes SDP and ICE candidates;
// the core patches the SDP and sends the signalling messages. GStreamer calls back on its own
// threads; everything is queued onto this object's thread before it leaves the class.
class CallEngine : public QObject
{
    Q_OBJECT
public:
    explicit CallEngine(QObject *parent = nullptr);
    ~CallEngine() override;

    // `turn`: the core's `callTurnServers` result. `relayOnly`: TURN candidates only.
    // Outgoing: emits localDescription("offer", sdp).
    void startOffer(const QVariantMap &turn, bool relayOnly, bool video);
    // Incoming: emits localDescription("answer", sdp) for the (patched) remote offer.
    void startAnswer(const QString &remoteOffer, const QVariantMap &turn, bool relayOnly, bool video);
    // Video calls: the camera on/off (front or back), and what the sender may use.
    void setCamera(bool on, bool front);
    void setVideoParams(int bitrateKbps, int width, int height, int fps);
    void setRemoteAnswer(const QString &sdp);
    // 10 ms audio frames played in the last call (tests).
    unsigned audioFramesReceived() const { return m_framesReceived; }
    // `[{candidate, sdpMid, sdpMLineIndex}]`; kept until the remote description is set.
    void addRemoteCandidates(const QVariantList &candidates);
    // Also before the pipeline exists: it starts as asked, and a new call keeps this until told
    // otherwise.
    void setMuted(bool muted);
    // A message for the peer's data channel (o2o-call.proto envelope from the core); kept until
    // the channel is open.
    void sendChannel(const QByteArray &data);
    void stop();
    bool active() const { return m_pipeline != nullptr; }

signals:
    void localDescription(const QString &type, const QString &sdp);
    void localCandidate(const QVariantMap &candidate);
    // ICE and DTLS are up: media flows.
    void connected();
    void failed(const QString &reason);
    // A message from the peer's data channel (for the core).
    void channelMessage(const QByteArray &data);
    // Video frames: the peer's and our camera's (preview).
    void remoteFrame(const QImage &image);
    void localFrame(const QImage &image);

private slots:
    void onLocalDescription(const QString &type, const QString &sdp);
    void onIceCandidate(uint mline, const QString &candidate);
    void onIceState(int state);
    void onConnectionState(int state);
    void onRemoteDescriptionSet();
    void onError(const QString &reason);
    void onChannelOpen();
    void onChannelData(const QByteArray &data);

private:
    // `vp8PayloadType` < 0: no video sending.
    bool build(const QVariantMap &turn, bool relayOnly, int opusPayloadType, int vp8PayloadType);
    void removeCamera();
    QString cameraCaps() const;
    // GstFlowReturn.
    static int onRemoteSample(GstElement *sink, void *self);
    static int onLocalSample(GstElement *sink, void *self);
    void addCandidate(const QVariantMap &candidate);
    bool remoteCertificateMatches();
    static void onNegotiationNeeded(GstElement *webrtc, void *self);
    static void onIceCandidateCb(GstElement *webrtc, unsigned mline, char *candidate, void *self);
    static void onPadAdded(GstElement *webrtc, GstPad *pad, void *self);
    static void onChannelOpenCb(GstWebRTCDataChannel *channel, void *self);
    static void onChannelDataCb(GstWebRTCDataChannel *channel, struct _GBytes *data, void *self);
    static void onIceStateCb(GstElement *webrtc, GParamSpec *spec, void *self);
    static void onConnectionStateCb(GstElement *webrtc, GParamSpec *spec, void *self);
    static void onOfferCreated(GstPromise *promise, void *self);
    static void onAnswerCreated(GstPromise *promise, void *self);
    static void onRemoteSet(GstPromise *promise, void *self);
    static int onBusMessage(GstBus *bus, GstMessage *message, void *self);
    static void onElementAdded(GstBin *pipeline, GstBin *bin, GstElement *element, void *self);
    // Returns GstPadProbeReturn.
    static int onCaptureBuffer(GstPad *pad, GstPadProbeInfo *info, void *self);
    static int onPlaybackBuffer(GstPad *pad, GstPadProbeInfo *info, void *self);
    static int onMediaBuffer(GstPad *pad, GstPadProbeInfo *info, void *self);
    void flushChannel();
    void descriptionCreated(GstPromise *promise, const char *field, const QString &type);

    GstElement *m_pipeline = nullptr;
    GstElement *m_webrtc = nullptr;
    GstElement *m_micVolume = nullptr;
    bool m_muted = false;
    GstWebRTCDataChannel *m_channel = nullptr;
    QScopedPointer<EchoCanceller> m_echoCanceller;
    // This call's DTLS certificate and key (PEM, ECDSA P-256). This copy is wiped in stop(); the
    // copies GStreamer makes when it gets them are not cleared (see there).
    QByteArray m_certificate;
    QString m_fingerprint;
    // The peer's, from its SDP: checked against its certificate once DTLS is up.
    QString m_remoteFingerprint;
    // 10 ms frames through the microphone and to the speaker (logged at the end of a call).
    std::atomic<unsigned> m_framesSent{0};
    std::atomic<unsigned> m_framesReceived{0};
    unsigned m_busWatch = 0;
    bool m_remoteSet = false;
    bool m_connected = false;
    // The peer's certificate matched its fingerprint: until then no media or channel data
    // passes in either direction (read from GStreamer threads).
    std::atomic<bool> m_verified{false};
    QVariantList m_pendingCandidates;
    bool m_channelOpen = false;
    bool m_video = false;
    GstElement *m_camera = nullptr;
    GstPad *m_cameraPad = nullptr;
    int m_videoWidth = 640;
    int m_videoHeight = 480;
    int m_videoFps = 25;
    QList<QByteArray> m_pendingChannel;
    // Received on the channel before the peer was authenticated: delivered after.
    QList<QByteArray> m_receivedChannel;
    // The `mid` of each m-line of the local description (candidates carry both).
    QStringList m_mids;
};

#endif // CALLENGINE_H
