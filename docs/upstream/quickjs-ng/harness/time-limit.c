/* Runs a script with a time limit enforced the documented way: an interrupt
 * handler that compares a clock with a deadline.
 *
 *   time-limit LIMIT_MS GIVE_UP_S script.js
 *
 * The deadline starts when the script calls armDeadline(), a global this
 * program adds, so setup doesn't count. Prints how long past the deadline the
 * script stopped. Exit 0: within a second of the deadline; 1: later; 3: still
 * running GIVE_UP_S seconds in.
 */
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#include <unistd.h>
#include "quickjs.h"

static double now_ms(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1e3 + t.tv_nsec / 1e6;
}

static double deadline, limit_ms;

static int handler(JSContext *ctx, void *opaque)
{
    return deadline > 0 && now_ms() > deadline;
}

static JSValue arm_deadline(JSContext *ctx, JSValueConst this_val, int argc,
                            JSValueConst *argv)
{
    deadline = now_ms() + limit_ms;
    return JS_UNDEFINED;
}

static void gave_up(int sig)
{
    static const char msg[] = "still running: the deadline was ignored\n";
    (void)!write(1, msg, sizeof(msg) - 1);
    _exit(3);
}

int main(int argc, char **argv)
{
    if (argc != 4)
        return 2;
    FILE *f = fopen(argv[3], "rb");
    if (!f)
        return 2;
    static char src[1 << 20];
    size_t len = fread(src, 1, sizeof(src) - 1, f);
    fclose(f);
    src[len] = 0;
    JSRuntime *rt = JS_NewRuntime();
    JSContext *ctx = JS_NewContext(rt);
    signal(SIGALRM, gave_up);
    alarm(atoi(argv[2]));
    limit_ms = atof(argv[1]);
    JS_SetInterruptHandler(rt, handler, NULL);
    JSValue global = JS_GetGlobalObject(ctx);
    JS_SetPropertyStr(ctx, global, "armDeadline",
                      JS_NewCFunction(ctx, arm_deadline, "armDeadline", 0));
    JS_FreeValue(ctx, global);
    JSValue v = JS_Eval(ctx, src, len, "<script>", JS_EVAL_TYPE_GLOBAL);
    int exception = JS_IsException(v);
    JS_FreeValue(ctx, v);
    double late = now_ms() - deadline;
    printf("%s %.0f ms after the deadline\n", exception ? "stopped" : "finished", late);
    return late < 1000 ? 0 : 1;
}
