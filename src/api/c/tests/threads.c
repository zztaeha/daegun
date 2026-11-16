/* Two fonts of one file, each with a location, shared by threads as rule 5 allows, every cache of both drawing from one
 * pool, glyphs drawn through each thread's rasterizer with the raster caches evicting, each answer held to the first
 * thread's. A ninth thread sets, reads and clears every cache and the pool's budget, moves each cache out of the pool
 * and back and prewarms meanwhile, each thread lets go of what it keeps every fourth round, and a second wave repeats
 * it all. The gate runs it under TSan with the library instrumented, so a race in what a font caches fails it. */
#define _POSIX_C_SOURCE 200809L

#include <pthread.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "daegun.h"

#define THREADS 8
#define ROUNDS 24
#define WAVES 2

static const char TEXT[] = "Typography is the craft of endowing human language with a durable visual form.";

static daegun_font *fonts[2];
static daegun_location *bolds[2];
static daegun_cache_pool *pool;
static atomic_int reading;

typedef struct {
    double width;
    size_t glyphs;
    size_t lines;
    double block;
    double drawn;
    size_t points;
    size_t instance;
    double ascent;
    uint64_t raster;
} answers;

typedef struct {
    answers at[2];
    daegun_shaped_run *kept[ROUNDS];
    daegun_rasterizer *rasterizer;
    int font;
    int failed;
} work;

static void move_to(void *user, float x, float y) { *(double *)user += (double)x + (double)y; }
static void line_to(void *user, float x, float y) { *(double *)user += (double)x - (double)y; }
static void quad_to(void *user, float cx, float cy, float x, float y)
{
    *(double *)user += (double)cx * 2 + (double)cy + (double)x + (double)y;
}
static void curve_to(void *user, float c1x, float c1y, float c2x, float c2y, float x, float y)
{
    *(double *)user += (double)c1x + (double)c1y * 3 + (double)c2x + (double)c2y + (double)x + (double)y;
}
static void close_path(void *user) { *(double *)user += 1; }

static int same(const answers *a, const answers *b)
{
    return a->width == b->width && a->glyphs == b->glyphs && a->lines == b->lines && a->block == b->block
        && a->drawn == b->drawn && a->points == b->points && a->instance == b->instance && a->ascent == b->ascent
        && a->raster == b->raster;
}

/* Everything one round asks of a font at one location: NULL for the default. */
static int ask(const daegun_font *font, const daegun_location *at, answers *out, daegun_shaped_run **keep,
               daegun_rasterizer *r)
{
    daegun_shaped_run *run = NULL;
    if (daegun_font_shape(font, TEXT, sizeof TEXT - 1, at, NULL, &run) != DAEGUN_OK
        || daegun_shaped_run_width(run, &out->width) != DAEGUN_OK) {
        daegun_shaped_run_free(run);
        return 0;
    }
    daegun_shaped_run_glyphs(run, &out->glyphs);
    *keep = run;

    daegun_layout_options opts;
    daegun_layout *layout = NULL;
    daegun_layout_info info;
    if (daegun_layout_options_default(&opts, sizeof opts) != DAEGUN_OK) {
        return 0;
    }
    opts.max_inline_size = 12000.0;
    int ok = daegun_font_layout(font, TEXT, sizeof TEXT - 1, at, &opts, &layout) == DAEGUN_OK
          && daegun_layout_info_of(layout, &info) == DAEGUN_OK;
    daegun_layout_free(layout);
    if (!ok) {
        return 0;
    }
    out->lines = info.line_count;
    out->block = info.block_size;

    out->drawn = 0;
    out->raster = 0;
    daegun_pen pen = { move_to, line_to, quad_to, curve_to, close_path, &out->drawn };
    for (uint32_t c = 'a'; c <= 'z'; c++) {
        uint32_t gid = 0;
        daegun_image image;
        if (daegun_font_glyph_id(font, c, &gid) != DAEGUN_OK
            || daegun_font_outline(font, gid, at, &pen) != DAEGUN_OK
            || daegun_rasterizer_glyph(r, font, gid, at, 20.0f, NULL, &image) != DAEGUN_OK) {
            return 0;
        }
        out->raster = out->raster * 31 + (uint64_t)image.width * 7 + image.height;
        for (size_t i = 0; i < image.pixels_len; i++) {
            out->raster = out->raster * 1099511628211u + image.pixels[i];
        }
    }

    uint32_t g = 0;
    daegun_hinted_outline *hinted = NULL;
    ok = daegun_font_glyph_id(font, 'g', &g) == DAEGUN_OK
      && daegun_font_hinted_outline(font, g, at, 16.0f, DAEGUN_HINT_AUTO, &hinted) == DAEGUN_OK;
    if (ok) {
        daegun_hinted_outline_points(hinted, &out->points);
    }
    daegun_hinted_outline_free(hinted);

    daegun_blob *instance = NULL;
    ok = ok && daegun_font_instance(font, at, &instance) == DAEGUN_OK;
    if (ok) {
        daegun_blob_data(instance, &out->instance);
    }
    daegun_blob_free(instance);

    daegun_line_metrics metrics;
    if (!ok || daegun_font_line_metrics(font, at, DAEGUN_HORIZONTAL, &metrics) != DAEGUN_OK) {
        return 0;
    }
    out->ascent = metrics.ascent;
    return 1;
}

static void *thread(void *arg)
{
    work *w = arg;
    w->rasterizer = daegun_rasterizer_new();
    for (int round = 0; round < ROUNDS; round++) {
        answers now;
        const daegun_location *at = round % 2 ? NULL : bolds[w->font];
        if (!ask(fonts[w->font], at, &now, &w->kept[round], w->rasterizer)) {
            fprintf(stderr, "  FAIL round %d: %s\n", round, daegun_last_error().data);
            w->failed = 1;
            break;
        }
        if (round < 2) {
            w->at[round] = now;
        } else if (!same(&now, &w->at[round % 2])) {
            fprintf(stderr, "  FAIL round %d answered otherwise than round %d\n", round, round % 2);
            w->failed = 1;
        }
        if (round % 4 == 3) {
            daegun_thread_release();
        }
    }
    atomic_fetch_sub(&reading, 1);
    return NULL;
}

/* Each cache's budget, and the pool's, set to nothing, to evicting and to room, their stats read and the cache cleared,
 * each cache moved out of the pool and back, and a prewarm at either location, until the readers are done: none of it
 * may change an answer. */
static void *mutator(void *arg)
{
    int *failed = arg;
    static const size_t budgets[] = { 0, 16 << 10, 8 << 20 };
    uint32_t gids[64];
    for (uint32_t i = 0; i < 64; i++) {
        gids[i] = i;
    }
    for (unsigned n = 0; atomic_load(&reading) > 0; n++) {
        daegun_font *font = fonts[n % 2];
        daegun_cache_stats stats;
        for (int32_t cache = DAEGUN_CACHE_SHAPE; cache <= DAEGUN_CACHE_RASTER; cache++) {
            if (daegun_font_set_cache_budget(font, cache, budgets[n % 3]) != DAEGUN_OK
                || daegun_font_cache_stats(font, cache, &stats) != DAEGUN_OK
                || daegun_font_clear_cache(font, cache) != DAEGUN_OK
                || daegun_font_set_cache_pool(font, cache, n % 3 ? pool : NULL) != DAEGUN_OK) {
                *failed = 1;
            }
        }
        if (daegun_cache_pool_set_budget(pool, budgets[(n + 1) % 3]) != DAEGUN_OK
            || daegun_cache_pool_stats(pool, &stats) != DAEGUN_OK) {
            *failed = 1;
        }
        size_t added = 0;
        if (daegun_font_prewarm(font, gids, 64, n % 4 < 2 ? bolds[n % 2] : NULL, &added) != DAEGUN_OK) {
            *failed = 1;
        }
    }
    return NULL;
}

static unsigned char *slurp(const char *path, size_t *len)
{
    FILE *f = fopen(path, "rb");
    if (!f) {
        return NULL;
    }
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    unsigned char *data = n > 0 ? malloc((size_t)n) : NULL;
    if (data && fread(data, 1, (size_t)n, f) != (size_t)n) {
        free(data);
        data = NULL;
    }
    fclose(f);
    *len = (size_t)n;
    return data;
}

int main(int argc, char **argv)
{
    size_t len = 0;
    unsigned char *bytes = argc > 1 ? slurp(argv[1], &len) : NULL;
    daegun_coord wght = { DAEGUN_TAG('w', 'g', 'h', 't'), 700.0 };
    pool = daegun_cache_pool_new(256 << 10);
    for (int k = 0; k < 2; k++) {
        if (!bytes || daegun_font_open(bytes, len, &fonts[k]) != DAEGUN_OK
            || daegun_font_location(fonts[k], &wght, 1, &bolds[k]) != DAEGUN_OK) {
            fprintf(stderr, "usage: threads path/to/InterVariable.ttf\n");
            return 2;
        }
        /* Room for a few dozen of the 52 images, so threads evict while others insert. */
        if (daegun_font_set_cache_budget(fonts[k], DAEGUN_CACHE_RASTER, 16 * 1024) != DAEGUN_OK) {
            return 2;
        }
        for (int32_t cache = DAEGUN_CACHE_SHAPE; cache <= DAEGUN_CACHE_RASTER; cache++) {
            if (daegun_font_set_cache_pool(fonts[k], cache, pool) != DAEGUN_OK) {
                return 2;
            }
        }
    }
    free(bytes);

    static work works[WAVES * THREADS];
    pthread_t threads[THREADS], mutating;
    int failed = 0, mutation_failed = 0;
    for (int wave = 0; wave < WAVES; wave++) {
        atomic_store(&reading, THREADS);
        for (int i = 0; i < THREADS; i++) {
            works[wave * THREADS + i].font = i % 2;
            if (pthread_create(&threads[i], NULL, thread, &works[wave * THREADS + i]) != 0) {
                fprintf(stderr, "  FAIL thread %d did not start\n", i);
                return 1;
            }
        }
        if (pthread_create(&mutating, NULL, mutator, &mutation_failed) != 0) {
            fprintf(stderr, "  FAIL the mutating thread did not start\n");
            return 1;
        }
        for (int i = 0; i < THREADS; i++) {
            pthread_join(threads[i], NULL);
        }
        pthread_join(mutating, NULL);
    }
    if (mutation_failed) {
        fprintf(stderr, "  FAIL a budget, stats, clear, pool or prewarm call failed\n");
        failed = 1;
    }
    for (int i = 0; i < WAVES * THREADS; i++) {
        failed |= works[i].failed;
        for (int k = 0; k < 2 && !works[i].failed; k++) {
            if (!same(&works[i].at[k], &works[0].at[k])) {
                fprintf(stderr, "  FAIL thread %d answered otherwise than thread 0\n", i);
                failed = 1;
            }
        }
        for (int round = 0; round < ROUNDS; round++) {
            daegun_shaped_run_free(works[i].kept[round]);
        }
        daegun_rasterizer_free(works[i].rasterizer);
    }
    /* Freed while the fonts are in it: they hold it until they go. */
    daegun_cache_pool_free(pool);
    for (int k = 0; k < 2; k++) {
        daegun_location_free(bolds[k]);
        daegun_font_free(fonts[k]);
    }
    printf("daegun C threads\n  %s\n", failed ? "FAILED" : "ok");
    return failed;
}
