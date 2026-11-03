// SPDX-License-Identifier: AGPL-3.0-only
// GStreamer first: GLib headers use `signals` as a member name, which Qt defines as a macro.
#include <gst/gst.h>

#include "videotranscoder.h"

#include <QDebug>
#include <QFile>

namespace {

// Android's VideoConfig: the longest edge and the bitrate per setting.
const int SmallEdge = 480;
const int MediumEdge = 848;
const int SmallBitrate = 384000;
const int MediumBitrate = 1500000;

// The H.264 encoders worth trying, hardware first. The bitrate property means bit/s everywhere
// except x264enc, which counts kbit/s.
struct Encoder
{
    const char *element;
    bool bitrateInKbit;
};
const Encoder Encoders[] = {
    {"droidvenc", false}, {"x264enc", true}, {"openh264enc", false}, {"avenc_h264", false}};

const Encoder *findEncoder()
{
    for (const Encoder &encoder : Encoders) {
        GstElementFactory *factory = gst_element_factory_find(encoder.element);
        if (factory) {
            gst_object_unref(factory);
            return &encoder;
        }
    }
    return nullptr;
}

// Everything in the pipeline but the encoder; without one of these there is nothing to do.
bool haveHelpers()
{
    for (const char *name : {"qtdemux", "decodebin", "videoconvert", "videoscale", "h264parse", "mp4mux"}) {
        GstElementFactory *factory = gst_element_factory_find(name);
        if (!factory) {
            // log-ok: `name` is one of our own element names, not user content
            qWarning() << "video scaling: no" << name;
            return false;
        }
        gst_object_unref(factory);
    }
    return true;
}

// The demuxer's pads arrive while it reads the header: video goes into the scaling branch,
// AAC audio straight into the muxer.
struct Branches
{
    GstElement *pipeline = nullptr;
    GstElement *videoBranch = nullptr; // decodebin of the video branch
    GstElement *audioBranch = nullptr; // queue in front of aacparse
    GstElement *audioParse = nullptr;
    GstElement *mux = nullptr;
    bool videoLinked = false;
    bool audioLinked = false;
    bool audioUnsupported = false;
};

void onDemuxPad(GstElement *, GstPad *pad, void *data)
{
    Branches *branches = static_cast<Branches *>(data);
    GstCaps *caps = gst_pad_get_current_caps(pad);
    if (!caps) {
        caps = gst_pad_query_caps(pad, nullptr);
    }
    const GstStructure *structure = caps ? gst_caps_get_structure(caps, 0) : nullptr;
    const gchar *name = structure ? gst_structure_get_name(structure) : "";
    if (g_str_has_prefix(name, "video/") && !branches->videoLinked) {
        GstPad *sink = gst_element_get_static_pad(branches->videoBranch, "sink");
        branches->videoLinked = sink && gst_pad_link(pad, sink) == GST_PAD_LINK_OK;
        if (sink) {
            gst_object_unref(sink);
        }
    } else if (g_str_has_prefix(name, "audio/") && !branches->audioLinked) {
        int version = 0;
        const bool aac = g_strcmp0(name, "audio/mpeg") == 0 && structure
            && gst_structure_get_int(structure, "mpegversion", &version) && version == 4;
        if (!aac) {
            // Re-encoding audio needs an encoder we may not have, and a video without its
            // sound is not what was asked for: give up now instead of transcoding in vain.
            branches->audioUnsupported = true;
            gst_element_post_message(branches->pipeline,
                                     gst_message_new_application(GST_OBJECT(branches->pipeline),
                                                                 gst_structure_new_empty("threesea-give-up")));
        } else {
            // The muxer's audio pad is only asked for now: a video without sound would
            // otherwise leave the muxer waiting for a track that never comes.
            GstPad *sink = gst_element_get_static_pad(branches->audioBranch, "sink");
            branches->audioLinked = sink && gst_pad_link(pad, sink) == GST_PAD_LINK_OK
                && gst_element_link(branches->audioParse, branches->mux) == TRUE;
            if (sink) {
                gst_object_unref(sink);
            }
        }
    }
    if (caps) {
        gst_caps_unref(caps);
    }
}

} // namespace

int VideoTranscoder::edgeForSetting(int setting)
{
    switch (setting) {
    case 1:
        return SmallEdge;
    case 2:
        return MediumEdge;
    default:
        return 0;
    }
}

int VideoTranscoder::bitrateForSetting(int setting)
{
    return setting == 1 ? SmallBitrate : MediumBitrate;
}

bool VideoTranscoder::scaleDown(const QString &source, int setting, const QString &target)
{
    const int edge = edgeForSetting(setting);
    if (edge == 0) {
        return false;
    }
    if (!gst_is_initialized()) {
        gst_init(nullptr, nullptr);
    }
    const Encoder *encoder = findEncoder();
    if (!encoder || !haveHelpers()) {
        qWarning() << "video scaling: no H.264 encoder on this system, sending as it is";
        return false;
    }

    // filesrc → qtdemux → (video) decodebin → convert/scale → encoder → mp4mux → filesink,
    // with the AAC audio track remuxed next to it.
    GstElement *pipeline = gst_pipeline_new("scale");
    GstElement *source_element = gst_element_factory_make("filesrc", nullptr);
    GstElement *demux = gst_element_factory_make("qtdemux", nullptr);
    GstElement *decode = gst_element_factory_make("decodebin", nullptr);
    GstElement *convert = gst_element_factory_make("videoconvert", nullptr);
    GstElement *scale = gst_element_factory_make("videoscale", nullptr);
    GstElement *limit = gst_element_factory_make("capsfilter", nullptr);
    GstElement *encode = gst_element_factory_make(encoder->element, nullptr);
    GstElement *parse = gst_element_factory_make("h264parse", nullptr);
    GstElement *mux = gst_element_factory_make("mp4mux", nullptr);
    GstElement *sink = gst_element_factory_make("filesink", nullptr);
    GstElement *audioParse = gst_element_factory_make("aacparse", nullptr);
    GstElement *videoQueue = gst_element_factory_make("queue", nullptr);
    GstElement *audioQueue = gst_element_factory_make("queue", nullptr);
    if (!pipeline || !source_element || !demux || !decode || !convert || !scale || !limit || !encode || !parse
        || !mux || !sink || !audioParse || !videoQueue || !audioQueue) {
        qWarning() << "video scaling: pipeline elements missing";
        if (pipeline) {
            gst_object_unref(pipeline);
        }
        return false;
    }
    g_object_set(source_element, "location", source.toUtf8().constData(), nullptr);
    g_object_set(sink, "location", target.toUtf8().constData(), nullptr);
    // videoscale keeps the aspect ratio: only the longest edge is given, as a range for the
    // other one. Android scales the longest edge, portrait or landscape.
    GstCaps *caps = gst_caps_from_string(
        QStringLiteral("video/x-raw,width=(int)[1,%1],height=(int)[1,%1],pixel-aspect-ratio=(fraction)1/1")
            .arg(edge)
            .toUtf8()
            .constData());
    g_object_set(limit, "caps", caps, nullptr);
    gst_caps_unref(caps);
    const int bitrate = bitrateForSetting(setting);
    g_object_set(encode, "bitrate", encoder->bitrateInKbit ? bitrate / 1000 : bitrate, nullptr);
    if (encoder->bitrateInKbit) { // x264enc: fast enough for a phone
        gst_util_set_object_arg(G_OBJECT(encode), "speed-preset", "veryfast");
    }
    g_object_set(scale, "add-borders", FALSE, nullptr);

    gst_bin_add_many(GST_BIN(pipeline), source_element, demux, decode, videoQueue, convert, scale, limit, encode,
                     parse, mux, sink, audioQueue, audioParse, nullptr);
    Branches branches;
    branches.pipeline = pipeline;
    branches.videoBranch = decode;
    branches.audioBranch = audioQueue;
    branches.audioParse = audioParse;
    branches.mux = mux;
    bool linked = gst_element_link(source_element, demux) == TRUE;
    linked = linked && gst_element_link_many(videoQueue, convert, scale, limit, encode, parse, nullptr) == TRUE;
    linked = linked && gst_element_link(parse, mux) == TRUE;
    linked = linked && gst_element_link(audioQueue, audioParse) == TRUE;
    linked = linked && gst_element_link(mux, sink) == TRUE;
    if (!linked) {
        qWarning() << "video scaling: cannot link the pipeline";
        gst_object_unref(pipeline);
        return false;
    }
    // The decoder's raw pad appears while it reads the stream.
    g_signal_connect(demux, "pad-added", G_CALLBACK(&onDemuxPad), &branches);
    g_signal_connect(decode, "pad-added", G_CALLBACK(+[](GstElement *, GstPad *pad, void *data) {
                         GstElement *queue = static_cast<GstElement *>(data);
                         GstPad *sink = gst_element_get_static_pad(queue, "sink");
                         if (sink) {
                             if (!gst_pad_is_linked(sink)) {
                                 gst_pad_link(pad, sink);
                             }
                             gst_object_unref(sink);
                         }
                     }),
                     videoQueue);

    bool ok = false;
    if (gst_element_set_state(pipeline, GST_STATE_PLAYING) != GST_STATE_CHANGE_FAILURE) {
        GstBus *bus = gst_element_get_bus(pipeline);
        // Long enough for a few minutes of video on a phone; anything slower is sent as it is.
        GstMessage *message = gst_bus_timed_pop_filtered(
            bus, 5 * 60 * GST_SECOND,
            static_cast<GstMessageType>(GST_MESSAGE_EOS | GST_MESSAGE_ERROR | GST_MESSAGE_APPLICATION));
        if (message) {
            if (GST_MESSAGE_TYPE(message) == GST_MESSAGE_EOS) {
                ok = true;
            } else if (GST_MESSAGE_TYPE(message) == GST_MESSAGE_APPLICATION) {
                qInfo() << "video scaling: the sound is not AAC, sending the video as it is";
            } else {
                GError *error = nullptr;
                gst_message_parse_error(message, &error, nullptr);
                // log-ok: GStreamer's own text, elements and codecs only
                qWarning() << "video scaling failed:" << (error ? error->message : "");
                if (error) {
                    g_error_free(error);
                }
            }
            gst_message_unref(message);
        } else {
            qWarning() << "video scaling: timed out";
        }
        gst_object_unref(bus);
    }
    if (branches.audioUnsupported) {
        ok = false; // the sound is not AAC: nothing of it would reach the file
    }
    gst_element_set_state(pipeline, GST_STATE_NULL);
    gst_object_unref(pipeline);
    if (!ok) {
        QFile::remove(target);
        return false;
    }
    qInfo() << "video scaled down with" << encoder->element << "to" << edge << "px";
    return true;
}
