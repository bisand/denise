/*
 * The C ABI end to end, in the shape a real host uses it.
 *
 *   cd denise-ffi/examples && make && ./panel panel.ppm
 *
 * There is no window here: the host owns the buffer, and this one is malloc'd
 * and written out as a PPM so you can look at the result on a machine with no
 * display at all. Swap the buffer for a DIB section and this is the Win32
 * control; swap it for a CGBitmapContext and it is the NSView.
 *
 * Three things it demonstrates that are easy to get wrong from C:
 *
 *   - The stride is deliberately not the width. Denise never writes past the
 *     visible width of a row, and a host that assumes rows are contiguous works
 *     everywhere until it meets a pitch-aligned framebuffer.
 *   - Keys and text are separate calls. The key is a position and drives Tab
 *     and Enter; the text is what the layout finally committed, which is the
 *     only way 'ø' can arrive at all.
 *   - Damage is read after painting and retired after presenting, not before.
 *     Retiring early forgets exactly what a failed present would need again.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "denise.h"

#define WIDTH 480
#define HEIGHT 320
/* Not WIDTH. See above. */
#define STRIDE (WIDTH + 13)

enum {
    MSG_NONE = 0, /* the ABI reserves 0; a widget given it emits nothing */
    MSG_SAVE = 1,
    MSG_RESET = 2
};

static int failed = 0;

static void check(int32_t status, const char *what)
{
    if (status != DENISE_OK) {
        fprintf(stderr, "%s: %s\n", what, denise_status_message(status));
        failed = 1;
    }
}

/* Down then up at the same point, which is what a click is. */
static void click(DeniseUi *ui, int32_t x, int32_t y)
{
    check(denise_ui_pointer_moved(ui, x, y), "pointer_moved");
    check(denise_ui_pointer_button(ui, DENISE_BUTTON_LEFT, true, x, y, 0), "button down");
    check(denise_ui_pointer_button(ui, DENISE_BUTTON_LEFT, false, x, y, 0), "button up");
}

/* A key position, then the character the layout produced from it. */
static void type_char(DeniseUi *ui, uint32_t key, uint32_t codepoint)
{
    check(denise_ui_key(ui, key, true, false, 0), "key down");
    check(denise_ui_text(ui, codepoint), "text");
    check(denise_ui_key(ui, key, false, false, 0), "key up");
}

static int write_ppm(const char *path, const uint32_t *pixels)
{
    FILE *f = fopen(path, "wb");
    if (!f) {
        perror(path);
        return 0;
    }
    fprintf(f, "P6\n%d %d\n255\n", WIDTH, HEIGHT);
    for (int y = 0; y < HEIGHT; y++) {
        for (int x = 0; x < WIDTH; x++) {
            uint32_t p = pixels[(size_t)y * STRIDE + (size_t)x];
            /* 0xAARRGGBB in a native-endian word, so this is shifts, not bytes. */
            unsigned char rgb[3] = {
                (unsigned char)((p >> 16) & 0xFF),
                (unsigned char)((p >> 8) & 0xFF),
                (unsigned char)(p & 0xFF)
            };
            fwrite(rgb, 1, 3, f);
        }
    }
    fclose(f);
    return 1;
}

int main(int argc, char **argv)
{
    const char *out = argc > 1 ? argv[1] : "panel.ppm";

    if (denise_abi_version() != DENISE_ABI_VERSION) {
        fprintf(stderr, "ABI mismatch: header %u, library %u\n",
                (unsigned)DENISE_ABI_VERSION, (unsigned)denise_abi_version());
        return 1;
    }
    printf("denise  %s (ABI %u)\n", denise_version(), (unsigned)denise_abi_version());

    DeniseUi *ui = denise_ui_new(WIDTH, HEIGHT, DENISE_THEME_DARK);
    if (!ui) {
        fprintf(stderr, "denise_ui_new failed\n");
        return 1;
    }

    /* A real host has a system cursor already, so Denise must not composite a
     * second one that trails it by a frame. Said once, it stays said. */
    check(denise_ui_show_cursor(ui, false), "show_cursor");

    uint64_t root = denise_ui_root(ui);
    DeniseRect card = { 40, 30, WIDTH - 80, HEIGHT - 60 };
    uint64_t panel = denise_ui_add_panel(ui, root, card,
                                         DENISE_ROLE_BASE_200, DENISE_ROLE_BASE_300, 1);

    /* Children are positioned relative to their parent. There is no layout
     * engine, which is what a fixed-resolution panel actually wants. */
    DeniseRect title_r = { 24, 20, card.width - 48, 28 };
    denise_ui_add_label(ui, panel, title_r, "Operator sign-in", DENISE_ROLE_BASE_CONTENT);

    DeniseRect field_r = { 24, 70, card.width - 48, 40 };
    uint64_t name = denise_ui_add_text_input(ui, panel, field_r, "Ola Nordmann",
                                             MSG_SAVE, 0, false);

    DeniseRect save_r = { 24, 130, 140, 44 };
    denise_ui_add_button(ui, panel, save_r, "Lagre", MSG_SAVE, DENISE_ROLE_PRIMARY);

    DeniseRect reset_r = { 180, 130, 140, 44 };
    denise_ui_add_button(ui, panel, reset_r, "Nullstill", MSG_RESET, DENISE_ROLE_NEUTRAL);

    DeniseRect note_r = { 24, 190, card.width - 48, 24 };
    uint64_t note = denise_ui_add_label(ui, panel, note_r, "", DENISE_ROLE_BASE_300);

    if (name == DENISE_NODE_NONE || note == DENISE_NODE_NONE) {
        fprintf(stderr, "a widget was not created\n");
        denise_ui_free(ui);
        return 1;
    }

    /* Type into the field, the long way round: 'æ' is on no US position, so the
     * key and the character genuinely differ. */
    check(denise_ui_focus(ui, name), "focus");
    type_char(ui, DENISE_KEY_K, 'K');
    type_char(ui, DENISE_KEY_J, 'j');
    type_char(ui, DENISE_KEY_QUOTE, 0xE6);  /* æ */
    type_char(ui, DENISE_KEY_R, 'r');

    char typed[64];
    intptr_t n = denise_ui_get_text(ui, name, typed, sizeof typed);
    if (n < 0) {
        fprintf(stderr, "get_text: %s\n", denise_status_message((int32_t)n));
        failed = 1;
    } else {
        printf("typed   \"%s\" (%d bytes)\n", typed, (int)n);
    }

    /* Press Lagre and read what it sent. */
    click(ui, card.x + save_r.x + 40, card.y + save_r.y + 20);

    uint32_t message;
    while (denise_ui_poll_message(ui, &message)) {
        printf("message %u%s\n", (unsigned)message,
               message == MSG_SAVE ? " (save)" : message == MSG_RESET ? " (reset)" : "");
        if (message == MSG_SAVE) {
            check(denise_ui_set_text(ui, note, "Lagret."), "set_text");
        }
    }

    /* One frame. `calloc` so the padding starting out zero means something when
     * it is still zero afterwards. */
    uint32_t *pixels = calloc((size_t)STRIDE * HEIGHT, sizeof *pixels);
    if (!pixels) {
        fprintf(stderr, "out of memory\n");
        denise_ui_free(ui);
        return 1;
    }

    check(denise_ui_tick(ui, 0), "tick");
    if (!denise_ui_needs_paint(ui)) {
        fprintf(stderr, "nothing to paint, which cannot be right on frame one\n");
        failed = 1;
    }

    DeniseFrame frame;
    frame.pixels = pixels;
    frame.len = (size_t)STRIDE * HEIGHT;
    frame.width = WIDTH;
    frame.height = HEIGHT;
    frame.stride = STRIDE;
    frame.format = DENISE_FORMAT_XRGB8888;
    /* First frame of this buffer: contents undefined, repaint everything. */
    frame.buffer_age = -1;
    check(denise_ui_paint(ui, &frame), "paint");

    DeniseRect damage[DENISE_MAX_DAMAGE_RECTS];
    intptr_t rects = denise_ui_damage(ui, damage, DENISE_MAX_DAMAGE_RECTS);
    if (rects < 0) {
        fprintf(stderr, "damage: %s\n", denise_status_message((int32_t)rects));
        failed = 1;
        rects = 0;
    }
    printf("damage  %d rectangle%s\n", (int)rects, rects == 1 ? "" : "s");
    for (intptr_t i = 0; i < rects && i < DENISE_MAX_DAMAGE_RECTS; i++) {
        printf("        %dx%d at %d,%d\n",
               damage[i].width, damage[i].height, damage[i].x, damage[i].y);
    }
    /* Only after the blit — here, after writing the file. */
    check(denise_ui_presented(ui), "presented");

    /* Denise promised never to touch the columns past `width`. Check it, because
     * this is the one mistake that looks like a driver bug rather than a host
     * one. */
    for (int y = 0; y < HEIGHT; y++) {
        for (int x = WIDTH; x < STRIDE; x++) {
            if (pixels[(size_t)y * STRIDE + (size_t)x] != 0) {
                fprintf(stderr, "row %d column %d is past the visible width\n", y, x);
                failed = 1;
                y = HEIGHT;
                break;
            }
        }
    }

    if (!write_ppm(out, pixels)) {
        failed = 1;
    } else {
        printf("wrote   %s (%dx%d)\n", out, WIDTH, HEIGHT);
    }

    free(pixels);
    denise_ui_free(ui);
    return failed;
}
