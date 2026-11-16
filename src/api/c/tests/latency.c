/* The C ABI's latency, row for row as api/latency.rs times it; perf/gate.sh builds it and holds it to that twin.
 *   cargo rustc --release --features capi --crate-type staticlib
 *   cc -std=c11 -O2 -I src/api/c/include src/api/c/tests/latency.c target/release/libdaegun.a -o latency
 * The defines open clock_gettime under -std=c11 and, as the first hides it, macOS's nanosecond clock. */
#define _POSIX_C_SOURCE 199309L
#define _DARWIN_C_SOURCE

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#ifdef _WIN32
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#endif

#include "daegun.h"

#define WARMUP 50
#define ROUNDS 200
#define BATCH  500
/* Copies of the bytes an open reads, each for ROUNDS / PLACES rounds in turn. */
#define PLACES 20
/* The clocks and memory take about 100 ms to come up after the core idles, longer than 50 batches of a short call. */
#define SETTLE_NS 300e6
/* Opens a sample, as one alone reads in whole ticks of the 41.67 ns clock. */
#define GROUP 8

static int cmp_double(const void *a, const void *b)
{
    double x = *(const double *)a, y = *(const double *)b;
    return (x > y) - (x < y);
}

/* A monotonic clock in nanoseconds: macOS's CLOCK_MONOTONIC counts whole microseconds, and Windows has none in C. */
static double now_ns(void)
{
#if defined(_WIN32)
    static LARGE_INTEGER freq;
    LARGE_INTEGER t;
    if (freq.QuadPart == 0)
        QueryPerformanceFrequency(&freq);
    QueryPerformanceCounter(&t);
    /* Split, as the count times 1e9 is past what a double holds exactly. */
    return (double)(t.QuadPart / freq.QuadPart) * 1e9
         + (double)(t.QuadPart % freq.QuadPart) * 1e9 / (double)freq.QuadPart;
#elif defined(__APPLE__)
    return (double)clock_gettime_nsec_np(CLOCK_UPTIME_RAW);
#else
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec * 1e9 + (double)ts.tv_nsec;
#endif
}

static void report(const char *name, double *samples, size_t per_sample)
{
    qsort(samples, ROUNDS, sizeof *samples, cmp_double);
    printf("  %-26s %9.1f ns   median %9.1f ns\n",
           name, samples[0] / (double)per_sample, samples[ROUNDS / 2] / (double)per_sample);
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

/* A call that fails would time as a fast one, so every status counts here and any failure fails the run. */
static size_t failed;

/* Each row's allocations a call, counted on macOS through the hook its allocator calls on every allocation, Rust's
 * and the caller's alike, then cleared. */
#if defined(__APPLE__)
typedef void(malloc_logger_t)(uint32_t, uintptr_t, uintptr_t, uintptr_t, uintptr_t, uint32_t);
extern malloc_logger_t *malloc_logger;
static uint64_t allocations;
static void count_allocation(uint32_t type, uintptr_t a, uintptr_t b, uintptr_t c, uintptr_t result, uint32_t skip)
{
    (void)a; (void)b; (void)c; (void)result; (void)skip;
    allocations += (type & 2) != 0;
}
#define COUNTED(label, n, body)                                                                    \
    do {                                                                                           \
        uint64_t from = allocations;                                                               \
        malloc_logger = count_allocation;                                                          \
        for (int i = 0; i < (n); i++) { body; }                                                    \
        malloc_logger = NULL;                                                                      \
        printf("  %-26s %9.2f allocations\n", label, (double)(allocations - from) / (double)(n));  \
    } while (0)
#else
#define COUNTED(label, n, body) do { } while (0)
#endif

int main(int argc, char **argv)
{
    const char *path = argc > 1 ? argv[1] : "assets/test-fonts/inter/InterVariable.ttf";
    FILE *fp = fopen(path, "rb");
    if (!fp) { fprintf(stderr, "cannot read %s\n", path); return 1; }
    fseek(fp, 0, SEEK_END);
    long len = ftell(fp);
    fseek(fp, 0, SEEK_SET);
    uint8_t *bytes = len > 0 ? malloc((size_t)len) : NULL;
    if (!bytes || fread(bytes, 1, (size_t)len, fp) != (size_t)len) {
        fprintf(stderr, "short read of %s\n", path); return 1;
    }
    fclose(fp);

    daegun_font *font = NULL;
    uint32_t gid = 0;
    uint16_t upm = 0;
    if (daegun_font_open(bytes, (size_t)len, &font) != DAEGUN_OK
        || daegun_font_glyph_id(font, (uint32_t)'g', &gid) != DAEGUN_OK
        || daegun_font_units_per_em(font, &upm) != DAEGUN_OK) {
        fprintf(stderr, "%s has no g: %s\n", path, daegun_last_error().data); return 1;
    }
    /* Inter's weight axis; a font without one is placed at its default, which times the same calls. */
    const daegun_coord bold_at = { DAEGUN_TAG('w', 'g', 'h', 't'), 700.0 };
    daegun_location *bold = NULL;
    if (daegun_font_location(font, &bold_at, 1, &bold) != DAEGUN_OK) {
        fprintf(stderr, "no location: %s\n", daegun_last_error().data); return 1;
    }

    static double s[ROUNDS];
    printf("\ndaegun C ABI, %d rounds after %d warmup and %.0f ms\n\n", ROUNDS, WARMUP, SETTLE_NS / 1e6);

    #define WARM(body)                                                                              \
        do {                                                                                        \
            double from = now_ns();                                                                 \
            for (int r = 0; r < WARMUP || now_ns() - from < SETTLE_NS; r++) { body; }               \
        } while (0)

    /* Where an open's bytes lie in memory moves its best by as much as 16%, and one copy fixes that for a whole
     * run: the best over 20 copies holds within 1% from run to run. */
    uint8_t *copies[PLACES];
    for (int i = 0; i < PLACES; i++) {
        copies[i] = malloc((size_t)len);
        if (!copies[i]) { fprintf(stderr, "no copy of %ld bytes\n", len); return 1; }
        memcpy(copies[i], bytes, (size_t)len);
    }
    WARM({
        daegun_font *f = NULL;
        failed += daegun_font_open(copies[0], (size_t)len, &f) != DAEGUN_OK;
        daegun_font_free(f);
    });
    /* The font before kept until the next is open, as the twin keeps it. */
    daegun_font *held = NULL;
    for (int r = 0; r < ROUNDS; r++) {
        double t = now_ns();
        daegun_font *f = NULL;
        failed += daegun_font_open(copies[r / (ROUNDS / PLACES)], (size_t)len, &f) != DAEGUN_OK;
        s[r] = now_ns() - t;
        daegun_font_free(held);
        held = f;
    }
    daegun_font_free(held);
    for (int i = 0; i < PLACES; i++) free(copies[i]);
    report("font_open", s, 1);
    COUNTED("font_open", GROUP, {
        daegun_font *f = NULL;
        failed += daegun_font_open(bytes, (size_t)len, &f) != DAEGUN_OK;
        daegun_font_free(f);
    });

    /* Each group's buffers copied just before it opens, as the twin's are, so long-evicted bytes time no misses. */
    WARM({
        uint8_t *buf = daegun_font_buffer_new((size_t)len);
        daegun_font *f = NULL;
        if (buf) {
            memcpy(buf, bytes, (size_t)len);
            failed += daegun_font_open_owned(buf, (size_t)len, 0, &f) != DAEGUN_OK;
        }
        daegun_font_free(f);
    });
    for (int r = 0; r < ROUNDS; r++) {
        uint8_t *bufs[GROUP];
        daegun_font *made[GROUP] = { NULL };
        for (int i = 0; i < GROUP; i++) {
            bufs[i] = daegun_font_buffer_new((size_t)len);
            if (!bufs[i]) { fprintf(stderr, "no buffer of %ld bytes\n", len); return 1; }
            memcpy(bufs[i], bytes, (size_t)len);
        }
        double t = now_ns();
        for (int i = 0; i < GROUP; i++) {
            failed += daegun_font_open_owned(bufs[i], (size_t)len, 0, &made[i]) != DAEGUN_OK;
        }
        s[r] = (now_ns() - t) / GROUP;
        for (int i = 0; i < GROUP; i++) {
            daegun_font_free(made[i]);
        }
    }
    report("font_open_owned", s, 1);
    COUNTED("font_open_owned", GROUP, {
        uint8_t *buf = daegun_font_buffer_new((size_t)len);
        daegun_font *f = NULL;
        if (buf) {
            memcpy(buf, bytes, (size_t)len);
            failed += daegun_font_open_owned(buf, (size_t)len, 0, &f) != DAEGUN_OK;
        }
        daegun_font_free(f);
    });

    /* A buffer taken and given back without opening anything must not leak. */
    daegun_font_buffer_free(daegun_font_buffer_new(1024), 1024);
    daegun_font_buffer_free(NULL, 0);

    #define BATCHED(label, body)                                                                   \
        do {                                                                                       \
            WARM({ for (int i = 0; i < BATCH; i++) { body; } });                                   \
            for (int r = 0; r < ROUNDS; r++) {                                                     \
                double t = now_ns();                                                               \
                for (int i = 0; i < BATCH; i++) { body; }                                          \
                s[r] = now_ns() - t;                                                               \
            }                                                                                      \
            report(label, s, BATCH);                                                               \
            COUNTED(label, BATCH, body);                                                           \
        } while (0)

    uint16_t u = 0;
    BATCHED("units_per_em", { failed += daegun_font_units_per_em(font, &u) != DAEGUN_OK; sink += u; });

    uint32_t g = 0;
    BATCHED("glyph_id", { failed += daegun_font_glyph_id(font, (uint32_t)'g', &g) != DAEGUN_OK; sink += g; });

    double adv = 0.0;
    BATCHED("advance", {
        failed += daegun_font_advance(font, gid, NULL, DAEGUN_HORIZONTAL, &adv) != DAEGUN_OK;
        sink += (uint64_t)adv;
    });
    BATCHED("advance at wght 700", {
        failed += daegun_font_advance(font, gid, bold, DAEGUN_HORIZONTAL, &adv) != DAEGUN_OK;
        sink += (uint64_t)adv;
    });

    daegun_pen pen = { p_move, p_line, p_quad, p_curve, p_close, NULL };
    BATCHED("outline", { failed += daegun_font_outline(font, gid, NULL, &pen) != DAEGUN_OK; });
    sink += pen_hits;

    double m = 0.0;
    BATCHED("ascender", { failed += daegun_font_ascender(font, NULL, &m) != DAEGUN_OK; sink += (uint64_t)m; });
    BATCHED("descender", { failed += daegun_font_descender(font, NULL, &m) != DAEGUN_OK; sink += (uint64_t)-m; });
    BATCHED("cap_height", { failed += daegun_font_cap_height(font, NULL, &m) != DAEGUN_OK; sink += (uint64_t)m; });
    uint32_t n = 0;
    BATCHED("glyph_count", { failed += daegun_font_glyph_count(font, &n) != DAEGUN_OK; sink += n; });
    daegun_line_metrics lm;
    BATCHED("line_metrics", {
        failed += daegun_font_line_metrics(font, NULL, DAEGUN_HORIZONTAL, &lm) != DAEGUN_OK;
        sink += (uint64_t)lm.ascent;
    });

    BATCHED("quads", {
        daegun_quads *q = NULL;
        failed += daegun_font_quads(font, gid, NULL, &q) != DAEGUN_OK;
        daegun_quads_free(q);
    });

    BATCHED("prepared_outline", {
        failed += daegun_font_prepared_outline(font, gid, NULL, 16.0f, NULL, &pen, NULL) != DAEGUN_OK;
    });
    sink += pen_hits;

    daegun_path *glyph = daegun_path_new();
    daegun_pen into;
    if (!glyph || daegun_path_as_pen(glyph, &into) != DAEGUN_OK
        || daegun_font_outline(font, gid, NULL, &into) != DAEGUN_OK) {
        fprintf(stderr, "no path of the g: %s\n", daegun_last_error().data); return 1;
    }
    float max_area = daegun_max_area_for(16.0f, (float)upm);
    BATCHED("flatten", {
        daegun_contours *c = NULL;
        failed += daegun_path_flatten(glyph, max_area, &c) != DAEGUN_OK;
        daegun_contours_free(c);
    });
    daegun_path_free(glyph);

    daegun_rasterizer *rasterizer = daegun_rasterizer_new();
    daegun_image image;
    BATCHED("raster_glyph", {
        failed += daegun_rasterizer_glyph(rasterizer, font, gid, NULL, 16.0f, NULL, &image) != DAEGUN_OK;
        sink += image.pixels_len;
    });
    daegun_rasterizer_free(rasterizer);

    printf("\n");
    daegun_location_free(bold);
    daegun_font_free(font);
    free(bytes);
    if (failed) {
        fprintf(stderr, "%zu timed calls failed, so their rows time a failure\n", failed);
        return 1;
    }
    return 0;
}
