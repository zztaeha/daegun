/* The README's C examples, compiled and run as written: readme.rs checks each C block appears below line for line.
 *   cargo rustc --features capi --crate-type staticlib
 *   cc -std=c11 -I src/api/c/include src/api/c/tests/readme.c target/debug/libdaegun.a -o readme
 *   ./readme assets/test-fonts/inter/InterVariable.ttf */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "daegun.h"

#define CHECK(cond, what)                                                                                          \
    do {                                                                                                           \
        if (!(cond)) {                                                                                             \
            fprintf(stderr, "FAIL %s\n", what);                                                                    \
            return 1;                                                                                              \
        }                                                                                                          \
    } while (0)

/* A rasterizer that counts what it's drawn. */
typedef struct {
    size_t events;
} rasterizer;

static void on_move(void *r, float x, float y) { (void)x; (void)y; ((rasterizer *)r)->events++; }
static void on_line(void *r, float x, float y) { (void)x; (void)y; ((rasterizer *)r)->events++; }
static void on_quad(void *r, float cx, float cy, float x, float y)
{
    (void)cx; (void)cy; (void)x; (void)y;
    ((rasterizer *)r)->events++;
}
static void on_cubic(void *r, float c1x, float c1y, float c2x, float c2y, float x, float y)
{
    (void)c1x; (void)c1y; (void)c2x; (void)c2y; (void)x; (void)y;
    ((rasterizer *)r)->events++;
}
static void on_close(void *r) { ((rasterizer *)r)->events++; }

static int the_loop(const uint8_t *data, size_t len)
{
    rasterizer my_rasterizer = {0};
    {
#include "daegun.h"

daegun_font *font = NULL;
if (daegun_font_open(data, len, &font) != DAEGUN_OK) return 1;

daegun_shaped_run *run = NULL;
if (daegun_font_shape(font, "Wave", 4, NULL, NULL, &run) != DAEGUN_OK) return 1;

size_t n = 0;
const uint32_t *gids = daegun_shaped_run_glyphs(run, &n);
const double *advances = daegun_shaped_run_advances(run, &n); /* in font units */

/* your rasterizer's callbacks, and the state they draw into */
daegun_pen pen = { on_move, on_line, on_quad, on_cubic, on_close, &my_rasterizer };
daegun_prepared_glyph glyph;
if (daegun_font_prepared_outline(font, gids[0], NULL, 32.0f, NULL, &pen, &glyph) != DAEGUN_OK) return 1;
/* the callbacks got the outline in pixels, and glyph.advance_width is in pixels too */

daegun_shaped_run_free(run);
daegun_font_free(font);
        CHECK(n == 4 && advances, "Wave shaped to four glyphs");
        CHECK(my_rasterizer.events > 0 && glyph.advance_width > 0, "the first glyph drew, with its advance");
    }
    return 0;
}

static int a_location(daegun_font *font)
{
    daegun_shaped_run *run = NULL;
const daegun_coord coords[] = {
    { DAEGUN_TAG('w', 'g', 'h', 't'), 700.0 },
    { DAEGUN_TAG('o', 'p', 's', 'z'), 28.0 },
};
daegun_location *bold = NULL;
if (daegun_font_location(font, coords, 2, &bold) != DAEGUN_OK) return 1;
if (daegun_font_shape(font, "Wave", 4, bold, NULL, &run) != DAEGUN_OK) return 1;
daegun_location_free(bold);
    CHECK(run != NULL, "Wave shaped at wght 700");
    daegun_shaped_run_free(run);
    return 0;
}

static int options(daegun_font *font)
{
    daegun_shaped_run *run = NULL;
daegun_shape_options opts;
if (daegun_shape_options_default(&opts, sizeof opts) != DAEGUN_OK) return 1;
opts.language = "tr";
if (daegun_font_shape(font, "fit", 3, NULL, &opts, &run) != DAEGUN_OK) return 1;
    CHECK(run != NULL, "fit shaped in Turkish");
    daegun_shaped_run_free(run);
    return 0;
}

static int a_list(daegun_font *font)
{
size_t count = 0;
if (daegun_font_table_tags(font, NULL, 0, &count) != DAEGUN_OK) return 1;
daegun_tag *tags = malloc(count * sizeof *tags);
if (daegun_font_table_tags(font, tags, count, &count) != DAEGUN_OK) return 1;
    CHECK(count > 0, "the font names its tables");
    free(tags);
    return 0;
}

static int subsetting(daegun_font *font, FILE *out)
{
const char *text = "Type is the voice of the page.";
daegun_subset *subset = NULL;
if (daegun_font_subset_text(font, text, strlen(text), NULL, &subset) != DAEGUN_OK) return 1;

size_t ttf_len = 0;
const uint8_t *ttf = daegun_subset_bytes(subset, &ttf_len);
fwrite(ttf, 1, ttf_len, out);
daegun_subset_free(subset);
    CHECK(ttf_len > 0 && ftell(out) == (long)ttf_len, "the subset was written");
    return 0;
}

static int rasterizing(daegun_font *font)
{
daegun_shaped_run *run = NULL;
if (daegun_font_shape(font, "Wave", 4, NULL, NULL, &run) != DAEGUN_OK) return 1;
size_t n = 0;
const uint32_t *gids = daegun_shaped_run_glyphs(run, &n);
const double *advances = daegun_shaped_run_advances(run, &n);
const daegun_offset *offsets = daegun_shaped_run_offsets(run, &n);
uint16_t upem = 0;
if (daegun_font_units_per_em(font, &upem) != DAEGUN_OK) return 1;
const double scale = 32.0 / upem;
const int baseline = 30;
static uint8_t canvas[40][96];

daegun_rasterizer *rasterizer = daegun_rasterizer_new();
daegun_raster_options opts;
if (daegun_raster_options_default(&opts, sizeof opts) != DAEGUN_OK) return 1;
double pen = 2.0;
for (size_t i = 0; i < n; i++) {
    /* whole pixels place the image, and the fraction moves the glyph within it */
    double x = pen + offsets[i].x * scale, y = offsets[i].y * scale;
    opts.offset[0] = (float)(x - (int)x);
    opts.offset[1] = (float)(y - (int)y);
    daegun_image image;
    if (daegun_rasterizer_glyph(rasterizer, font, gids[i], NULL, 32.0f, &opts, &image) != DAEGUN_OK) return 1;
    int left = (int)x + image.left, top = baseline - (int)y - image.top;
    for (uint32_t row = 0; row < image.height; row++) {
        for (uint32_t col = 0; col < image.width; col++) {
            unsigned sum = canvas[top + (int)row][left + (int)col] + image.pixels[row * image.stride + col];
            canvas[top + (int)row][left + (int)col] = sum > 255 ? 255 : (uint8_t)sum;
        }
    }
    pen += advances[i] * scale;
}
daegun_rasterizer_free(rasterizer);
daegun_shaped_run_free(run);
    /* The ink's box, as Rust's example finds it. */
    int x0 = 96, x1 = -1, y0 = 40, y1 = -1;
    for (int y = 0; y < 40; y++) {
        for (int x = 0; x < 96; x++) {
            if (!canvas[y][x]) continue;
            if (x < x0) x0 = x;
            if (x > x1) x1 = x;
            if (y < y0) y0 = y;
            y1 = y;
        }
    }
    CHECK(x0 == 2 && x1 == 83 && y0 == 6 && y1 == 30, "Wave's ink isn't where Rust's example puts it");
    return 0;
}

int main(int argc, char **argv)
{
    const char *path = argc > 1 ? argv[1] : "assets/test-fonts/inter/InterVariable.ttf";
    FILE *fp = fopen(path, "rb");
    CHECK(fp, "the font reads");
    fseek(fp, 0, SEEK_END);
    long len = ftell(fp);
    fseek(fp, 0, SEEK_SET);
    uint8_t *data = len > 0 ? malloc((size_t)len) : NULL;
    CHECK(data && fread(data, 1, (size_t)len, fp) == (size_t)len, "the font reads whole");
    fclose(fp);

    daegun_font *font = NULL;
    FILE *out = tmpfile();
    CHECK(out && daegun_font_open(data, (size_t)len, &font) == DAEGUN_OK, "the font opens");
    int failed = the_loop(data, (size_t)len) || a_location(font) || options(font) || a_list(font)
                 || subsetting(font, out) || rasterizing(font);
    fclose(out);
    daegun_font_free(font);
    free(data);
    if (!failed) printf("the README's 6 C blocks ran\n");
    return failed;
}
