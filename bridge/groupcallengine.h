// SPDX-License-Identifier: AGPL-3.0-only
#ifndef GROUPCALLENGINE_H
#define GROUPCALLENGINE_H

#include <QByteArray>
#include <QHash>
#include <QImage>
#include <QList>
#include <QMutex>
#include <QSet>
#include <QObject>
#include <QScopedPointer>
#include <QTimer>
#include <QString>
#include <QVariantList>

#include <atomic>

typedef struct _GstElement GstElement;
typedef struct _GstObject GstObject;
typedef struct _GstPromise GstPromise;
typedef struct _GstBin GstBin;
typedef struct _GstWebRTCDataChannel GstWebRTCDataChannel;
typedef struct _GParamSpec GParamSpec;
typedef struct _GstPad GstPad;
typedef struct _GstBus GstBus;
typedef struct _GstMessage GstMessage;
typedef struct _GstPadProbeInfo GstPadProbeInfo;
class EchoCanceller;

// The media side of a group call (ADR 0012, Full only): webrtcbin against Threema's SFU. The
// core builds the "remote offer" (the SFU sends none) and this engine answers it, with our own
// ECDSA certificate whose SHA-256 went into the join request, and carries the SFU's data channel
// ("p2s", negotiated, id 0). Media: Opus and VP8, each frame encrypted by the core; our VP8 is
// packetized by the core too (GStreamer's payloader can't parse encrypted frames).
class GroupCallEngine : public QObject
{
    Q_OBJECT
public:
    // Our camera's simulcast layers (l, m, h).
    static const int VIDEO_LAYERS = 3;

    explicit GroupCallEngine(QObject *parent = nullptr);
    ~GroupCallEngine() override;

    // A new certificate for the next call; returns its SHA-256 (lowercase hex) for the join.
    QString prepare();
    // The core's remote offer and the SFU's candidates (`[{candidate, sdpMLineIndex}]`).
    void start(const QString &offer, const QVariantList &candidates);
    // Participants came or went: the core's new remote offer.
    void updateOffer(const QString &offer);
    void sendChannel(const QByteArray &data);
    // Also before the pipeline exists: it starts as asked, and a new call keeps this until told
    // otherwise.
    void setMuted(bool muted);
    // Which participant sends on which MID (microphone and camera, from the core's participant
    // list).
    void setMicrophoneMids(const QHash<QString, quint32> &mids);
    // Our camera on or off (front or back on a phone).
    void setCamera(bool on, bool front);
    void stop();
    bool active() const { return m_pipeline != nullptr; }

signals:
    // The SFU's data channel is open (the SFU sends Hello on it).
    void channelOpen();
    void channelMessage(const QByteArray &data);
    void connected();
    void failed(const QString &reason);
    // Our camera's picture, a participant's picture (upright), our camera went off.
    void localFrame(const QImage &image);
    void participantFrame(quint32 participant, const QImage &image);
    void localVideoStopped();

private slots:
    void onRemoteSet();
    void onAnswer(const QString &sdp);
    void onConnectionState(int state);
    void onChannelOpen();
    void onChannelData(const QByteArray &data);
    void onError(const QString &reason);
    void sendConsent();
    void onParticipantFrame(quint32 participant, const QImage &image);

private:
    void setRemote(const QString &offer);
    bool remoteCertificateMatches();
    void flushChannel();
    static void onRemoteSetCb(GstPromise *promise, void *self);
    static void onAnswerCreated(GstPromise *promise, void *self);
    static void onConnectionStateCb(GstElement *webrtc, GParamSpec *spec, void *self);
    static void onElementAdded(GstBin *pipeline, GstBin *bin, GstElement *element, void *self);
    static void onChannelOpenCb(GstWebRTCDataChannel *channel, void *self);
    static void onChannelDataCb(GstWebRTCDataChannel *channel, struct _GBytes *data, void *self);
    static void onPadAdded(GstElement *webrtc, GstPad *pad, void *self);
    // GstPadProbeReturn: frame encryption after the encoder, decryption before the decoder, and
    // the echo canceller's two taps.
    static int onEncrypt(GstPad *pad, GstPadProbeInfo *info, void *self);
    static int onAddMid(GstPad *pad, GstPadProbeInfo *info, void *self);
    static int onBusMessage(GstBus *bus, GstMessage *message, void *self);
    static int onDecrypt(GstPad *pad, GstPadProbeInfo *info, void *stream);
    static int onIncomingRtp(GstPad *pad, GstPadProbeInfo *info, void *stream);
    static int onCaptureBuffer(GstPad *pad, GstPadProbeInfo *info, void *self);
    static int onPlaybackBuffer(GstPad *pad, GstPadProbeInfo *info, void *self);
    // GstFlowReturn: appsink callbacks (our encoded frames, our preview, a participant's picture).
    static int onEncodedFrame(GstElement *sink, void *layer);
    static int onLocalSample(GstElement *sink, void *self);
    static int onRemoteSample(GstElement *sink, void *stream);
    static int onKeyFrameRequest(GstPad *pad, GstPadProbeInfo *info, void *self);
    bool buildAudio(const QString &offer);
    bool buildVideo(const QString &offer);
    void linkSenders();
    void linkMicrophone();
    void removeCamera();
    quint32 participantOf(const QString &mid);
    bool isSharedAudio(GstObject *source) const;

    GstElement *m_pipeline = nullptr;
    GstElement *m_webrtc = nullptr;
    GstWebRTCDataChannel *m_channel = nullptr;
    GstElement *m_micVolume = nullptr;
    bool m_muted = false;
    unsigned m_busWatch = 0;
    // The microphone's RTP end until linkMicrophone() hands it to webrtcbin.
    GstElement *m_sendTail = nullptr;
    GstElement *m_sendBin = nullptr;
    // Our microphone's MID and its header extension ID (written on every packet).
    QByteArray m_mid;
    int m_midExtension = -1;
    GstElement *m_mixer = nullptr;
    // Video: the encoders' input, the RTP source to webrtcbin (and its bin until
    // linkSenders), the camera branch and its pad on the input.
    GstElement *m_videoInput = nullptr;
    GstElement *m_videoSource = nullptr;
    GstElement *m_videoSendBin = nullptr;
    GstElement *m_camera = nullptr;
    GstPad *m_cameraPad = nullptr;
    bool m_frontCamera = true;
    // Our camera's MID, the RID and video orientation extensions' IDs.
    QByteArray m_cameraMid;
    int m_ridExtension = -1;
    int m_orientationExtension = -1;
    // A simulcast layer: its encoder and RTP stream (touched only on that encoder's thread).
    struct VideoLayer
    {
        GroupCallEngine *engine = nullptr;
        int index = 0;
        GstElement *encoder = nullptr;
        quint32 ssrc = 0;
        quint16 sequence = 0;
        quint32 timestampOffset = 0;
    };
    VideoLayer m_videoLayers[VIDEO_LAYERS];
    QScopedPointer<EchoCanceller> m_echoCanceller;
    // MID → participant, read from GStreamer threads.
    QMutex m_midsLock;
    QHash<QString, quint32> m_mids;
    // This call's certificate and key (PEM). This copy is wiped in stop(); the copies GStreamer
    // makes when it gets them are not cleared (see there).
    QByteArray m_certificate;
    QVariantList m_candidates;
    bool m_candidatesAdded = false;
    bool m_channelOpen = false;
    bool m_connected = false;
    QList<QByteArray> m_pendingChannel;
    // The SFU's certificate fingerprint (from the join, in its offer): checked once DTLS is up;
    // until it matched no media or channel data passes either way (read from GStreamer threads).
    QString m_remoteFingerprint;
    std::atomic<bool> m_verified{false};
    QList<QByteArray> m_receivedChannel;
    // Participants with a picture on its way to the GUI: their next ones are dropped until it got
    // there (read from GStreamer threads).
    QMutex m_framesLock;
    QSet<quint32> m_pendingFrames;
    // ICE consent checks (see sendConsent) with the credentials of both sides.
    QTimer m_consent;
    QString m_localUfrag;
    QString m_remoteUfrag;
    QString m_remotePassword;
    // An offer that arrived while the previous one was still being answered.
    QString m_nextOffer;
    bool m_negotiating = false;
};

#endif // GROUPCALLENGINE_H
