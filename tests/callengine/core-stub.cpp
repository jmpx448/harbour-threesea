// SPDX-License-Identifier: AGPL-3.0-only
// The loopback test runs without the core; the filter has its tests there (call_video.rs).
#include <threesea_core.h>

int32_t threesea_call_vp8_filter(const uint8_t *, size_t, bool *waiting)
{
    *waiting = false;
    return 1;
}
