/* clock_gettime and CLOCK_MONOTONIC are POSIX, which glibc hides under -std=c11 without this. */
#define _POSIX_C_SOURCE 199309L

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "daegun.h"

#define WARMUP 50
#define ROUNDS 200
#define BATCH  500

static int cmp_double(const void *a, const void *b)
{
    double x = *(const double *)a, y = *(const double *)b;
    return (x > y) - (x < y);
}

static double now_ns(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec * 1e9 + (double)ts.tv_nsec;
}

static void report(const char *name, double *samples, size_t n, size_t per_sample)
{
    qsort(samples, n, sizeof *samples, cmp_double);
    printf("  %-26s %9.1f ns   median %9.1f ns\n",
           name, samples[0] / (double)per_sample, samples[n / 2] / (double)per_sample);
}

static size_t pen_hits;
static void p_move(void *u, float x, float y) { (void)u; (void)x; (void)y; pen_hits++; }
static void p_line(void *u, float x, float y) { (void)u; (void)x; (void)y; pen_hits++; }
static void p_quad(void *u, float a, float b, float c, float d)
{ (void)u; (void)a; (void)b; (void)c; (void)d; pen_hits++; }
static void p_curve(void *u, float a, float b, float c, float d, float e, float f)
{ (void)u; (void)a; (void)b; (void)c; (void)d; (void)e; (void)f; pen_hits++; }
static void p_close(void *u) { (void)u; pen_hits++; }

static volatile uint64_t sink;

int main(int argc, char **argv)
{
    const char *path = argc > 1 ? argv[1] : "assets/test-fonts/inter/InterVariable.ttf";
    FILE *fp = fopen(path, "rb");
    if (!fp) { fprintf(stderr, "cannot read %s\n", path); return 1; }
    fseek(fp, 0, SEEK_END);
    long len = ftell(fp);
    fseek(fp, 0, SEEK_SET);
    uint8_t *bytes = malloc((size_t)len);
    if (!bytes || fread(bytes, 1, (size_t)len, fp) != (size_t)len) {
        fprintf(stderr, "short read of %s\n", path); return 1;
    }
    fclose(fp);

    daegun_font *font = NULL;
    if (daegun_font_open(bytes, (size_t)len, &font) != DAEGUN_OK) {
        fprintf(stderr, "open failed: %s\n", daegun_last_error().data); return 1;
    }
    uint16_t gid = 0;
    daegun_font_glyph_id(font, (uint32_t)'g', &gid);

    static double s[ROUNDS];
    printf("\ndaegun – C ABI, %d rounds after %d warmup\n\n", ROUNDS, WARMUP);

    for (int r = 0; r < WARMUP; r++) {
        daegun_font *f = NULL;
        daegun_font_open(bytes, (size_t)len, &f);
        daegun_font_free(f);
    }
    for (int r = 0; r < ROUNDS; r++) {
        double t = now_ns();
        daegun_font *f = NULL;
        daegun_font_open(bytes, (size_t)len, &f);
        s[r] = (now_ns() - t) * BATCH;
        daegun_font_free(f);
    }
    report("font_open", s, ROUNDS, BATCH);

    for (int r = 0; r < ROUNDS; r++) {
        uint8_t *buf = daegun_font_buffer_new((size_t)len);
        memcpy(buf, bytes, (size_t)len);
        daegun_font *f = NULL;
        double t = now_ns();
        daegun_font_open_owned(buf, (size_t)len, &f);
        s[r] = (now_ns() - t) * BATCH;
        daegun_font_free(f);
    }
    report("font_open_owned", s, ROUNDS, BATCH);

    /* A buffer taken and given back without opening anything must not leak. */
    daegun_font_buffer_free(daegun_font_buffer_new(1024), 1024);
    daegun_font_buffer_free(NULL, 0);

    #define BATCHED(label, body)                                                                   \
        do {                                                                                       \
            for (int r = 0; r < WARMUP; r++) { for (int i = 0; i < BATCH; i++) { body; } }          \
            for (int r = 0; r < ROUNDS; r++) {                                                     \
                double t = now_ns();                                                               \
                for (int i = 0; i < BATCH; i++) { body; }                                          \
                s[r] = now_ns() - t;                                                               \
            }                                                                                      \
            report(label, s, ROUNDS, BATCH);                                                       \
        } while (0)

    uint16_t upm = 0;
    BATCHED("upm", { daegun_font_upm(font, &upm); sink += upm; });

    uint16_t g = 0;
    BATCHED("glyph_id", { daegun_font_glyph_id(font, (uint32_t)'g', &g); sink += g; });

    BATCHED("advance_widths x1", {
        daegun_f64_list *adv = NULL;
        daegun_font_advance_widths(font, &gid, 1, NULL, 0, &adv);
        size_t n = 0;
        const double *d = daegun_f64_list_data(adv, &n);
        sink += (uint64_t)(d && n ? d[0] : 0.0);
        daegun_f64_list_free(adv);
    });

    daegun_pen pen = { p_move, p_line, p_quad, p_curve, p_close, NULL };
    BATCHED("outline_glyph", { daegun_font_outline_glyph(font, gid, &pen); });
    sink += pen_hits;

    BATCHED("glyph_quads", {
        daegun_quads *q = NULL;
        daegun_font_glyph_quads(font, gid, NULL, 0, &q);
        daegun_quads_free(q);
    });

    BATCHED("prepared_outline", {
        daegun_font_prepared_outline(font, gid, 16.0f, NULL, 0, NULL, &pen, NULL);
    });
    sink += pen_hits;

    daegun_path *glyph = daegun_path_new();
    daegun_pen into;
    daegun_path_as_pen(glyph, &into);
    daegun_font_outline_glyph(font, gid, &into);
    float max_area = 0.0f;
    daegun_flatten_max_area_for(16.0f, (float)upm, &max_area);
    BATCHED("flatten", {
        daegun_contours *c = NULL;
        daegun_path_flatten(glyph, max_area, &c);
        daegun_contours_free(c);
    });
    daegun_path_free(glyph);

    printf("\n");
    daegun_font_free(font);
    free(bytes);
    return 0;
}
