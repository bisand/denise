/*
 * Denise — a direct-rendering UI toolkit for systems without a desktop
 * environment. https://github.com/bisand/denise
 *
 * MIT licensed. This header is the ABI contract: it is written by hand and the
 * Rust is checked against it, not the other way round. A generated header
 * follows whatever the implementation happens to say this week, which is the
 * opposite of what "stable ABI" means.
 *
 * THE SHAPE OF IT
 *
 *   The host owns the window and the pixel buffer. Denise owns the widget tree
 *   and draws into whatever it is handed. There is no surface here and no event
 *   loop; both belong to the host, and a library that owned either would be
 *   unembeddable in the places this exists for.
 *
 *     DeniseUi *ui = denise_ui_new(800, 480, DENISE_THEME_DARK);
 *     uint64_t root = denise_ui_root(ui);
 *     DeniseRect r = { 20, 20, 160, 44 };
 *     denise_ui_add_button(ui, root, r, "Save", MSG_SAVE, DENISE_ROLE_PRIMARY);
 *
 *     for (;;) {
 *         denise_ui_tick(ui, now_ms());
 *         if (denise_ui_needs_paint(ui)) {
 *             DeniseFrame f = { pixels, len, w, h, stride,
 *                               DENISE_FORMAT_XRGB8888, age };
 *             denise_ui_paint(ui, &f);
 *             DeniseRect damage[DENISE_MAX_DAMAGE_RECTS];
 *             intptr_t n = denise_ui_damage(ui, damage, DENISE_MAX_DAMAGE_RECTS);
 *             blit(damage, n);
 *             denise_ui_presented(ui);
 *         }
 *         uint32_t msg;
 *         while (denise_ui_poll_message(ui, &msg)) handle(msg);
 *     }
 *
 * RULES THE WHOLE ABI KEEPS
 *
 *   - Handles are opaque. A DeniseUi* comes from denise_ui_new and goes to
 *     denise_ui_free. Nothing else may free it.
 *   - A node is a uint64_t and DENISE_NODE_NONE (0) is never a valid one. Ids
 *     carry a generation, so an id kept past a denise_ui_remove fails to
 *     resolve rather than addressing whoever took the slot.
 *   - A message is a uint32_t you choose, and 0 means "no message". A button
 *     given 0 emits nothing and denise_ui_poll_message never yields it.
 *   - Strings are NUL-terminated UTF-8, in and out. Invalid UTF-8 is refused
 *     rather than mangled.
 *   - A negative return is a DeniseStatus, and denise_status_message describes
 *     every one of them.
 *   - Nothing is thread-safe. One DeniseUi belongs to one thread.
 *   - Panics do not cross. A bug inside Denise returns DENISE_ERR_PANIC rather
 *     than taking your process down; the call did nothing, and that DeniseUi
 *     should be freed.
 */

#ifndef DENISE_H
#define DENISE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#if defined(_WIN32) && !defined(DENISE_STATIC)
#  ifdef DENISE_BUILDING
#    define DENISE_API __declspec(dllexport)
#  else
#    define DENISE_API __declspec(dllimport)
#  endif
#else
#  define DENISE_API
#endif

/* Bumped when a signature, a constant, or the meaning of one changes. Adding a
 * function does not bump it. Check it once at load time. */
#define DENISE_ABI_VERSION 1u

/* The most rectangles denise_ui_damage can ever report. Sized so a caller can
 * put the array on the stack and stop thinking about it. */
#define DENISE_MAX_DAMAGE_RECTS 16

/* Not a node: the absence of focus, a constructor that failed. */
#define DENISE_NODE_NONE ((uint64_t)0)

/* ------------------------------------------------------------------ status */

typedef enum DeniseStatus {
    DENISE_OK = 0,
    /* A required pointer was NULL. */
    DENISE_ERR_NULL = -1,
    /* An argument was out of range, or a string was not valid UTF-8. */
    DENISE_ERR_INVALID = -2,
    /* The node id does not name a live node. */
    DENISE_ERR_NO_NODE = -3,
    /* The buffer supplied is too small for the result. */
    DENISE_ERR_BUFFER_TOO_SMALL = -4,
    /* The node is not a widget of the kind that call needs. */
    DENISE_ERR_WRONG_WIDGET = -5,
    /* Denise panicked. The call did nothing; free that DeniseUi. */
    DENISE_ERR_PANIC = -6
} DeniseStatus;

/* ------------------------------------------------------------------- theme */

/* A widget never names a colour, it names a role, and the theme decides what
 * that means. Every surface role has a *_CONTENT partner guaranteed to reach
 * 3:1 against it, so a theme cannot be configured into illegibility. */
typedef enum DeniseRole {
    DENISE_ROLE_NONE = -1,       /* no fill, no border */
    DENISE_ROLE_BASE_100 = 0,    /* page and panel background */
    DENISE_ROLE_BASE_200 = 1,    /* a recessed surface */
    DENISE_ROLE_BASE_300 = 2,    /* borders and dividers */
    DENISE_ROLE_BASE_CONTENT = 3,
    DENISE_ROLE_PRIMARY = 4,     /* the main action */
    DENISE_ROLE_PRIMARY_CONTENT = 5,
    DENISE_ROLE_SECONDARY = 6,
    DENISE_ROLE_SECONDARY_CONTENT = 7,
    DENISE_ROLE_ACCENT = 8,
    DENISE_ROLE_ACCENT_CONTENT = 9,
    DENISE_ROLE_NEUTRAL = 10,
    DENISE_ROLE_NEUTRAL_CONTENT = 11,
    DENISE_ROLE_INFO = 12,
    DENISE_ROLE_INFO_CONTENT = 13,
    DENISE_ROLE_SUCCESS = 14,
    DENISE_ROLE_SUCCESS_CONTENT = 15,
    DENISE_ROLE_WARNING = 16,
    DENISE_ROLE_WARNING_CONTENT = 17,
    DENISE_ROLE_ERROR = 18,
    DENISE_ROLE_ERROR_CONTENT = 19
} DeniseRole;

#define DENISE_THEME_LIGHT 0u
#define DENISE_THEME_DARK 1u
#define DENISE_THEME_HIGH_CONTRAST 2u

/* ------------------------------------------------------------------ pixels */

/* One word is 0xAARRGGBB in NATIVE byte order. On a little-endian machine that
 * is B, G, R, A in memory — which is what DRM's XRGB8888 and a Win32 BI_RGB DIB
 * section both mean, and is NOT what an RGBA byte-order library means. */
#define DENISE_FORMAT_ARGB8888 0u  /* alpha honoured */
#define DENISE_FORMAT_XRGB8888 1u  /* high byte ignored */

typedef struct DeniseRect {
    int32_t x;
    int32_t y;
    int32_t width;
    int32_t height;
} DeniseRect;

/* A pixel buffer you own, described for one call to denise_ui_paint.
 *
 * stride is in PIXELS and may exceed width. A host that assumes rows are
 * contiguous works on a desktop and shears on a pitch-aligned framebuffer.
 *
 * buffer_age is how many presents ago this buffer's contents are from, or
 * negative for "undefined, repaint everything" — modelled on
 * EGL_EXT_buffer_age. Report what is true: a double-buffered host that claims 1
 * every frame shows stale content on alternate frames, which looks like
 * flicker and is actually arithmetic. */
typedef struct DeniseFrame {
    uint32_t *pixels;
    size_t len;          /* in words, not bytes */
    uint32_t width;
    uint32_t height;
    uint32_t stride;     /* in pixels */
    uint32_t format;
    int32_t buffer_age;
} DeniseFrame;

/* ------------------------------------------------------------------- input */

#define DENISE_BUTTON_LEFT 0u
#define DENISE_BUTTON_RIGHT 1u
#define DENISE_BUTTON_MIDDLE 2u
/* Set on any further button; the low bits carry the platform's own index. */
#define DENISE_BUTTON_OTHER 0x100u

#define DENISE_MOD_SHIFT 0x1u
#define DENISE_MOD_CTRL 0x2u
#define DENISE_MOD_ALT 0x4u
#define DENISE_MOD_SUPER 0x8u

#define DENISE_TOUCH_DOWN 0u
#define DENISE_TOUCH_MOVED 1u
#define DENISE_TOUCH_UP 2u
#define DENISE_TOUCH_CANCELLED 3u  /* the system took it away; not a lift */

/* A key POSITION, named after the US layout as a convention. It is not a
 * character: DENISE_KEY_SEMICOLON is where 'ø' lives on a Norwegian keyboard,
 * and what was actually typed arrives through denise_ui_text instead.
 *
 * Positions that carry an ASCII character on the US layout are numbered with
 * it, so a key log is readable in hex and half the table needs no lookup.
 * Everything else lives in a block: 0x100 for named keys, 0x200 + n for F<n>,
 * 0x300 + n for the numpad digits. */
typedef enum DeniseKey {
    DENISE_KEY_A = 0x41,
    DENISE_KEY_B = 0x42,
    DENISE_KEY_C = 0x43,
    DENISE_KEY_D = 0x44,
    DENISE_KEY_E = 0x45,
    DENISE_KEY_F = 0x46,
    DENISE_KEY_G = 0x47,
    DENISE_KEY_H = 0x48,
    DENISE_KEY_I = 0x49,
    DENISE_KEY_J = 0x4a,
    DENISE_KEY_K = 0x4b,
    DENISE_KEY_L = 0x4c,
    DENISE_KEY_M = 0x4d,
    DENISE_KEY_N = 0x4e,
    DENISE_KEY_O = 0x4f,
    DENISE_KEY_P = 0x50,
    DENISE_KEY_Q = 0x51,
    DENISE_KEY_R = 0x52,
    DENISE_KEY_S = 0x53,
    DENISE_KEY_T = 0x54,
    DENISE_KEY_U = 0x55,
    DENISE_KEY_V = 0x56,
    DENISE_KEY_W = 0x57,
    DENISE_KEY_X = 0x58,
    DENISE_KEY_Y = 0x59,
    DENISE_KEY_Z = 0x5a,
    DENISE_KEY_0 = 0x30,
    DENISE_KEY_1 = 0x31,
    DENISE_KEY_2 = 0x32,
    DENISE_KEY_3 = 0x33,
    DENISE_KEY_4 = 0x34,
    DENISE_KEY_5 = 0x35,
    DENISE_KEY_6 = 0x36,
    DENISE_KEY_7 = 0x37,
    DENISE_KEY_8 = 0x38,
    DENISE_KEY_9 = 0x39,
    DENISE_KEY_SPACE = 0x20,
    DENISE_KEY_QUOTE = 0x27,
    DENISE_KEY_COMMA = 0x2c,
    DENISE_KEY_MINUS = 0x2d,
    DENISE_KEY_PERIOD = 0x2e,
    DENISE_KEY_SLASH = 0x2f,
    DENISE_KEY_SEMICOLON = 0x3b,
    DENISE_KEY_EQUAL = 0x3d,
    DENISE_KEY_BRACKET_LEFT = 0x5b,
    DENISE_KEY_BACKSLASH = 0x5c,
    DENISE_KEY_BRACKET_RIGHT = 0x5d,
    DENISE_KEY_BACKQUOTE = 0x60,
    DENISE_KEY_ESCAPE = 0x100,
    DENISE_KEY_ENTER = 0x101,
    DENISE_KEY_TAB = 0x102,
    DENISE_KEY_BACKSPACE = 0x103,
    DENISE_KEY_DELETE = 0x104,
    DENISE_KEY_INSERT = 0x105,
    DENISE_KEY_HOME = 0x106,
    DENISE_KEY_END = 0x107,
    DENISE_KEY_PAGE_UP = 0x108,
    DENISE_KEY_PAGE_DOWN = 0x109,
    DENISE_KEY_ARROW_UP = 0x10a,
    DENISE_KEY_ARROW_DOWN = 0x10b,
    DENISE_KEY_ARROW_LEFT = 0x10c,
    DENISE_KEY_ARROW_RIGHT = 0x10d,
    DENISE_KEY_CAPS_LOCK = 0x10e,
    DENISE_KEY_NUM_LOCK = 0x10f,
    DENISE_KEY_SCROLL_LOCK = 0x110,
    DENISE_KEY_INTL_BACKSLASH = 0x111,
    DENISE_KEY_SHIFT_LEFT = 0x120,
    DENISE_KEY_SHIFT_RIGHT = 0x121,
    DENISE_KEY_CONTROL_LEFT = 0x122,
    DENISE_KEY_CONTROL_RIGHT = 0x123,
    DENISE_KEY_ALT_LEFT = 0x124,
    DENISE_KEY_ALT_RIGHT = 0x125,
    DENISE_KEY_SUPER_LEFT = 0x126,
    DENISE_KEY_SUPER_RIGHT = 0x127,
    DENISE_KEY_F1 = 0x201,
    DENISE_KEY_F2 = 0x202,
    DENISE_KEY_F3 = 0x203,
    DENISE_KEY_F4 = 0x204,
    DENISE_KEY_F5 = 0x205,
    DENISE_KEY_F6 = 0x206,
    DENISE_KEY_F7 = 0x207,
    DENISE_KEY_F8 = 0x208,
    DENISE_KEY_F9 = 0x209,
    DENISE_KEY_F10 = 0x20a,
    DENISE_KEY_F11 = 0x20b,
    DENISE_KEY_F12 = 0x20c,
    DENISE_KEY_NUMPAD_0 = 0x300,
    DENISE_KEY_NUMPAD_1 = 0x301,
    DENISE_KEY_NUMPAD_2 = 0x302,
    DENISE_KEY_NUMPAD_3 = 0x303,
    DENISE_KEY_NUMPAD_4 = 0x304,
    DENISE_KEY_NUMPAD_5 = 0x305,
    DENISE_KEY_NUMPAD_6 = 0x306,
    DENISE_KEY_NUMPAD_7 = 0x307,
    DENISE_KEY_NUMPAD_8 = 0x308,
    DENISE_KEY_NUMPAD_9 = 0x309,
    DENISE_KEY_NUMPAD_ENTER = 0x30a,
    DENISE_KEY_NUMPAD_ADD = 0x30b,
    DENISE_KEY_NUMPAD_SUBTRACT = 0x30c,
    DENISE_KEY_NUMPAD_MULTIPLY = 0x30d,
    DENISE_KEY_NUMPAD_DIVIDE = 0x30e,
    DENISE_KEY_NUMPAD_DECIMAL = 0x30f
} DeniseKey;

/* Set on a key this build cannot name; the low bits carry the raw platform
 * scancode, so two unknown keys are still distinguishable. */
#define DENISE_KEY_UNIDENTIFIED 0x80000000u

/* ---------------------------------------------------------------- library */

/* Denise's version, NUL-terminated. Never NULL, never freed. */
DENISE_API const char *denise_version(void);

/* The ABI version this library was built with. Compare with
 * DENISE_ABI_VERSION, which is what your header was built with. */
DENISE_API uint32_t denise_abi_version(void);

/* A description of a status code, NUL-terminated. Never NULL, never freed —
 * an unrecognised code gets a generic message, so logging an error cannot
 * itself produce one. */
DENISE_API const char *denise_status_message(int32_t status);

/* -------------------------------------------------------------- lifecycle */

typedef struct DeniseUi DeniseUi;

/* Creates a user interface of width x height pixels in one of the built-in
 * themes. Returns NULL if the size is empty or the theme is not one of them. */
DENISE_API DeniseUi *denise_ui_new(uint32_t width, uint32_t height, uint32_t theme);

/* As denise_ui_new, with the theme's metrics — control heights, radii, borders —
 * at a scale factor given in hundredths: 150 is 1.5x, 200 a 2x display. The
 * rectangles the host passes stay physical pixels; the host computes them, so
 * the host multiplies them. Rebuild with a new factor on a DPI change. */
DENISE_API DeniseUi *denise_ui_new_scaled(uint32_t width, uint32_t height,
                                          uint32_t theme, uint32_t scale_x100);

/* Destroys it. NULL is accepted and does nothing, as free does. Every node id
 * taken from it is dead afterwards. */
DENISE_API void denise_ui_free(DeniseUi *ui);

/* Switches theme and repaints everything. */
DENISE_API int32_t denise_ui_set_theme(DeniseUi *ui, uint32_t theme);

/* Writes the surface size. Either output may be NULL. */
DENISE_API int32_t denise_ui_size(DeniseUi *ui, uint32_t *width, uint32_t *height);

/* Takes the next message, or returns false if there is none. Never yields 0. */
DENISE_API bool denise_ui_poll_message(DeniseUi *ui, uint32_t *out);

/* ------------------------------------------------------------------- tree */

/* The root of the base scene. Never DENISE_NODE_NONE for a live handle. */
DENISE_API uint64_t denise_ui_root(DeniseUi *ui);

/* The root of the topmost scene — the one input reaches. */
DENISE_API uint64_t denise_ui_top_root(DeniseUi *ui);

/* A themed rectangle: the background other widgets sit on. fill and border are
 * DeniseRole values, or DENISE_ROLE_NONE. Panels are invisible to hit testing,
 * so putting a button on one does not cost the click. */
DENISE_API uint64_t denise_ui_add_panel(DeniseUi *ui, uint64_t parent, DeniseRect layout,
                                        int32_t fill, int32_t border, int32_t border_width);

/* Static text. */
DENISE_API uint64_t denise_ui_add_label(DeniseUi *ui, uint64_t parent, DeniseRect layout,
                                        const char *text, int32_t role);

/* A button that emits message when activated. A message of 0 makes it inert:
 * it still draws, still lights on hover and press, and emits nothing. */
DENISE_API uint64_t denise_ui_add_button(DeniseUi *ui, uint64_t parent, DeniseRect layout,
                                         const char *label, uint32_t message, int32_t role);

/* An editable single-line field. placeholder may be NULL; submit is emitted on
 * Enter, or 0 for nothing; max_chars of 0 means unlimited. */
DENISE_API uint64_t denise_ui_add_text_input(DeniseUi *ui, uint64_t parent, DeniseRect layout,
                                             const char *placeholder, uint32_t submit,
                                             uint32_t max_chars, bool password);

/* Pushes a modal scene over everything below, dimmed by dim (0 none, 255
 * opaque), and returns its root. Input only reaches the topmost scene, so
 * nothing underneath is clickable, focusable or reachable by Tab. */
DENISE_API uint64_t denise_ui_push_scene(DeniseUi *ui, uint8_t dim);

/* Closes the topmost scene and everything in it. False if only the base scene
 * is left, which is never removable. */
DENISE_API bool denise_ui_pop_scene(DeniseUi *ui);

/* How many scenes are stacked. At least 1. */
DENISE_API uint32_t denise_ui_scene_count(DeniseUi *ui);

DENISE_API int32_t denise_ui_remove(DeniseUi *ui, uint64_t node);
DENISE_API int32_t denise_ui_set_layout(DeniseUi *ui, uint64_t node, DeniseRect layout);
DENISE_API int32_t denise_ui_bounds(DeniseUi *ui, uint64_t node, DeniseRect *out);
DENISE_API int32_t denise_ui_set_visible(DeniseUi *ui, uint64_t node, bool visible);
DENISE_API int32_t denise_ui_set_enabled(DeniseUi *ui, uint64_t node, bool enabled);
DENISE_API int32_t denise_ui_set_z(DeniseUi *ui, uint64_t node, int32_t z);

/* Gives keyboard focus to a node, or clears it with DENISE_NODE_NONE. */
DENISE_API int32_t denise_ui_focus(DeniseUi *ui, uint64_t node);

/* Replaces a label's text, a button's caption or a field's contents. */
DENISE_API int32_t denise_ui_set_text(DeniseUi *ui, uint64_t node, const char *text);

/* Copies a widget's text out as NUL-terminated UTF-8, returning its length in
 * bytes excluding the NUL, or a negative status. Call with out NULL and cap 0
 * to ask how much room it needs; allocate one more byte than it says. */
DENISE_API intptr_t denise_ui_get_text(DeniseUi *ui, uint64_t node, char *out, size_t cap);

/* ------------------------------------------------------------------ input */

DENISE_API int32_t denise_ui_pointer_moved(DeniseUi *ui, int32_t x, int32_t y);
DENISE_API int32_t denise_ui_pointer_button(DeniseUi *ui, uint32_t button, bool down,
                                            int32_t x, int32_t y, uint32_t modifiers);
DENISE_API int32_t denise_ui_pointer_scroll(DeniseUi *ui, float delta_x, float delta_y,
                                            int32_t x, int32_t y);

/* The pointer left the surface. Worth sending: without it a button stays lit
 * under a cursor that is somewhere else entirely. */
DENISE_API int32_t denise_ui_pointer_left(DeniseUi *ui);

/* One finger. phase is one of the DENISE_TOUCH_* values, and id identifies
 * that finger for as long as it is down. */
DENISE_API int32_t denise_ui_touch(DeniseUi *ui, uint64_t id, uint32_t phase,
                                   int32_t x, int32_t y);

/* A key position went down or up. This drives navigation and shortcuts; what
 * was typed goes through denise_ui_text. */
DENISE_API int32_t denise_ui_key(DeniseUi *ui, uint32_t key, bool down, bool repeat,
                                 uint32_t modifiers);

/* One committed character, as a Unicode scalar value. Send it after the key
 * that produced it, and send nothing for a dead key — '¨' then 'o' is one call
 * with U+00F6. Control characters are refused: Enter, Tab and Backspace are
 * keys, and a host that sends them as text too makes a field insert a '\r' it
 * can never show. */
DENISE_API int32_t denise_ui_text(DeniseUi *ui, uint32_t codepoint);

/* Advances the clock, which drives the caret blink and any animation. now_ms
 * is monotonic and its origin does not matter. Call once a frame, before
 * painting. */
DENISE_API int32_t denise_ui_tick(DeniseUi *ui, uint64_t now_ms);

/* When the next tick is due, on the same clock, or -1 if nothing is animating.
 * This is what lets a host block instead of poll: a SetTimer interval on
 * Win32, a poll timeout on a bare frame loop. Without it the choice is a
 * spinning idle loop or a caret that does not blink. */
DENISE_API int64_t denise_ui_next_wake_ms(DeniseUi *ui);

/* Shows or hides the composited cursor sprite, and stops Denise deciding.
 *
 * Left alone the sprite appears on the first pointer motion and disappears when
 * a finger arrives, which is what a panel with no window system underneath
 * wants. An embedded host is the other case: it already has a system cursor,
 * and a second one a frame behind it is worse than none. Call this once with
 * false at startup and it stays off. */
DENISE_API int32_t denise_ui_show_cursor(DeniseUi *ui, bool visible);

/* --------------------------------------------------------------- painting */

/* Whether anything has been marked dirty since the last denise_ui_presented.
 * Ignoring this and painting unconditionally is correct and just costs more —
 * on a panel showing an unchanging screen, all of it. */
DENISE_API bool denise_ui_needs_paint(DeniseUi *ui);

/* Draws every damaged region into the buffer frame describes. */
DENISE_API int32_t denise_ui_paint(DeniseUi *ui, const DeniseFrame *frame);

/* Lists what denise_ui_paint drew. Returns how many there are, which may
 * exceed cap; the first cap are written. Pass NULL and 0 to ask the count. */
DENISE_API intptr_t denise_ui_damage(DeniseUi *ui, DeniseRect *out, size_t cap);

/* Retires this frame's damage. Call after the blit, not before: what it
 * forgets is exactly what a failed present would need to draw again. */
DENISE_API int32_t denise_ui_presented(DeniseUi *ui);

/* Marks the whole surface for repaint — resized, uncovered, or restored from
 * a state Denise cannot see. */
DENISE_API int32_t denise_ui_invalidate_all(DeniseUi *ui);

/* Marks one node for repaint. Rarely needed: everything that changes widget
 * state through this ABI invalidates on the way in. */
DENISE_API int32_t denise_ui_invalidate(DeniseUi *ui, uint64_t node);

#ifdef __cplusplus
}
#endif

#endif /* DENISE_H */
