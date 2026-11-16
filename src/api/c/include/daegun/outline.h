/* Outlines: glyphs drawn in font units, hinted in pixels and prepared for a rasterizer, and the path tools around
 * them. Rust's outline module and Font's outline calls. */

#ifndef DAEGUN_OUTLINE_H
#define DAEGUN_OUTLINE_H

#include <daegun/base.h>

#ifdef __cplusplus
extern "C" {
#endif

DAEGUN_HEADER_BEGIN

/* pens and points */

/** What you hand daegun to receive an outline: five callbacks, each passed `user` untouched. A NULL callback
 * skips its event. Each must return normally, as CALLBACKS in base.h says, and may call back into daegun,
 * on the same font too: no lock is held while one runs. */
typedef struct {
    void (DAEGUN_CALLBACK *DAEGUN_NULLABLE move_to)(void *DAEGUN_NULLABLE user, float x, float y);
    void (DAEGUN_CALLBACK *DAEGUN_NULLABLE line_to)(void *DAEGUN_NULLABLE user, float x, float y);
    void (DAEGUN_CALLBACK *DAEGUN_NULLABLE quad_to)(void *DAEGUN_NULLABLE user, float cx, float cy, float x, float y);
    void (DAEGUN_CALLBACK *DAEGUN_NULLABLE curve_to)(void *DAEGUN_NULLABLE user, float c1x, float c1y, float c2x,
                                                     float c2y, float x, float y);
    void (DAEGUN_CALLBACK *DAEGUN_NULLABLE close)(void *DAEGUN_NULLABLE user);
    void *DAEGUN_NULLABLE user;
} daegun_pen;

/** A point of an outline, y up, in the units it was drawn in. */
typedef struct {
    float x;
    float y;
} daegun_point;

/* glyph outlines */

/** Draws a glyph's outline at a location onto a pen, in font units, y up, from its origin. A failure is the
 * decoder's, for a glyph that doesn't decode, by which time the pen may hold part of it. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_outline(const daegun_font *font, uint32_t gid, const daegun_location *DAEGUN_NULLABLE location,
                    const daegun_pen *pen);
/** Whether glyf flags the glyph as one whose contours or components may overlap: OVERLAP_SIMPLE on it or a
 * glyph it's built from, or OVERLAP_COMPOUND on a component, which a rasterizer may take as a cue to
 * antialias with more care. False for a glyph CFF, CFF2 or VARC draws, which carry no such flag, and for one
 * glyf leaves unflagged, whose contours may overlap all the same. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_has_overlap_flag(const daegun_font *font, uint32_t gid, bool *out);

/* paths */

/** An outline held as a value: built by the five calls a pen makes, and readable, replayable and strokeable
 * at any point. */
typedef struct daegun_path daegun_path;

DAEGUN_API void DAEGUN_CALL
daegun_path_free(daegun_path *DAEGUN_NULLABLE path);
/** An empty path. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_MALLOC(malloc(daegun_path_free, 1), returns_nonnull)
daegun_path * DAEGUN_CALL
daegun_path_new(void);

DAEGUN_API daegun_status DAEGUN_CALL
daegun_path_move_to(daegun_path *path, float x, float y);
DAEGUN_API daegun_status DAEGUN_CALL
daegun_path_line_to(daegun_path *path, float x, float y);
DAEGUN_API daegun_status DAEGUN_CALL
daegun_path_quad_to(daegun_path *path, float cx, float cy, float x, float y);
DAEGUN_API daegun_status DAEGUN_CALL
daegun_path_curve_to(daegun_path *path, float c1x, float c1y, float c2x, float c2y, float x, float y);
DAEGUN_API daegun_status DAEGUN_CALL
daegun_path_close(daegun_path *path);
/** Empties it, keeping its room, so a path drawn into again allocates only to grow. */
DAEGUN_API daegun_status DAEGUN_CALL
daegun_path_clear(daegun_path *path);
/** A pen that draws into the path, BORROWING it until it is freed, so an outline can be kept as a value. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_path_as_pen(daegun_path *path, daegun_pen *out);

/** Whether nothing was drawn into it. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_path_is_empty(const daegun_path *path, bool *out);
/** The bytes it holds: one a verb and eight a point. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_path_cost(const daegun_path *path, size_t *out);
/** The tight box it draws, its curves' extremes included. DAEGUN_ABSENT for a path with no points. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_path_bounds(const daegun_path *path, daegun_bounds *out);

/* What each verb takes from the points, in order. */
#define DAEGUN_VERB_MOVE   0  /* one point */
#define DAEGUN_VERB_LINE   1  /* one point */
#define DAEGUN_VERB_QUAD   2  /* two: a control, then the end */
#define DAEGUN_VERB_CUBIC  3  /* three: two controls, then the end */
#define DAEGUN_VERB_CLOSE  4  /* none */

/** Borrowed views, valid until the path is freed or drawn into: its verbs, a byte each, and in one list the
 * points they take. */
DAEGUN_API const uint8_t *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_path_verbs(const daegun_path *path, size_t *out_count);
DAEGUN_API const daegun_point *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_path_points(const daegun_path *path, size_t *out_count);

/** Draws it onto a pen through a 2x3 transform, six doubles [a, b, c, d, dx, dy], or NULL for none. Every
 * contour reaches the pen closed, as a fill reads it. The pen may draw into this path: it is replayed as it
 * was when the call began. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_path_replay(const daegun_path *path, const double *DAEGUN_NULLABLE transform, const daegun_pen *pen);

/* strokes */

#define DAEGUN_JOIN_MITER  0
#define DAEGUN_JOIN_ROUND  1
#define DAEGUN_JOIN_BEVEL  2

#define DAEGUN_CAP_BUTT    0
#define DAEGUN_CAP_ROUND   1
#define DAEGUN_CAP_SQUARE  2

/** How a path is stroked: its width in the path's units, its sign ignored and 0 drawing nothing, how it ends
 * open contours, how it turns corners, and for a miter how far the point may reach, in half widths,
 * beveled past it. A miter limit not above 0 means 4. Rust's default is a width of 1, butt caps and miters
 * reaching 4. */
typedef struct {
    float   width;
    int32_t cap;          /**< DAEGUN_CAP_* */
    int32_t join;         /**< DAEGUN_JOIN_* */
    float   miter_limit;  /**< read with DAEGUN_JOIN_MITER only */
} daegun_stroke_style;

/** Strokes a path onto a pen as filled contours, each point reaching the pen as it's worked out, so a stroke
 * of any size draws. `tolerance` is how far the flattened curves may sit from the true ones, in the path's
 * units; one not above zero, or NaN, means 0.1. Drawing nothing, DAEGUN_INVALID_ARGUMENT for a width not
 * finite, and DAEGUN_OVERFLOW for a point not finite, as a miter's tip overflows long before the width. The
 * pen may draw into this path, as daegun_path_replay's may. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_path_stroke(const daegun_path *path, const daegun_stroke_style *style, float tolerance,
                   const daegun_pen *pen);
/** The same, its self-intersections resolved into one boundary, for a filler that doesn't do nonzero
 * winding: a stroke overlaps itself at every join. Drawing nothing, as daegun_path_stroke fails, and
 * DAEGUN_LIMIT_EXCEEDED past DAEGUN_MAX_FLATTEN_POINTS points, as the stroke is held whole to be resolved. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_path_stroke_simplified(const daegun_path *path, const daegun_stroke_style *style, float tolerance,
                              const daegun_pen *pen);

/* flattening and overlaps */

/* The most points daegun_path_flatten keeps, and daegun_path_stroke_simplified holds to resolve. */
#define DAEGUN_MAX_FLATTEN_POINTS 1048576

/** The max_area daegun_path_flatten takes to draw a path in font units at `px` pixels to the em, as 1.1.7's
 * rasterizer flattened. A px so small the area overflows, or one below zero, asks for the coarsest. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_CONST float DAEGUN_CALL
daegun_max_area_for(float px, float units_per_em);

typedef struct daegun_contours daegun_contours;

/** A path as closed polygons, none under three points or repeating its first. A curve halves until twice
 * the area of its ends-and-midpoint triangle is within max_area, one not finite and above zero halving all
 * the way. DAEGUN_INVALID_ARGUMENT for a point not finite or past FLT_MAX / 16, and DAEGUN_LIMIT_EXCEEDED
 * past DAEGUN_MAX_FLATTEN_POINTS points kept or twice that read. Contours freed on a thread are its next
 * flatten's, their lists reused, so flattening glyph after glyph, freeing each, allocates only to grow. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_path_flatten(const daegun_path *path, float max_area, daegun_contours *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
/** Contours of your own: `count` of them, contour i the lengths[i] points that follow the ones before it in
 * `points`. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(2, 3) daegun_status DAEGUN_CALL
daegun_contours_new(const daegun_point *DAEGUN_NULLABLE points, DAEGUN_IN(count) const size_t *DAEGUN_NULLABLE lengths,
                    size_t count, daegun_contours *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_contours_count(const daegun_contours *contours, size_t *out);
/** One contour's points, BORROWED until the contours are freed; NULL past the last. */
DAEGUN_API const daegun_point *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_contours_points(const daegun_contours *contours, size_t index, size_t *out_count);
DAEGUN_API void DAEGUN_CALL
daegun_contours_free(daegun_contours *DAEGUN_NULLABLE contours);

/** One counterclockwise boundary, whatever the input's winding, in place of contours that overlap, for a
 * rasterizer that sums coverage. DAEGUN_ABSENT means keep the contours: they don't overlap, where they meet
 * they're too close to call, touching without crossing or running together along a curve, or they meet more
 * often than their number of curves allows, as no font does. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_contours_resolve_overlaps(const daegun_contours *contours, daegun_contours *DAEGUN_NULLABLE *DAEGUN_NONNULL out);

/* quadratic curves */

/** A quadratic curve: its start, its control and its end. */
typedef struct {
    daegun_point start;
    daegun_point control;
    daegun_point end;
} daegun_quad;

typedef struct daegun_quads daegun_quads;

/** A glyph's outline at a location as quadratic curves in font units, as a curve-evaluating GPU shader takes
 * them: a line becomes a curve with its control at the midpoint, and a cubic splits to within 1/4096 em,
 * or into 256 at most, past which f32 can't place its points any closer. Wound as
 * daegun_quads_normalize_winding winds them. DAEGUN_NO_OUTLINE for a glyph that draws nothing, such as a
 * space, DAEGUN_OVERFLOW for a point not finite or past FLT_MAX / 16, and else the decoder's. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_quads(const daegun_font *font, uint32_t gid, const daegun_location *DAEGUN_NULLABLE location,
                  daegun_quads *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
/** A path as quadratic curves in its own units, in the order it was drawn and not rewound, an open contour
 * closed, a cubic split to within `tolerance`. DAEGUN_INVALID_ARGUMENT for a tolerance not finite and
 * above zero, and else as daegun_font_quads. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_path_quads(const daegun_path *path, float tolerance, daegun_quads *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
/** Rewinds them in place so their total signed area isn't positive: clockwise, TrueType's convention. A
 * rewound curve keeps its place in the list, so the curves need not chain. */
DAEGUN_API daegun_status DAEGUN_CALL
daegun_quads_normalize_winding(daegun_quads *quads);
/** The curves, BORROWED until the quads are freed or rewound. */
DAEGUN_API const daegun_quad *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_quads_data(const daegun_quads *quads, size_t *out_count);
DAEGUN_API void DAEGUN_CALL
daegun_quads_free(daegun_quads *DAEGUN_NULLABLE quads);

/* hinting */

/* Which hinter fits a glyph to the pixel grid: none; the font's own hints, its TrueType bytecode (moving x
 * only in a font declaring native ClearType) or its CFF or CFF2 hints; one chosen once for the font, the
 * autohinter for TrueType outlines with no font program and a CVT program of at most 7 bytes, else the
 * font's own bytecode or CFF hints, a glyph they don't hint left unhinted; or the autohinter for all. */
#define DAEGUN_HINT_NONE        0
#define DAEGUN_HINT_FONT        1
#define DAEGUN_HINT_AUTO        3
#define DAEGUN_HINT_AUTO_FORCE  4

/** Whether the autohinter may take a glyph under a mode: two of the four let it, so it is no comparison
 * against one constant. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_hint_mode_may_autohint(int32_t mode, bool *out);

typedef struct daegun_hinted_outline daegun_hinted_outline;

/** A glyph hinted at `px` pixels to the em at a location, in pixels from its origin, y up. DAEGUN_ABSENT
 * where no hinter takes it: DAEGUN_HINT_NONE, a px past 65,535, a glyph too large to hint at that size, or
 * one hinting leaves where scaling puts it, as one its font's own hints leave alone. Draw it unhinted then.
 * DAEGUN_INVALID_ARGUMENT unless px is finite and above zero, as daegun_font_prepared_outline answers. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_hinted_outline(const daegun_font *font, uint32_t gid, const daegun_location *DAEGUN_NULLABLE location,
                           float px, int32_t mode, daegun_hinted_outline *DAEGUN_NULLABLE *DAEGUN_NONNULL out);

/* What a hinted point is: on the curve, or a control of a curve of one kind. */
#define DAEGUN_POINT_ON_CURVE   0
#define DAEGUN_POINT_QUADRATIC  1  /* a quadratic curve's control, as TrueType's are */
#define DAEGUN_POINT_CUBIC      2  /* one of a cubic curve's two controls, as CFF's are */

/** Borrowed views, valid until the outline is freed: its points, each point's DAEGUN_POINT_* at the same index,
 * a byte each, and where each contour ends, as the index of its last point. */
DAEGUN_API const daegun_point *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_hinted_outline_points(const daegun_hinted_outline *outline, size_t *out_count);
DAEGUN_API const uint8_t *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_hinted_outline_kinds(const daegun_hinted_outline *outline, size_t *out_count);
DAEGUN_API const size_t *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_hinted_outline_contours(const daegun_hinted_outline *outline, size_t *out_count);
/** Draws it onto a pen, in pixels. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_hinted_outline_draw(const daegun_hinted_outline *outline, const daegun_pen *pen);
DAEGUN_API void DAEGUN_CALL
daegun_hinted_outline_free(daegun_hinted_outline *DAEGUN_NULLABLE outline);

/** A CFF stem hint, its edges as the charstring states them in font units, and whether it hints a vertical
 * stem, its edges then x positions. An edge hint keeps its width of -20 or -21, so its max is below its
 * min. */
typedef struct {
    float min;
    float max;
    bool  vertical;
} daegun_cff_stem;

/** A hint mask the charstring sets as it draws: the point it takes effect from, counted as the glyph's
 * points are drawn, and bits_len bytes of a bit a stem, in the stems' order, high bit first, BORROWED until
 * the hints are freed. */
typedef struct {
    size_t         point;
    const uint8_t *DAEGUN_NULLABLE bits DAEGUN_COUNTED_BY(bits_len);
    size_t         bits_len;
} daegun_cff_mask;

typedef struct daegun_cff_hints daegun_cff_hints;

/** A CFF glyph's stem hints and masks at a location, a CFF2 font's blended there as its outline is.
 * DAEGUN_ABSENT for a glyph that isn't CFF or doesn't decode. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_cff_hints(const daegun_font *font, uint32_t gid, const daegun_location *DAEGUN_NULLABLE location,
                      daegun_cff_hints *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
/** The stems in the charstring's order, BORROWED until the hints are freed. */
DAEGUN_API const daegun_cff_stem *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_cff_hints_stems(const daegun_cff_hints *hints, size_t *out_count);
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_cff_hints_mask_count(const daegun_cff_hints *hints, size_t *out);
/** The masks in the order the charstring sets them. DAEGUN_INVALID_ARGUMENT past the last. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_cff_hints_mask_at(const daegun_cff_hints *hints, size_t index, daegun_cff_mask *out);
DAEGUN_API void DAEGUN_CALL
daegun_cff_hints_free(daegun_cff_hints *DAEGUN_NULLABLE hints);

/* prepared outlines */

/** What daegun_font_prepared_outline does to an outline, in font units, as Rust's OutlineOptions: hinting,
 * then the oblique and the transform, then a stroke or an embolden. Either overlaps itself, which a
 * rasterizer summing coverage needs daegun_contours_resolve_overlaps for. Fill it with
 * daegun_outline_options_default, which gives zero to all but its size, as every field's default is. */
typedef struct {
    size_t   size;          /**< sizeof(daegun_outline_options) as your header has it */
    int32_t  hinting;       /**< DAEGUN_HINT_* */
    /** A 2x3 transform [a, b, c, d, dx, dy], with has_transform, its offsets in font units, applied after
     * hinting. It scales neither a stroke, an embolden nor the advances. */
    float    transform[6];
    daegun_stroke_style stroke;  /**< with has_stroke, in place of the fill, its width in font units */
    /** Extra stem width in font units, with has_embolden, which widens the advances too. Ignored with a
     * stroke, which takes its place, and at zero or below. */
    float    embolden;
    /** The tangent of a lean from upright, with has_oblique, positive to the right, applied before the
     * transform so the glyph leans in its own frame. */
    float    oblique;
    bool     has_transform;
    bool     has_stroke;
    bool     has_embolden;
    bool     has_oblique;
} daegun_outline_options;

/** Zeroes `size` bytes of *out and sets its size; pass sizeof *out. DAEGUN_INVALID_ARGUMENT for a size
 * below 2.0's, and DAEGUN_UNSUPPORTED for one larger than this library knows, each writing nothing. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_outline_options_default(daegun_outline_options *out, size_t size);

/** What daegun_font_prepared_outline drew: the advances in pixels, an embolden widening both, the one down
 * 0 for a font without vertical metrics, and whether a hinter ran. */
typedef struct {
    float advance_width;
    float advance_height;
    bool  hinted;
} daegun_prepared_glyph;

/** A glyph's outline at a location for a rasterizer of your own: hinted, leaned, transformed, stroked or
 * emboldened as the options say, then onto the pen in pixels at `px` to the em, y up, from the glyph's
 * origin. A NULL opts means the defaults, and out may be NULL. Drawing nothing, DAEGUN_INVALID_ARGUMENT
 * unless px is finite and above zero and the options finite, DAEGUN_OVERFLOW for anything past FLT_MAX once
 * scaled, and else the decoder's. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_prepared_outline(const daegun_font *font, uint32_t gid, const daegun_location *DAEGUN_NULLABLE location,
                             float px, const daegun_outline_options *DAEGUN_NULLABLE opts, const daegun_pen *pen,
                             daegun_prepared_glyph *DAEGUN_NULLABLE out);

DAEGUN_LAYOUT_ASSERT(sizeof(daegun_pen) == 6 * sizeof(void *), "a pen is five callbacks and a pointer");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_point) == 8, "a point is two floats");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_quad) == 24, "a quad is three points");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_stroke_style) == 16, "a stroke style is two floats and two codes");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_cff_stem) == 12, "a stem is two floats and a flag");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_prepared_glyph) == 12, "a prepared glyph is two floats and a flag");
#ifdef DAEGUN_LAYOUT_64
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_outline_options) == 64, "outline options end without padding");
DAEGUN_LAYOUT_ASSERT(offsetof(daegun_outline_options, transform) == 12, "outline options layout");
DAEGUN_LAYOUT_ASSERT(offsetof(daegun_outline_options, stroke) == 36, "outline options layout");
DAEGUN_LAYOUT_ASSERT(offsetof(daegun_outline_options, has_transform) == 60, "outline options layout");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_cff_mask) == 24, "a mask is a size_t, a pointer and a size_t");
#endif

DAEGUN_HEADER_END

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* DAEGUN_OUTLINE_H */
