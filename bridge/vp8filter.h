// SPDX-License-Identifier: AGPL-3.0-only
#ifndef VP8FILTER_H
#define VP8FILTER_H

#include <gst/gst.h>

#include <QDebug>

#include <threesea_core.h>

// A received VP8 stream's frames on their way to the decoder: the core lets only pictures of a
// sensible size through (a small key frame can declare 16383x16383). One per stream, used on its
// streaming thread only.
struct Vp8Filter
{
    bool waiting = true;
    bool refused = false;

    bool passes(GstBuffer *frame)
    {
        GstMapInfo map;
        if (!gst_buffer_map(frame, &map, GST_MAP_READ)) {
            return false;
        }
        const int verdict = threesea_call_vp8_filter(map.data, map.size, &waiting);
        gst_buffer_unmap(frame, &map);
        if (verdict < 0 && !refused) {
            qWarning() << "call video: key frame refused";
        }
        refused = verdict < 0 || (refused && verdict == 0);
        return verdict > 0;
    }
};

#endif // VP8FILTER_H
