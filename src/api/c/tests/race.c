/* Two threads drawing into one path at once, which rule 5 forbids: scripts/tools/perf/gate.sh expects TSan to
 * report it, so a clean run of threads.c means the library was instrumented, not that the check saw nothing. */
#include <pthread.h>

#include "daegun.h"

static daegun_path *path;

static void *draw(void *arg)
{
    for (int i = 0; i < 1000; i++) {
        (void)daegun_path_line_to(path, (float)i, arg ? 1.0f : 0.0f);
    }
    return NULL;
}

int main(void)
{
    pthread_t a, b;
    path = daegun_path_new();
    (void)daegun_path_move_to(path, 0, 0);
    if (pthread_create(&a, NULL, draw, NULL) != 0 || pthread_create(&b, NULL, draw, path) != 0) {
        return 2;
    }
    pthread_join(a, NULL);
    pthread_join(b, NULL);
    daegun_path_free(path);
    return 0;
}
