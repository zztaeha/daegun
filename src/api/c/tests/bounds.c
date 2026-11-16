/* The counts the headers give -fbounds-safety: each view a call returns, a struct's view, a lent image, a string, a
 * visitor's stops and a segmenter's arrays, indexed by its count. With an argument it indexes past a run's glyphs,
 * which must trap. */
#include <ptrcheck.h>
#include <stdio.h>
#include <stdlib.h>
#include "daegun.h"

#define INTER "assets/test-fonts/inter/InterVariable.ttf"
#define COLR "assets/test-fonts/colr-v1-test-glyphs/test_glyphs.ttf"

static unsigned long read_items;

/* NULL for a handle a call requires, as rule 2 lets a caller pass, out of the compiler's sight. */
static void *__single volatile hidden_null;
#define NONE ((void *)hidden_null)

#define VIEW(type, call, ...)                       \
    do {                                            \
        size_t count = 0;                           \
        const type *view = call(__VA_ARGS__, &count); \
        if (count > 1) {                            \
            type second = view[1];                  \
            (void)second;                           \
            read_items += count;                    \
        }                                           \
    } while (0)

static daegun_font *open_font(const char *path)
{
    FILE *f = fopen(path, "rb");
    if (!f) exit(2);
    fseek(f, 0, SEEK_END);
    size_t len = (size_t)ftell(f);
    rewind(f);
    uint8_t *data = malloc(len);
    if (!data || fread(data, 1, len, f) != len) exit(2);
    fclose(f);
    daegun_font *__single font = NULL;
    if (daegun_font_open(data, len, &font) != DAEGUN_OK) exit(2);
    free(data);
    return font;
}

static void enter(void *user, const daegun_paint_node *node, const daegun_color_stop *DAEGUN_LENT(stops_count) stops,
                  size_t stops_count)
{
    (void)node;
    for (size_t i = 0; i < stops_count; i++) *(double *)user += stops[i].offset;
}

static size_t segment(void *user, DAEGUN_IN(len) const char *text, size_t len, DAEGUN_OUT(cap) size_t *out, size_t cap)
{
    (void)user;
    if (len != 6 || (unsigned char)text[len - 1] != 0x82 || cap == 0) return 0;
    out[cap - 1] = 3;
    return 1;
}

/* Every view a handle lends: those without a handle made here answer no count for NULL, as rule 2 has it. */
static void views_build(void)
{
    VIEW(uint8_t, daegun_blob_data, NONE);
    VIEW(daegun_scene_op, daegun_scene_ops, NONE);
    VIEW(daegun_clip_shape, daegun_scene_shapes, NONE);
    VIEW(daegun_gradient, daegun_scene_gradients, NONE);
    VIEW(daegun_stop, daegun_scene_stops, NONE);
    VIEW(daegun_paint_node, daegun_paint_graph_nodes, NONE);
    VIEW(daegun_color_stop, daegun_paint_graph_stops, NONE);
    VIEW(daegun_coord, daegun_named_instances_coords, NONE, 0);
    VIEW(daegun_coord, daegun_location_coords, NONE);
    VIEW(double, daegun_location_normalized, NONE);
    VIEW(daegun_math_variant, daegun_math_construction_variants, NONE);
    VIEW(daegun_stat_axis, daegun_stat_axes, NONE);
    VIEW(daegun_axis_value, daegun_stat_combo_values, NONE);
    VIEW(uint8_t, daegun_path_verbs, NONE);
    VIEW(daegun_point, daegun_path_points, NONE);
    VIEW(daegun_point, daegun_contours_points, NONE, 0);
    VIEW(daegun_quad, daegun_quads_data, NONE);
    VIEW(daegun_point, daegun_hinted_outline_points, NONE);
    VIEW(uint8_t, daegun_hinted_outline_kinds, NONE);
    VIEW(size_t, daegun_hinted_outline_contours, NONE);
    VIEW(daegun_cff_stem, daegun_cff_hints_stems, NONE);
    VIEW(float, daegun_subpixel_layout_weights, NONE, 0);
    VIEW(daegun_region_axis, daegun_ivs_region, NONE, 0);
    VIEW(size_t, daegun_ivs_ivd_region_indices, NONE, 0);
    VIEW(int32_t, daegun_ivs_ivd_row, NONE, 0, 0);
    VIEW(uint32_t, daegun_shaped_run_glyphs, NONE);
    VIEW(double, daegun_shaped_run_advances, NONE);
    VIEW(daegun_offset, daegun_shaped_run_offsets, NONE);
    VIEW(uint32_t, daegun_shaped_run_clusters, NONE);
    VIEW(uint8_t, daegun_shaped_run_unsafe_to_break, NONE);
    VIEW(uint8_t, daegun_shaped_run_unsafe_to_concat, NONE);
    VIEW(uint8_t, daegun_shaped_run_safe_to_insert_tatweel, NONE);
    VIEW(uint8_t, daegun_subset_bytes, NONE);
    VIEW(daegun_baseline, daegun_base_script_baselines, NONE);
    VIEW(uint8_t, daegun_bidi_paragraph_levels, NONE);
    VIEW(size_t, daegun_bidi_paragraph_visual_order, NONE);
    VIEW(daegun_paragraph_run, daegun_bidi_paragraph_paragraphs, NONE);
}

int main(int argc, char **argv)
{
    (void)argv;
    views_build();

    daegun_font *__single inter = open_font(INTER);
    daegun_shaped_run *__single run = NULL;
    const char *text = "abc";
    if (daegun_font_shape(inter, text, 3, NULL, NULL, &run) != DAEGUN_OK) return 3;
    size_t count = 0;
    const uint32_t *glyphs = daegun_shaped_run_glyphs(run, &count);
    if (count != 3) return 3;
    if (argc > 1) return (int)glyphs[count];
    uint32_t last = glyphs[count - 1];
    const double *advances = daegun_shaped_run_advances(run, &count);
    double width = advances[count - 1];

    daegun_str error = daegun_last_error();
    if (error.data != NULL && error.len > 0 && fwrite(error.data, 1, 0, stdout) != 0) return 3;
    daegun_str version = daegun_version();
    if (version.len < 2 || version.data[1] != '.') return 3;

    daegun_font *__single colr = open_font(COLR);
    double offsets = 0;
    daegun_paint_visitor visitor = { enter, NULL, &offsets };
    for (uint32_t glyph = 0; glyph < 200; glyph++) (void)daegun_font_visit_paint_graph(colr, glyph, NULL, 0, &visitor);
    if (offsets == 0) return 4;

    size_t breaks = 0;
    daegun_line_break found[16];
    const char *thai = "\xe0\xb8\x81\xe0\xb8\x82";
    daegun_status status = daegun_line_break_opportunities(thai, 6, segment, NULL, found, 16, &breaks);
    if (status != DAEGUN_OK || breaks != 2 || found[0].at != 3) return 5;

    daegun_rasterizer *__single raster = daegun_rasterizer_new();
    daegun_image image;
    if (daegun_rasterizer_glyph(raster, inter, last, NULL, 16.0f, NULL, &image) != DAEGUN_OK || image.pixels_len == 0)
        return 6;
    unsigned corner = image.pixels[image.pixels_len - 1];
    daegun_rasterizer_free(raster);

    daegun_shaped_run_free(run);
    daegun_font_free(colr);
    daegun_font_free(inter);
    printf("bounds: read %lu items, last glyph %u, width %.0f, corner %u\n", read_items, last, width, corner);
    return 0;
}
