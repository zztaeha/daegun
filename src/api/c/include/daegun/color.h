/* Color glyphs: bitmaps, COLR layers and palettes, paint graphs and the scenes lowered from them, and the color
 * math to sample a scene's gradients and composite its layers. Rust's color module and Font's color calls. */

#ifndef DAEGUN_COLOR_H
#define DAEGUN_COLOR_H

#include <daegun/base.h>
#include <daegun/outline.h>

#ifdef __cplusplus
extern "C" {
#endif

DAEGUN_HEADER_BEGIN

/** An sRGB color with straight, not premultiplied, alpha: 0 transparent to 255 opaque. */
typedef struct {
    uint8_t r;
    uint8_t g;
    uint8_t b;
    uint8_t a;
} daegun_rgba;

/* bitmaps */

typedef struct daegun_bitmap daegun_bitmap;

/** A glyph's bitmap from sbix, CBDT, EBDT or bdat, from the strike nearest `ppem`, at or above it first.
 * DAEGUN_ABSENT for a glyph no strike holds. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_bitmap(const daegun_font *font, uint32_t gid, uint16_t ppem,
                   daegun_bitmap *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
DAEGUN_API void DAEGUN_CALL
daegun_bitmap_free(daegun_bitmap *DAEGUN_NULLABLE bitmap);

/* A PNG as the font stores it, as daegun doesn't decode images, or a monochrome or grayscale strike's image
 * decoded, a byte of coverage a pixel, 0 to 255, rows top to bottom. */
#define DAEGUN_BITMAP_PNG       0
#define DAEGUN_BITMAP_COVERAGE  1

/** A bitmap: its DAEGUN_BITMAP_* format; the strike's size in pixels per em, which may not be the size asked
 * for; the image's left and top edges in pixels at that size from the glyph's origin, y up; with coverage,
 * its width and height in pixels; whether to draw it mirrored left to right, as Apple's sbix 'flip' asks;
 * and its bytes, BORROWED until the bitmap is freed. */
typedef struct {
    int32_t        format;
    uint16_t       ppem;
    int16_t        left;
    int16_t        top;
    uint16_t       width;
    uint16_t       height;
    bool           mirrored;
    const uint8_t *DAEGUN_NULLABLE data DAEGUN_COUNTED_BY(data_len);
    size_t         data_len;
} daegun_bitmap_info;

DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_bitmap_info_of(const daegun_bitmap *bitmap, daegun_bitmap_info *out);

/* COLR version 0 and palettes */

/* Every call taking a palette takes it by index, and one past the font's reads palette 0, the default. */

/** One layer, drawn back to front: a glyph's outline filled with the palette's color, or with the text's
 * color where `foreground` is set, `color` then black at the alpha the text's color is drawn at. */
typedef struct {
    uint32_t    glyph;
    daegun_rgba color;
    bool        foreground;
} daegun_colr_layer;

/** A COLR version 0 glyph's layers in a palette, into your buffer (BUFFERS). DAEGUN_ABSENT for a glyph with
 * none. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_WRITES(4, 5) daegun_status DAEGUN_CALL
daegun_font_colr_layers(const daegun_font *font, uint32_t gid, uint16_t palette,
                        DAEGUN_OUT(out_len) daegun_colr_layer *DAEGUN_NULLABLE out, size_t out_len, size_t *out_count);

/** A CPAL palette: its index, whether the font marks it readable on a light and on a dark background, and
 * with has_name_id, its name's ID in the name table. */
typedef struct {
    uint16_t index;
    uint16_t name_id;
    bool     light_safe;
    bool     dark_safe;
    bool     has_name_id;
} daegun_palette;

/** The font's CPAL palettes into your buffer (BUFFERS); none for a font without CPAL or whose CPAL fails
 * sanitizing. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_WRITES(2, 3) daegun_status DAEGUN_CALL
daegun_font_palettes(const daegun_font *font, DAEGUN_OUT(out_len) daegun_palette *DAEGUN_NULLABLE out, size_t out_len,
                     size_t *out_count);
/** A palette's colors in entry order, as a picker shows them, into your buffer (BUFFERS): none for a font
 * without CPAL. A palette past the font's reads palette 0, the default, as drawing does. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_WRITES(3, 4) daegun_status DAEGUN_CALL
daegun_font_palette_colors(const daegun_font *font, uint16_t palette,
                           DAEGUN_OUT(out_len) daegun_rgba *DAEGUN_NULLABLE out, size_t out_len, size_t *out_count);
/** Each palette entry's name ID in the name table, so a picker can label the colors a palette sets, and at
 * the same index a byte, 0 for an entry the font leaves unnamed, into your buffers (BUFFERS), either NULL
 * to leave it out. None where it names none. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_palette_entry_labels(const daegun_font *font,
                                 DAEGUN_OUT_OR_NULL(out_len) uint16_t *DAEGUN_NULLABLE out_name_ids,
                                 DAEGUN_OUT_OR_NULL(out_len) uint8_t *DAEGUN_NULLABLE out_named, size_t out_len,
                                 size_t *out_count);

/* scenes */

/* How a fill covers: where the contours wind around a nonzero number of times, as glyph outlines are filled, or an
 * odd number. */
#define DAEGUN_FILL_NONZERO  0
#define DAEGUN_FILL_EVENODD  1

/* How a layer combines with what lies under it: the Porter-Duff operators, then the blend modes of W3C
 * Compositing and Blending, in COLR's composite-mode numbering. */
#define DAEGUN_COMPOSITE_CLEAR            0
#define DAEGUN_COMPOSITE_SRC              1
#define DAEGUN_COMPOSITE_DEST             2
#define DAEGUN_COMPOSITE_SRC_OVER         3
#define DAEGUN_COMPOSITE_DEST_OVER        4
#define DAEGUN_COMPOSITE_SRC_IN           5
#define DAEGUN_COMPOSITE_DEST_IN          6
#define DAEGUN_COMPOSITE_SRC_OUT          7
#define DAEGUN_COMPOSITE_DEST_OUT         8
#define DAEGUN_COMPOSITE_SRC_ATOP         9
#define DAEGUN_COMPOSITE_DEST_ATOP       10
#define DAEGUN_COMPOSITE_XOR             11
#define DAEGUN_COMPOSITE_PLUS            12
#define DAEGUN_COMPOSITE_SCREEN          13
#define DAEGUN_COMPOSITE_OVERLAY         14
#define DAEGUN_COMPOSITE_DARKEN          15
#define DAEGUN_COMPOSITE_LIGHTEN         16
#define DAEGUN_COMPOSITE_COLOR_DODGE     17
#define DAEGUN_COMPOSITE_COLOR_BURN      18
#define DAEGUN_COMPOSITE_HARD_LIGHT      19
#define DAEGUN_COMPOSITE_SOFT_LIGHT      20
#define DAEGUN_COMPOSITE_DIFFERENCE      21
#define DAEGUN_COMPOSITE_EXCLUSION       22
#define DAEGUN_COMPOSITE_MULTIPLY        23
#define DAEGUN_COMPOSITE_HSL_HUE         24
#define DAEGUN_COMPOSITE_HSL_SATURATION  25
#define DAEGUN_COMPOSITE_HSL_COLOR       26
#define DAEGUN_COMPOSITE_HSL_LUMINOSITY  27

/* How a gradient goes on past its first and last stops: the end stops' colors carry on, the stops repeat,
 * or they repeat every other time reversed. COLR's numbering. */
#define DAEGUN_EXTEND_PAD      0
#define DAEGUN_EXTEND_REPEAT   1
#define DAEGUN_EXTEND_REFLECT  2

/** What a color glyph draws with, as Rust's SceneOptions: its palette, and the text's color, which a layer
 * or paint asking for the foreground takes. Fill it with daegun_scene_options_default: zeroing it is not
 * the default, which is opaque black. */
typedef struct {
    size_t      size;        /**< sizeof(daegun_scene_options) as your header has it */
    daegun_rgba foreground;
    uint16_t    palette;
} daegun_scene_options;

/** Sets the defaults in `size` bytes of *out and its size; pass sizeof *out. DAEGUN_INVALID_ARGUMENT for a
 * size below 2.0's, and DAEGUN_UNSUPPORTED for one larger than this library knows, each writing nothing. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_scene_options_default(daegun_scene_options *out, size_t size);

/** A scene: ops to draw in order over the paths, clip shapes and gradients it holds. */
typedef struct daegun_scene daegun_scene;

/** A color glyph as a scene, from its COLR version 1 paint graph or else its version 0 layers. In font
 * units, y up, each path held once and placed by its op's transform, at most 128 levels of clips and layers
 * deep, a version 1 glyph inside its clip box. A NULL opts means the defaults. DAEGUN_ABSENT for a glyph with
 * no color description, and no ops for one that draws nothing; DAEGUN_LIMIT_EXCEEDED where its outlines
 * hold more than 16 points a byte of the font's glyf, CFF, CFF2 and VARC tables, and a million at least, each glyph
 * held once and its decoding, drawn or not, counted with them: a charstring's step, a point or glyph glyf decodes,
 * or a unit of VARC's work, a value it decodes as one point and a component it loads as 64. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_color_scene(const daegun_font *font, uint32_t gid, const daegun_location *DAEGUN_NULLABLE location,
                        const daegun_scene_options *DAEGUN_NULLABLE opts,
                        daegun_scene *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
DAEGUN_API void DAEGUN_CALL
daegun_scene_free(daegun_scene *DAEGUN_NULLABLE scene);

#define DAEGUN_OP_FILL        0  /* a path filled */
#define DAEGUN_OP_PUSH_CLIP   1  /* a clip to its shapes together, until the POP_CLIP that pairs with it */
#define DAEGUN_OP_POP_CLIP    2
#define DAEGUN_OP_PUSH_LAYER  3  /* a layer drawn on its own, until the POP_LAYER that composites it */
#define DAEGUN_OP_POP_LAYER   4

/** One op, each field read by the kinds named beside it. A transform is [a, b, c, d, e, f], x' = a*x + c*y + e
 * and y' = b*x + d*y + f. */
typedef struct {
    double      transform[6];   /**< FILL: where the path is placed */
    int32_t     kind;           /**< DAEGUN_OP_* */
    int32_t     rule;           /**< FILL: DAEGUN_FILL_* */
    int32_t     blend;          /**< PUSH_LAYER: DAEGUN_COMPOSITE_* */
    float       opacity;        /**< PUSH_LAYER: as it composites, 0 to 1 */
    size_t      path;           /**< FILL */
    size_t      gradient;       /**< FILL with has_gradient, else it fills with `color` */
    size_t      shapes_start;   /**< PUSH_CLIP: its shapes, start to end of the scene's */
    size_t      shapes_end;
    daegun_rgba color;          /**< FILL */
    bool        has_gradient;   /**< FILL */
} daegun_scene_op;

/** One shape of a clip: one of the scene's paths, filled by a DAEGUN_FILL_* rule and placed by a transform. */
typedef struct {
    double  transform[6];
    size_t  path;
    int32_t rule;
} daegun_clip_shape;

#define DAEGUN_GRADIENT_LINEAR  0  /* values: x0, y0, offset 0, to x1, y1, offset 1 */
#define DAEGUN_GRADIENT_RADIAL  1  /* values: x0, y0, r0, offset 0 on that circle, to x1, y1, r1 */
#define DAEGUN_GRADIENT_SWEEP   2  /* values: cx, cy, start_angle, end_angle, degrees counterclockwise */

/** One stop of a gradient: where along it the stop lies, 0 at its start and 1 at its end, and its color. */
typedef struct {
    float       offset;
    daegun_rgba color;
} daegun_stop;

/** A gradient: its shape in its own space, which `transform` takes to the scene's, how it extends, and its
 * stops, stops_count of the scene's from stops_start, sorted within 0 to 1, the geometry moved to a color
 * line's first and last stop, so a radius can fall below 0 and paint nothing. */
typedef struct {
    double   transform[6];
    float    values[6];     /**< as kind says */
    int32_t  kind;          /**< DAEGUN_GRADIENT_* */
    int32_t  extend;        /**< DAEGUN_EXTEND_* */
    size_t   stops_start;
    size_t   stops_count;
} daegun_gradient;

/** Borrowed views, valid until the scene is freed: the ops in drawing order, every clip shape, every
 * gradient and every gradient's stops. */
DAEGUN_API const daegun_scene_op *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_scene_ops(const daegun_scene *scene, size_t *out_count);
DAEGUN_API const daegun_clip_shape *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_scene_shapes(const daegun_scene *scene, size_t *out_count);
DAEGUN_API const daegun_gradient *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_scene_gradients(const daegun_scene *scene, size_t *out_count);
DAEGUN_API const daegun_stop *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_scene_stops(const daegun_scene *scene, size_t *out_count);

/** A path a scene holds, BORROWED until it is freed: its verbs, a DAEGUN_VERB_* byte each, and the points
 * they take. */
typedef struct {
    const uint8_t      *DAEGUN_NULLABLE verbs DAEGUN_COUNTED_BY(verbs_len);
    size_t              verbs_len;
    const daegun_point *DAEGUN_NULLABLE points DAEGUN_COUNTED_BY(points_len);
    size_t              points_len;
} daegun_path_view;

DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_scene_path_count(const daegun_scene *scene, size_t *out);
/** DAEGUN_INVALID_ARGUMENT past the last path, for this and the copy. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_scene_path_at(const daegun_scene *scene, size_t path, daegun_path_view *out);
/** A path of the scene copied into a path of your own, for the calls a daegun_path takes. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_scene_path_copy(const daegun_scene *scene, size_t path, daegun_path *DAEGUN_NULLABLE *DAEGUN_NONNULL out);

/** A COLR version 1 glyph's clip box in font units, which what it draws stays inside, so it sizes a surface
 * without walking the scene. At a location each edge moves by its delta, rounded to a whole unit.
 * DAEGUN_ABSENT for a glyph without one. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_colr_clip_box(const daegun_font *font, uint32_t gid, const daegun_location *DAEGUN_NULLABLE location,
                          daegun_bounds *out);

/* sampling and compositing */

/* How a gradient blends between its stops: in linear light with alpha premultiplied, as the OpenType spec
 * asks, or in the stored sRGB values with alpha straight, blending the numbers the palette holds. */
#define DAEGUN_INTERPOLATE_LINEAR_LIGHT  0
#define DAEGUN_INTERPOLATE_SRGB          1

/** What a gradient paints at each pixel, worked out once for its placement: build one for each gradient and
 * transform, not each pixel. */
typedef struct daegun_ramp daegun_ramp;

/** A scene's gradient made ready to sample, `to_device` taking the scene's space to your pixels after the
 * gradient's own transform. DAEGUN_INVALID_ARGUMENT past the last gradient. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_scene_ramp(const daegun_scene *scene, size_t gradient, const double to_device[DAEGUN_NONNULL 6],
                  int32_t interpolation, daegun_ramp *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
/** The color at pixel (x, y), sampled at its center. DAEGUN_ABSENT where the gradient paints nothing, as a
 * radial one does outside its cone. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_ramp_at(const daegun_ramp *ramp, double x, double y, daegun_rgba *out);
DAEGUN_API void DAEGUN_CALL
daegun_ramp_free(daegun_ramp *DAEGUN_NULLABLE ramp);

/** A mode's blend function B(Cb, Cs) of a backdrop and a source color, each red, green and blue, straight
 * and 0 to 1; the source itself for a Porter-Duff mode, which doesn't blend. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_blend(int32_t mode, const float backdrop[DAEGUN_NONNULL 3], const float source[DAEGUN_NONNULL 3],
             float out[DAEGUN_NONNULL 3]);
/** A source at source_alpha composited onto a backdrop at backdrop_alpha under a mode, the source blending
 * with the backdrop only where they overlap: Cs' = (1 - ab)*Cs + ab*B(Cb, Cs). Colors go in straight and
 * 0 to 1, past which the HSL modes can answer NaN, and come back premultiplied by the alpha in *out_alpha. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_composite(int32_t mode, const float source[DAEGUN_NONNULL 3], float source_alpha,
                 const float backdrop[DAEGUN_NONNULL 3], float backdrop_alpha, float out[DAEGUN_NONNULL 3],
                 float *out_alpha);

/* paint graphs */

/* A COLR version 1 glyph's paints, positions in font units, y up, and angles in degrees counterclockwise.
 * Each paint's `values` hold what its kind says, the rest 0; a paint that scales, rotates or skews keeps a
 * point fixed, `center` with has_center and else the origin. */
#define DAEGUN_PAINT_LAYERS            0  /* its children painted in turn, bottom to top */
#define DAEGUN_PAINT_GLYPH             1  /* its child painted inside `glyph`'s outline */
#define DAEGUN_PAINT_COLR_GLYPH        2  /* `glyph`'s own graph, which is its child */
#define DAEGUN_PAINT_SOLID             3  /* `color` over the whole clip, or the text's with `foreground` */
#define DAEGUN_PAINT_LINEAR_GRADIENT   4  /* x0, y0 to x1, y1, then x2, y2, which sets its rotation */
#define DAEGUN_PAINT_RADIAL_GRADIENT   5  /* x0, y0, r0 to x1, y1, r1 */
#define DAEGUN_PAINT_SWEEP_GRADIENT    6  /* cx, cy, start_angle, end_angle */
#define DAEGUN_PAINT_TRANSFORM         7  /* its child through [xx, yx, xy, yy, dx, dy] */
#define DAEGUN_PAINT_TRANSLATE         8  /* dx, dy */
#define DAEGUN_PAINT_SCALE             9  /* sx, sy */
#define DAEGUN_PAINT_SCALE_UNIFORM    10  /* s */
#define DAEGUN_PAINT_ROTATE           11  /* angle */
#define DAEGUN_PAINT_SKEW             12  /* x_angle, y_angle */
#define DAEGUN_PAINT_COMPOSITE        13  /* its first child, the source, onto its second by `mode` */

/** One stop of a color line: where it lies, 0 at the line's start, 1 at its end and past either beyond them;
 * the palette's color, its alpha scaled by the stop's; and whether it takes the text's color in its place,
 * `color` then black at that alpha. */
typedef struct {
    double      offset;
    daegun_rgba color;
    bool        foreground;
} daegun_color_stop;

/** One paint, each field read by the kinds named beside it. The graph lists its paints in the order a
 * visitor meets them, the root first, so a paint's children start at the index after its own, each next
 * child at the `end` of the one before, until the paint's own end. */
typedef struct {
    double      values[6];      /**< as kind says */
    double      center[2];      /**< with has_center */
    size_t      end;            /**< one past the last paint below it */
    size_t      stops_start;    /**< gradients: stops_count of the graph's stops */
    size_t      stops_count;
    uint32_t    glyph;          /**< GLYPH and COLR_GLYPH */
    int32_t     kind;           /**< DAEGUN_PAINT_* */
    int32_t     extend;         /**< gradients: DAEGUN_EXTEND_* */
    int32_t     mode;           /**< COMPOSITE: DAEGUN_COMPOSITE_* */
    daegun_rgba color;          /**< SOLID */
    bool        foreground;     /**< SOLID */
    bool        has_center;     /**< SCALE, SCALE_UNIFORM, ROTATE and SKEW */
} daegun_paint_node;

typedef struct daegun_paint_graph daegun_paint_graph;

/** A COLR version 1 glyph's paint graph at a location, in a palette. DAEGUN_ABSENT for a glyph with none. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_paint_graph(const daegun_font *font, uint32_t gid, const daegun_location *DAEGUN_NULLABLE location,
                        uint16_t palette, daegun_paint_graph *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
/** Borrowed views, valid until the graph is freed: its paints, the root first, and every gradient's stops in
 * the font's order. */
DAEGUN_API const daegun_paint_node *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_paint_graph_nodes(const daegun_paint_graph *graph, size_t *out_count);
DAEGUN_API const daegun_color_stop *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_paint_graph_stops(const daegun_paint_graph *graph, size_t *out_count);
DAEGUN_API void DAEGUN_CALL
daegun_paint_graph_free(daegun_paint_graph *DAEGUN_NULLABLE graph);

/** A graph walked with no graph built: each paint entered, with a gradient's stops and their count lent for the
 * call and `end` and stops_start 0, then its children in the graph's order, then left. Both callbacks may be NULL,
 * and each must return normally, as CALLBACKS in base.h says, and may call back into daegun, on the same
 * font too: no lock is held while one runs. */
typedef struct {
    void (DAEGUN_CALLBACK *DAEGUN_NULLABLE enter)(void *DAEGUN_NULLABLE user, const daegun_paint_node *node,
                                                  const daegun_color_stop *DAEGUN_NULLABLE stops
                                                  DAEGUN_LENT(stops_count), size_t stops_count);
    void (DAEGUN_CALLBACK *DAEGUN_NULLABLE leave)(void *DAEGUN_NULLABLE user);
    void *DAEGUN_NULLABLE user;
} daegun_paint_visitor;

/** daegun_font_paint_graph's graph handed to a visitor paint by paint. DAEGUN_ABSENT for a glyph with none. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_visit_paint_graph(const daegun_font *font, uint32_t gid, const daegun_location *DAEGUN_NULLABLE location,
                              uint16_t palette, const daegun_paint_visitor *visitor);

DAEGUN_LAYOUT_ASSERT(sizeof(daegun_rgba) == 4, "a color is four bytes");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_colr_layer) == 12, "a layer is a glyph, a color and a flag");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_palette) == 8, "a palette is two words and three flags");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_stop) == 8, "a stop is a float and a color");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_color_stop) == 16, "a color stop is a double, a color and a flag");
#ifdef DAEGUN_LAYOUT_64
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_scene_options) == 16, "scene options layout");
DAEGUN_LAYOUT_ASSERT(offsetof(daegun_scene_options, palette) == 12, "scene options layout");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_bitmap_info) == 32, "bitmap info layout");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_scene_op) == 104, "scene op layout");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_clip_shape) == 64, "clip shape layout");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_gradient) == 96, "gradient layout");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_path_view) == 32, "path view layout");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_paint_node) == 112, "paint node layout");
#endif

DAEGUN_HEADER_END

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* DAEGUN_COLOR_H */
