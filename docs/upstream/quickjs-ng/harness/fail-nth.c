/* Refuses one allocation of a script run, to exercise out-of-memory paths.
 * The allocator a runtime gets through JS_NewRuntime2 may return NULL, as
 * JS_SetMemoryLimit's check does; only that one allocation is refused.
 *
 *   fail-nth count script.js   print how many allocations the run makes
 *   fail-nth run N script.js   run it with allocation N refused
 *   fail-nth sweep script.js   run every N in a child; print those that crash
 *
 * Link against libqjs built with ASan, which also turns the small-block arena
 * off, so every allocation reaches this allocator. Allocations made while the
 * runtime and context start are not counted. With FAIL_NTH_TRACE set, `run`
 * prints the stack of the refused allocation.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#ifdef __APPLE__
#include <malloc/malloc.h>
#define malloc_usable_size malloc_size
#else
#include <malloc.h>
#endif
#include <signal.h>
#include <sys/wait.h>
#include "quickjs.h"

static long counter, refuse_at;
static int counting;

static int refuse(void)
{
    if (counting && ++counter == refuse_at) {
        if (getenv("FAIL_NTH_TRACE")) {
            extern void __sanitizer_print_stack_trace(void);
            __sanitizer_print_stack_trace();
        }
        return 1;
    }
    return 0;
}
static void *t_calloc(void *o, size_t n, size_t s)
{
    return refuse() ? NULL : calloc(n, s);
}
static void *t_malloc(void *o, size_t s)
{
    return refuse() ? NULL : malloc(s);
}
static void t_free(void *o, void *p)
{
    free(p);
}
static void *t_realloc(void *o, void *p, size_t s)
{
    /* Growing can fail; a shrink can't, with any allocator that matters. */
    if (!p || s > malloc_usable_size(p))
        if (refuse())
            return NULL;
    return realloc(p, s);
}
static size_t t_usable(const void *p)
{
    return malloc_usable_size((void *)p);
}
static const JSMallocFunctions mf = {
    t_calloc, t_malloc, t_free, t_realloc, t_usable,
};

static char *slurp(const char *path, size_t *len)
{
    FILE *f = fopen(path, "rb");
    if (!f) {
        perror(path);
        exit(2);
    }
    fseek(f, 0, SEEK_END);
    *len = ftell(f);
    rewind(f);
    char *b = malloc(*len + 1);
    if (fread(b, 1, *len, f) != *len)
        exit(2);
    b[*len] = 0;
    fclose(f);
    return b;
}

static void run_once(const char *source, size_t len)
{
    JSRuntime *rt = JS_NewRuntime2(&mf, NULL);
    JSContext *ctx = JS_NewContext(rt), *c;
    counting = 1;
    JSValue v = JS_Eval(ctx, source, len, "<script>", JS_EVAL_TYPE_GLOBAL);
    JS_FreeValue(ctx, v);
    JS_FreeValue(ctx, JS_GetException(ctx));
    while (JS_ExecutePendingJob(rt, &c) != 0) {
        JS_FreeValue(ctx, JS_GetException(ctx));
    }
    counting = 0;
    JS_FreeContext(ctx);
    JS_FreeRuntime(rt);
}

int main(int argc, char **argv)
{
    size_t len;
    if (argc >= 3 && !strcmp(argv[1], "count")) {
        char *s = slurp(argv[2], &len);
        refuse_at = -1;
        run_once(s, len);
        printf("%ld\n", counter);
        return 0;
    }
    if (argc == 4 && !strcmp(argv[1], "run")) {
        char *s = slurp(argv[3], &len);
        refuse_at = atol(argv[2]);
        run_once(s, len);
        return 0;
    }
    if (argc == 3 && !strcmp(argv[1], "sweep")) {
        char *s = slurp(argv[2], &len);
        int fds[2];
        long last = 0, crashes = 0;
        if (pipe(fds))
            return 2;
        pid_t p = fork();
        if (!p) {
            refuse_at = -1;
            run_once(s, len);
            if (write(fds[1], &counter, sizeof(counter)) != sizeof(counter))
                _exit(1);
            _exit(0);
        }
        if (read(fds[0], &last, sizeof(last)) != sizeof(last))
            return 2;
        waitpid(p, NULL, 0);
        for (long n = 1; n <= last; n++) {
            p = fork();
            if (!p) {
                /* quiet child; rerun with `run N` to see the report */
                if (!freopen("/dev/null", "w", stderr) || !freopen("/dev/null", "w", stdout))
                    _exit(2);
                refuse_at = n;
                run_once(s, len);
                _exit(0);
            }
            int st;
            waitpid(p, &st, 0);
            if (!WIFEXITED(st) || WEXITSTATUS(st) != 0) {
                printf("%ld\n", n);
                fflush(stdout);
                crashes++;
            }
        }
        fprintf(stderr, "%ld of %ld refusals crashed\n", crashes, last);
        return 0;
    }
    fprintf(stderr, "usage: see the top of fail-nth.c\n");
    return 2;
}
