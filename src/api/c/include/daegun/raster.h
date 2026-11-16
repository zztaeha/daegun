/* daegun's rasterizer, and what any rasterizer needs beyond outlines: subpixel layouts and the filters they sample
 * through, and an atlas packer. Rust's raster module. */

#ifndef DAEGUN_RASTER_H
#define DAEGUN_RASTER_H

#include <daegun/base.h>
#include <daegun/color.h>

#ifdef __cplusplus
extern "C" {
#endif

DAEGUN_HEADER_BEGIN

/* subpixel layouts */

/** How a glyph rasterized at several samples a pixel filters down to one value a channel: plain coverage, a
 * display's stripes, or a filter of your own. A channel is its weights over a window of taps_x by taps_y
 * samples, starting `origin` samples from the pixel's first. */
typedef struct daegun_subpixel_layout daegun_subpixel_layout;

/* The most samples a layout takes a pixel, the most taps its filter has, each way, and the most weights a
 * channel of a filter has, the taps squared. */
#define DAEGUN_MAX_OVERSAMPLE  4
#define DAEGUN_MAX_TAPS        8
#define DAEGUN_MAX_WEIGHTS    64

/* The order of a display's subpixel stripes: red, green, blue, left to right or top to bottom, or blue,
 * green, red. */
#define DAEGUN_STRIPES_RGB  0
#define DAEGUN_STRIPES_BGR  1

/** One sample a pixel and one channel: plain coverage. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_subpixel_layout_grayscale(daegun_subpixel_layout *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
/** Three samples across a pixel, for stripes side by side, through a five-tap filter that softens color
 * fringes. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_subpixel_layout_horizontal(int32_t order, daegun_subpixel_layout *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
/** Three samples down a pixel, for stripes stacked, through the same filter. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_subpixel_layout_vertical(int32_t order, daegun_subpixel_layout *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
/** Three samples a pixel, across for stripes side by side (DAEGUN_HORIZONTAL) or down for stacked ones, each
 * channel its stripe's sample alone. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_subpixel_layout_unfiltered(int32_t order, int32_t orientation,
                                  daegun_subpixel_layout *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
/** A filter of your own: taps_x * taps_y weights for each of three channels, one channel after another, each
 * row by row, over oversample samples a pixel each way. DAEGUN_INVALID_ARGUMENT for an oversample of 0 or
 * past DAEGUN_MAX_OVERSAMPLE, taps of 0 or past DAEGUN_MAX_TAPS, weights_len other than three channels'
 * worth, or a weight that isn't finite. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(7, 8) daegun_status DAEGUN_CALL
daegun_subpixel_layout_from_weights(uint8_t oversample_x, uint8_t oversample_y, uint8_t taps_x, uint8_t taps_y,
                                    int8_t origin_x, int8_t origin_y,
                                    DAEGUN_IN(weights_len) const float *DAEGUN_NULLABLE weights, size_t weights_len,
                                    daegun_subpixel_layout *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
DAEGUN_API void DAEGUN_CALL
daegun_subpixel_layout_free(daegun_subpixel_layout *DAEGUN_NULLABLE layout);

/** Whether it has one channel. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_subpixel_layout_is_grayscale(const daegun_subpixel_layout *layout, bool *out);
/** 1 for grayscale, 3 for a color layout. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_subpixel_layout_channels(const daegun_subpixel_layout *layout, uint8_t *out);
/** Samples a pixel, each way. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_subpixel_layout_oversample(const daegun_subpixel_layout *layout, uint8_t *out_x, uint8_t *out_y);
/** The filter's taps, each way. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_subpixel_layout_taps(const daegun_subpixel_layout *layout, uint8_t *out_x, uint8_t *out_y);
/** Where the filter's window starts, in samples from the pixel's first, each way. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_subpixel_layout_origin(const daegun_subpixel_layout *layout, int8_t *out_x, int8_t *out_y);
/** The whole pixels a bitmap needs on each side of the glyph for the filter's window, each way. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_subpixel_layout_pad(const daegun_subpixel_layout *layout, size_t *out_x, size_t *out_y);
/** A channel's taps_x * taps_y weights, row by row, BORROWED until the layout is freed; NULL past its
 * channels. */
DAEGUN_API const float *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_subpixel_layout_weights(const daegun_subpixel_layout *layout, size_t channel, size_t *out_count);
/** Its identity, for a cache key: a 64-bit hash of the parameters as given, so two filters share a key only by
 * a collision, and one written another way (an extra zero weight, a -0.0) gets a key of its own. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_subpixel_layout_key(const daegun_subpixel_layout *layout, uint64_t *out);

/* atlases */

/** Where a glyph landed in an atlas, in pixels: its left and top edges, its width and its height. */
typedef struct {
    size_t x;
    size_t y;
    size_t w;
    size_t h;
} daegun_rect;

/** A shelf packer for an atlas of one size: glyphs go left to right on shelves, each as tall as the glyph that
 * opened it, a glyph on the shortest shelf it fits. */
typedef struct daegun_shelf_packer daegun_shelf_packer;

DAEGUN_API void DAEGUN_CALL
daegun_shelf_packer_free(daegun_shelf_packer *DAEGUN_NULLABLE packer);
/** An empty atlas of this size, in pixels. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_MALLOC(malloc(daegun_shelf_packer_free, 1), returns_nonnull)
daegun_shelf_packer * DAEGUN_CALL
daegun_shelf_packer_new(size_t width, size_t height);
/** Room for a glyph this size. DAEGUN_ABSENT for one empty either way or larger than the atlas, and where it's
 * full: an answer, so start another atlas. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_shelf_packer_insert(daegun_shelf_packer *packer, size_t w, size_t h, daegun_rect *out);
/** Empties the atlas, keeping its size. */
DAEGUN_API daegun_status DAEGUN_CALL
daegun_shelf_packer_reset(daegun_shelf_packer *packer);

/* rasterizing */

/** Draws glyphs and paths into images it lends, keeping every buffer between calls, so once it has drawn its
 * largest image it allocates nothing. Each call takes it whole: give each thread its own. */
typedef struct daegun_rasterizer daegun_rasterizer;

/* The most pixels an image a rasterizer lends is across or down; a draw into a daegun_target takes any size. */
#define DAEGUN_MAX_SIDE 16384

/* The most bytes a draw whose image a rasterizer lends holds in its buffers, the image's among them; past it the draw
 * is refused, as an outline a few pixels to the em can still reach DAEGUN_MAX_SIDE, where it would take gigabytes. A
 * draw into a daegun_target holds the caller's pixels and works in bands of rows of a bounded size. */
#define DAEGUN_MAX_DRAW_BYTES 268435456

/* How coverage becomes alpha: as it is, exact, for a compositor that blends in linear light; or the alpha a
 * compositor blending sRGB values as they are needs, for text of the options' color. */
#define DAEGUN_GAMMA_LINEAR  0
#define DAEGUN_GAMMA_SRGB    1

/* A distance field's kind: one channel, the distance; or three whose median is the distance, each measuring to
 * its own edges, so corners stay sharp when it's scaled up. */
#define DAEGUN_FIELD_SDF   0
#define DAEGUN_FIELD_MSDF  1

/** Pixels a rasterizer lent, BORROWED until the next call on that rasterizer or its free: `height` rows `stride`
 * bytes apart, each pixel `channels` bytes (1 for coverage, 3 for a subpixel layout's channels, 4 for
 * premultiplied sRGB RGBA), its left edge `left` pixels right of the glyph's origin and its top edge `top`
 * pixels above it, y up. */
typedef struct {
    int32_t        left;
    int32_t        top;
    uint32_t       width;
    uint32_t       height;
    size_t         stride;
    const uint8_t *DAEGUN_NULLABLE pixels DAEGUN_COUNTED_BY(pixels_len);
    size_t         pixels_len;
    uint8_t        channels;
} daegun_image;

/** Pixels a caller holds for a draw to fill, as Rust's Target, every one written: `height` rows `stride` bytes apart,
 * `width` pixels each, its left edge `left` pixels right of the glyph's origin and its top edge `top` pixels above it,
 * y up, as an image lies. */
typedef struct {
    int32_t   left;
    int32_t   top;
    uint32_t  width;
    uint32_t  height;
    size_t    stride;
    uint8_t  *DAEGUN_NULLABLE pixels DAEGUN_COUNTED_BY(pixels_len);
    size_t    pixels_len;
} daegun_target;

/** How daegun_rasterizer_glyph draws, as Rust's RasterOptions. Fill it with daegun_raster_options_default, which
 * gives zero to all but its size, as every field's default is: the outline as the font draws it, at the pixel's
 * corner, as plain coverage, its stems as they are and its coverage exact. */
typedef struct {
    size_t   size;          /**< sizeof(daegun_raster_options) as your header has it */
    /** Hinting, a transform, a stroke, an embolden or a lean, as daegun_font_prepared_outline takes them; NULL
     * for none. */
    const daegun_outline_options *DAEGUN_NULLABLE outline;
    /** A display's stripes, which coverage filters through into three bytes a pixel; NULL for plain coverage. */
    const daegun_subpixel_layout *DAEGUN_NULLABLE layout;
    /** How far right and up the glyph moves before it's drawn, in pixels: a pen position's fraction, so text set
     * between pixels keeps its spacing. */
    float    offset[2];
    int32_t  gamma;         /**< DAEGUN_GAMMA_* */
    daegun_rgba text;       /**< with DAEGUN_GAMMA_SRGB, the text's color, its alpha unread */
    /** With DAEGUN_GAMMA_SRGB, how much thicker coverage is made, 0 for none to 1, the more as the backdrop is
     * lighter. */
    float    contrast;
    /** Whether thin stems widen, as text drawn small reads too light otherwise: by the documented CFF curve on the
     * font's stem in pixels, none from 2 1/3 pixels on, and none with a stroke. */
    bool     darken;
} daegun_raster_options;

DAEGUN_API void DAEGUN_CALL
daegun_rasterizer_free(daegun_rasterizer *DAEGUN_NULLABLE rasterizer);
/** A rasterizer with nothing kept yet. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_MALLOC(malloc(daegun_rasterizer_free, 1), returns_nonnull)
daegun_rasterizer * DAEGUN_CALL
daegun_rasterizer_new(void);
/** Sets the bytes the outlines it learned may hold: each glyph's outline in font units and what its overlaps are,
 * learned at its first draw so later draws at any size skip both. Past it the glyphs least recently drawn go,
 * whichever font's; 4 MB by default, and at 0 each draw learns its glyph again. */
DAEGUN_API daegun_status DAEGUN_CALL
daegun_rasterizer_set_cache_budget(daegun_rasterizer *rasterizer, size_t bytes);

/** Zeroes `size` bytes of *out and sets its size; pass sizeof *out. DAEGUN_INVALID_ARGUMENT for a size
 * below 2.0's, and DAEGUN_UNSUPPORTED for one larger than this library knows, each writing nothing. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_raster_options_default(daegun_raster_options *out, size_t size);

/** A glyph at a location, `px` pixels to the em, as the options say; NULL options mean the defaults. Drawing
 * nothing, DAEGUN_INVALID_ARGUMENT for a code not above, for what daegun_font_prepared_outline refuses or for a point
 * not finite once placed, DAEGUN_LIMIT_EXCEEDED for an image past DAEGUN_MAX_SIDE or DAEGUN_MAX_DRAW_BYTES or an
 * outline whose overlaps would take more than 2^27 passes over a pixel, and 512 more a point and 8 a pixel, to
 * resolve, and else the outline's. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_rasterizer_glyph(daegun_rasterizer *rasterizer, const daegun_font *font, uint32_t gid,
                        const daegun_location *DAEGUN_NULLABLE location, float px,
                        const daegun_raster_options *DAEGUN_NULLABLE opts, daegun_image *out);

/** A glyph drawn as daegun_rasterizer_glyph draws it with no layout, into the target's pixels, one byte of coverage
 * each, however large the target: the outline is placed and cut to the target once, then worked out a band of rows
 * at a time, each within a bounded size, so no image is held. DAEGUN_INVALID_ARGUMENT for options with a layout or
 * pixels that don't hold the target's rows, and else as daegun_rasterizer_glyph but for an image's size, the pixels
 * then left part written. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_rasterizer_glyph_into(daegun_rasterizer *rasterizer, const daegun_font *font, uint32_t gid,
                             const daegun_location *DAEGUN_NULLABLE location, float px,
                             const daegun_raster_options *DAEGUN_NULLABLE opts, const daegun_target *target);

/** A color glyph's COLR scene, as daegun_font_color_scene gives it in the scene options' palette and foreground,
 * composited in linear light into premultiplied sRGB RGBA, four bytes a pixel, inside the glyph's clip box where
 * it has one. It takes the options' transform, lean and offset; NULL options or scene options mean the defaults.
 * DAEGUN_ABSENT for a glyph without a scene; drawing nothing, DAEGUN_INVALID_ARGUMENT for options with a layout,
 * a stroke, an embolden or hinting, DAEGUN_LIMIT_EXCEEDED for a scene whose work would pass over each pixel it's
 * worked out over more than 1,024 times and 2^29 besides, a fill over its box, a clip shape over its box and the
 * frame and a layer twice over the frame, their points, cells and overlaps counted with them, and else as
 * daegun_rasterizer_glyph. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_rasterizer_color_glyph(daegun_rasterizer *rasterizer, const daegun_font *font, uint32_t gid,
                              const daegun_location *DAEGUN_NULLABLE location, float px,
                              const daegun_raster_options *DAEGUN_NULLABLE opts,
                              const daegun_scene_options *DAEGUN_NULLABLE scene, daegun_image *out);

/** A color glyph's scene drawn as daegun_rasterizer_color_glyph draws it, into the target's pixels, four bytes of
 * premultiplied sRGB RGBA each, transparent where it paints nothing, however large the target: the scene is worked out
 * a band of the target's rows at a time, each within a bounded size. DAEGUN_ABSENT for a glyph without a scene, the
 * pixels left as they were; DAEGUN_INVALID_ARGUMENT for pixels that don't hold the target's rows, and else as
 * daegun_rasterizer_color_glyph but for an image's size, the pixels then left part written. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_rasterizer_color_glyph_into(daegun_rasterizer *rasterizer, const daegun_font *font, uint32_t gid,
                                   const daegun_location *DAEGUN_NULLABLE location, float px,
                                   const daegun_raster_options *DAEGUN_NULLABLE opts,
                                   const daegun_scene_options *DAEGUN_NULLABLE scene, const daegun_target *target);

/** A path filled by a DAEGUN_FILL_* rule, after a 2x3 transform, six doubles [a, b, c, d, dx, dy], or NULL for
 * none, in pixels, y up. Drawing nothing, DAEGUN_INVALID_ARGUMENT for a rule not above or a point not finite once
 * transformed, and DAEGUN_LIMIT_EXCEEDED for an image past DAEGUN_MAX_SIDE or DAEGUN_MAX_DRAW_BYTES or a path
 * whose overlaps would take more than 2^27 passes over a pixel, and 512 more a point and 8 a pixel, to resolve. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_rasterizer_path(daegun_rasterizer *rasterizer, const daegun_path *path, int32_t rule,
                       const double *DAEGUN_NULLABLE transform, daegun_image *out);

/** A path filled as daegun_rasterizer_path fills it, into the target's pixels, one byte of coverage each, however
 * large the target: the path is placed and cut to the target once, then worked out a band of rows at a time, each
 * within a bounded size. DAEGUN_INVALID_ARGUMENT for a rule not above, pixels that don't hold the target's rows or a
 * point not finite once transformed, and DAEGUN_LIMIT_EXCEEDED for a path whose overlaps would take more than 2^27
 * passes over a pixel, and 512 more a point and 8 a pixel, to resolve, the pixels then left part written. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_rasterizer_path_into(daegun_rasterizer *rasterizer, const daegun_path *path, int32_t rule,
                            const double *DAEGUN_NULLABLE transform, const daegun_target *target);

/** A glyph's signed distance field at a location, `px` pixels to the em, its outline as the font draws it at the
 * origin: each pixel the distance from its center to the outline, inside positive, as the level
 * 127.5 + 127.5 * d / spread rounded, so it reaches `spread` pixels each way. A DAEGUN_FIELD_SDF field has one
 * channel, a DAEGUN_FIELD_MSDF field three. Drawing nothing, DAEGUN_INVALID_ARGUMENT for a kind not above or a
 * spread or size not finite and above 0, DAEGUN_LIMIT_EXCEEDED for an outline whose chords cross or meet each other
 * more than 64 times a chord, and else as daegun_rasterizer_glyph. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_rasterizer_distance_field(daegun_rasterizer *rasterizer, const daegun_font *font, uint32_t gid,
                                 const daegun_location *DAEGUN_NULLABLE location, float px, int32_t field,
                                 float spread, daegun_image *out);

DAEGUN_LAYOUT_ASSERT(sizeof(daegun_rect) == 4 * sizeof(size_t), "a rect is four size_t");
#ifdef DAEGUN_LAYOUT_64
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_image) == 48, "an image's fields, then its channels and padding");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_target) == 40, "a target's fields");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_raster_options) == 48, "raster options end in padding");
DAEGUN_LAYOUT_ASSERT(offsetof(daegun_raster_options, offset) == 24, "raster options layout");
DAEGUN_LAYOUT_ASSERT(offsetof(daegun_raster_options, darken) == 44, "raster options layout");
#endif

DAEGUN_HEADER_END

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* DAEGUN_RASTER_H */
