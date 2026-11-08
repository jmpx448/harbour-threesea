// SPDX-License-Identifier: AGPL-3.0-only
// The smallest Wayland compositor that lets a Qt 5.6 client draw with GL in the SDK: wl_shm only
// (Mesa's llvmpipe draws into shared memory when there is no GPU), one output, wl_shell. It shows
// nothing; it keeps the last frame a surface committed and writes it as a binary PPM to argv[1]
// after argv[2] seconds, then exits. For scripts/test-mapview.sh only; nothing of it ships.
#define _GNU_SOURCE
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <wayland-server.h>

static struct wl_display *display;
static const char *out_path;
static int width = 540, height = 960;
static uint32_t *frame;
static int frame_w, frame_h, frames;

struct surface {
    struct wl_resource *resource;
    struct wl_resource *pending;
    struct wl_list callbacks; // wl_resource links
};

static void destroy_resource(struct wl_client *c, struct wl_resource *r) { (void)c; wl_resource_destroy(r); }

// wl_region: accepted and ignored.
static void region_add(struct wl_client *c, struct wl_resource *r, int32_t x, int32_t y, int32_t w, int32_t h)
{ (void)c; (void)r; (void)x; (void)y; (void)w; (void)h; }
static const struct wl_region_interface region_impl = { destroy_resource, region_add, region_add };

static void surface_attach(struct wl_client *c, struct wl_resource *r, struct wl_resource *buffer, int32_t x, int32_t y)
{
    (void)c; (void)x; (void)y;
    struct surface *s = wl_resource_get_user_data(r);
    s->pending = buffer;
}
static void surface_damage(struct wl_client *c, struct wl_resource *r, int32_t x, int32_t y, int32_t w, int32_t h)
{ (void)c; (void)r; (void)x; (void)y; (void)w; (void)h; }
static void callback_unlink(struct wl_resource *cb) { wl_list_remove(wl_resource_get_link(cb)); }
static void surface_frame(struct wl_client *c, struct wl_resource *r, uint32_t id)
{
    struct surface *s = wl_resource_get_user_data(r);
    struct wl_resource *cb = wl_resource_create(c, &wl_callback_interface, 1, id);
    // Destroying a callback (here or when the client goes) takes it off the surface's list.
    wl_resource_set_implementation(cb, NULL, NULL, callback_unlink);
    wl_list_insert(s->callbacks.prev, wl_resource_get_link(cb));
}
static void surface_region(struct wl_client *c, struct wl_resource *r, struct wl_resource *region)
{ (void)c; (void)r; (void)region; }
static void surface_commit(struct wl_client *c, struct wl_resource *r)
{
    (void)c;
    struct surface *s = wl_resource_get_user_data(r);
    if (s->pending) {
        struct wl_shm_buffer *shm = wl_shm_buffer_get(s->pending);
        if (shm) {
            int w = wl_shm_buffer_get_width(shm), h = wl_shm_buffer_get_height(shm);
            int stride = wl_shm_buffer_get_stride(shm);
            // The biggest surface is the window (Qt also commits small ones, e.g. a cursor).
            if (w * h >= frame_w * frame_h) {
                frame = realloc(frame, (size_t)w * h * 4);
                wl_shm_buffer_begin_access(shm);
                const uint8_t *data = wl_shm_buffer_get_data(shm);
                for (int y = 0; y < h; ++y)
                    memcpy(frame + (size_t)y * w, data + (size_t)y * stride, (size_t)w * 4);
                wl_shm_buffer_end_access(shm);
                frame_w = w; frame_h = h; ++frames;
            }
        }
        wl_buffer_send_release(s->pending);
        s->pending = NULL;
    }
    struct wl_resource *cb, *tmp;
    wl_resource_for_each_safe(cb, tmp, &s->callbacks) {
        wl_callback_send_done(cb, 0);
        wl_resource_destroy(cb);
    }
}
static void surface_transform(struct wl_client *c, struct wl_resource *r, int32_t v) { (void)c; (void)r; (void)v; }
static void surface_offset(struct wl_client *c, struct wl_resource *r, int32_t x, int32_t y) { (void)c; (void)r; (void)x; (void)y; }
static const struct wl_surface_interface surface_impl = {
    destroy_resource, surface_attach, surface_damage, surface_frame, surface_region, surface_region,
    surface_commit, surface_transform, surface_transform, surface_damage, surface_offset,
};
static void surface_free(struct wl_resource *r)
{
    struct surface *s = wl_resource_get_user_data(r);
    struct wl_resource *cb, *tmp;
    wl_resource_for_each_safe(cb, tmp, &s->callbacks) wl_resource_destroy(cb);
    free(s);
}

static void compositor_surface(struct wl_client *c, struct wl_resource *r, uint32_t id)
{
    struct surface *s = calloc(1, sizeof *s);
    wl_list_init(&s->callbacks);
    s->resource = wl_resource_create(c, &wl_surface_interface, wl_resource_get_version(r), id);
    wl_resource_set_implementation(s->resource, &surface_impl, s, surface_free);
}
static void compositor_region(struct wl_client *c, struct wl_resource *r, uint32_t id)
{
    struct wl_resource *region = wl_resource_create(c, &wl_region_interface, wl_resource_get_version(r), id);
    wl_resource_set_implementation(region, &region_impl, NULL, NULL);
}
static const struct wl_compositor_interface compositor_impl = { compositor_surface, compositor_region };
static void bind_compositor(struct wl_client *c, void *d, uint32_t v, uint32_t id)
{
    (void)d;
    struct wl_resource *r = wl_resource_create(c, &wl_compositor_interface, v, id);
    wl_resource_set_implementation(r, &compositor_impl, NULL, NULL);
}

// wl_shell: every surface is a toplevel of the output's size.
static void shsurf_pong(struct wl_client *c, struct wl_resource *r, uint32_t serial) { (void)c; (void)r; (void)serial; }
static void shsurf_move(struct wl_client *c, struct wl_resource *r, struct wl_resource *seat, uint32_t serial)
{ (void)c; (void)r; (void)seat; (void)serial; }
static void shsurf_resize(struct wl_client *c, struct wl_resource *r, struct wl_resource *seat, uint32_t serial, uint32_t edges)
{ (void)c; (void)r; (void)seat; (void)serial; (void)edges; }
static void shsurf_toplevel(struct wl_client *c, struct wl_resource *r)
{ (void)c; wl_shell_surface_send_configure(r, 0, width, height); }
static void shsurf_transient(struct wl_client *c, struct wl_resource *r, struct wl_resource *p, int32_t x, int32_t y, uint32_t f)
{ (void)c; (void)r; (void)p; (void)x; (void)y; (void)f; }
static void shsurf_fullscreen(struct wl_client *c, struct wl_resource *r, uint32_t m, uint32_t f, struct wl_resource *o)
{ (void)c; (void)m; (void)f; (void)o; wl_shell_surface_send_configure(r, 0, width, height); }
static void shsurf_popup(struct wl_client *c, struct wl_resource *r, struct wl_resource *seat, uint32_t serial,
                         struct wl_resource *p, int32_t x, int32_t y, uint32_t f)
{ (void)c; (void)r; (void)seat; (void)serial; (void)p; (void)x; (void)y; (void)f; }
static void shsurf_maximized(struct wl_client *c, struct wl_resource *r, struct wl_resource *o)
{ (void)c; (void)o; wl_shell_surface_send_configure(r, 0, width, height); }
static void shsurf_title(struct wl_client *c, struct wl_resource *r, const char *t) { (void)c; (void)r; (void)t; }
static const struct wl_shell_surface_interface shsurf_impl = {
    shsurf_pong, shsurf_move, shsurf_resize, shsurf_toplevel, shsurf_transient, shsurf_fullscreen,
    shsurf_popup, shsurf_maximized, shsurf_title, shsurf_title,
};
static void shell_get(struct wl_client *c, struct wl_resource *r, uint32_t id, struct wl_resource *surface)
{
    (void)surface;
    struct wl_resource *s = wl_resource_create(c, &wl_shell_surface_interface, wl_resource_get_version(r), id);
    wl_resource_set_implementation(s, &shsurf_impl, NULL, NULL);
}
static const struct wl_shell_interface shell_impl = { shell_get };
static void bind_shell(struct wl_client *c, void *d, uint32_t v, uint32_t id)
{
    (void)d;
    struct wl_resource *r = wl_resource_create(c, &wl_shell_interface, v, id);
    wl_resource_set_implementation(r, &shell_impl, NULL, NULL);
}

static const struct wl_output_interface output_impl = { destroy_resource };
static void bind_output(struct wl_client *c, void *d, uint32_t v, uint32_t id)
{
    (void)d;
    struct wl_resource *r = wl_resource_create(c, &wl_output_interface, v, id);
    wl_resource_set_implementation(r, &output_impl, NULL, NULL);
    // A phone's portrait screen, 62 x 110 mm.
    wl_output_send_geometry(r, 0, 0, 62, 110, WL_OUTPUT_SUBPIXEL_UNKNOWN, "test", "test",
                            WL_OUTPUT_TRANSFORM_NORMAL);
    wl_output_send_mode(r, WL_OUTPUT_MODE_CURRENT | WL_OUTPUT_MODE_PREFERRED, width, height, 60000);
    if (v >= 2) wl_output_send_done(r);
}

static int write_and_quit(void *data)
{
    (void)data;
    FILE *f = fopen(out_path, "wb");
    if (!f || !frame) {
        fprintf(stderr, "shotcompositor: no frame (%d frames)\n", frames);
        exit(1);
    }
    fprintf(f, "P6\n%d %d\n255\n", frame_w, frame_h);
    for (int i = 0; i < frame_w * frame_h; ++i) {
        uint32_t p = frame[i]; // ARGB8888/XRGB8888, little endian
        unsigned char rgb[3] = { (p >> 16) & 255, (p >> 8) & 255, p & 255 };
        fwrite(rgb, 1, 3, f);
    }
    fclose(f);
    fprintf(stderr, "shotcompositor: %dx%d, %d frames\n", frame_w, frame_h, frames);
    wl_display_terminate(display);
    return 0;
}

int main(int argc, char **argv)
{
    if (argc < 3) {
        fprintf(stderr, "usage: %s out.ppm seconds\n", argv[0]);
        return 2;
    }
    out_path = argv[1];
    display = wl_display_create();
    const char *socket = wl_display_add_socket_auto(display);
    if (!socket) return 1;
    printf("%s\n", socket);
    fflush(stdout);
    wl_display_init_shm(display);
    wl_global_create(display, &wl_compositor_interface, 4, NULL, bind_compositor);
    wl_global_create(display, &wl_shell_interface, 1, NULL, bind_shell);
    wl_global_create(display, &wl_output_interface, 2, NULL, bind_output);
    struct wl_event_source *t = wl_event_loop_add_timer(wl_display_get_event_loop(display), write_and_quit, NULL);
    wl_event_source_timer_update(t, atoi(argv[2]) * 1000);
    signal(SIGPIPE, SIG_IGN);
    wl_display_run(display);
    wl_display_destroy(display);
    return 0;
}
