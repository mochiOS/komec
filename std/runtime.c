#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "bundle.h"
#include "io/keyboard.h"
#include <pthread.h>
#include <unistd.h>
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
extern void *dlopen(const char *filename, int flag);
extern void *dlsym(void *handle, const char *symbol);
extern int dlclose(void *handle);
#ifndef RTLD_LAZY
#define RTLD_LAZY 1
#endif
#include <ctype.h>

typedef struct Sub {
    char *name;
    void *callback;
    struct Sub *next;
} Sub;

static Sub *subscriptions = NULL;

/// ランタイムにコールバックを保存する
///
/// # Arguments
/// * `name` - コールバックの名前 (デバッグ用)
/// * `callback` - コールバック関数へのポインタ
void __kome_runtime_subscribe(const char *name, void *callback) {
    char *copy = NULL;
    if (name) copy = strdup(name);
    else copy = strdup("(null)");

    Sub *s = (Sub*)malloc(sizeof(Sub));
    s->name = copy;
    s->callback = callback;
    s->next = subscriptions;
    subscriptions = s;

    fprintf(stderr, "__kome_runtime_subscribe: %s -> %p\n", copy, callback);
}

/// キーボードイベントが発生したときに呼び出される関数
///
/// # Arguments
/// * `any` - 任意のデータ
/// * `closure` - コールバック関数へのポインタ
void keyboard_onPress(void *any, void *closure) {
    (void)any;
    fprintf(stderr, "keyboard_onPress: closure=%p - invoking subscriptions\n", closure);

    if (closure) {
        void (*cb)(void) = (void(*)(void))closure;
        cb();
    }

    Sub *cur = subscriptions;
    while (cur) {
        if (cur->callback) {
            void (*cb2)(void) = (void(*)(void))cur->callback;
            cb2();
        }
        cur = cur->next;
    }
}

/*
 * Convenience wrapper used by Kome code when calling `keyboard.scan(any) { ... }`.
 * The Kome code generation will produce a call to `keyboard_scan` with the
 * closure function pointer; here we register that closure with the runtime so
 * it will be invoked when a key press is detected.
 */
void keyboard_scan(void *any, void *closure) {
    (void)any;
    if (closure) {
        __kome_runtime_subscribe("keyboard.scan", closure);
        fprintf(stderr, "keyboard_scan: subscribed closure %p\n", closure);
    } else {
        fprintf(stderr, "keyboard_scan: called with NULL closure\n");
    }
}

/// プログラムのコマンドライン引数を解析して、.komeファイルを見つける。
/// そのファイルを読み込んで、bundleとrecipeの定義を探し、対応する関数を動的にロードして保存する。
/// その後、定期的にキーボードイベントをシミュレートして、保存したコールバック関数を呼び出す。
///
/// # Arguments
/// * `arg` - 使われない引数lol
static void *event_thread(void *arg) {
    (void)arg;
    /* wait for potential subscription registration to happen in main */
    usleep(200 * 1000); /* 200ms */

    char cmdline_buf[4096];
    FILE *f = fopen("/proc/self/cmdline", "r");
    if (!f) return NULL;
    size_t n = fread(cmdline_buf, 1, sizeof(cmdline_buf)-1, f);
    fclose(f);
    if (n == 0) return NULL;
    cmdline_buf[n] = '\0';

    char *p = cmdline_buf;
    char *source_file = NULL;
    while (p < cmdline_buf + n) {
        if (strlen(p) > 5) {
            size_t L = strlen(p);
            if (L >= 5 && strcmp(p + L - 5, ".kome") == 0) {
                source_file = strdup(p);
                break;
            }
        }
        p += strlen(p) + 1;
    }

    #define MAX_FUNCS 64
    void (*found_funcs[MAX_FUNCS])(void);
    int found_count = 0;

    if (source_file) {
        FILE *sf = fopen(source_file, "r");
        if (sf) {
            char line[1024];
            char bundle_name[256] = "";
            while (fgets(line, sizeof(line), sf)) {
                char *s = line;
                while (*s == ' ' || *s == '\t') s++;
                if (strncmp(s, "bundle ", 7) == 0) {
                    char *start = s + 7;
                    while (*start == ' ') start++;
                    char *end = start;
                    while (*end && (*end == '_' || isalnum((unsigned char)*end))) end++;
                    size_t blen = end - start;
                    if (blen > 0 && blen < sizeof(bundle_name)) {
                        strncpy(bundle_name, start, blen);
                        bundle_name[blen] = '\0';
                    }
                }
                if (strncmp(s, "recipe ", 7) == 0) {
                    char *start = s + 7;
                    while (*start == ' ') start++;
                    char *end = start;
                    while (*end && (*end == '_' || isalnum((unsigned char)*end))) end++;
                    size_t rlen = end - start;
                    if (rlen > 0 && rlen < 200 && bundle_name[0] != '\0') {
                        char funcname[512];
                        snprintf(funcname, sizeof(funcname), "%s_recipe_%.*s", bundle_name, (int)rlen, start);
                        void *main_handle = dlopen(NULL, RTLD_LAZY);
                        void *sym = NULL;
                        if (main_handle) {
                            sym = dlsym(main_handle, funcname);
                        }
                        if (sym && found_count < MAX_FUNCS) {
                            found_funcs[found_count++] = (void(*)(void))sym;
                            fprintf(stderr, "runtime: discovered recipe function: %s -> %p\n", funcname, sym);
                        }
                    }
                }
            }
            fclose(sf);
        }
        free(source_file);
    }

    /* simulate a few key presses */
    for (int i = 0; i < 5; ++i) {
        fprintf(stderr, "runtime: simulating keyboard press %d\n", i+1);
        for (int j = 0; j < found_count; ++j) {
            found_funcs[j]();
        }
        keyboard_onPress(NULL, NULL);
        usleep(200 * 1000);
    }

    return NULL;
}

/* Explicit event processing function.
 * Call this after main() to deliver queued callbacks.
 * This is needed because JIT generation happens after constructor runs,
 * so subscriptions are not set up until after main() runs.
 *
 * This function will:
 * 1. Try to find and call onPress() to register recipes
 * 2. Then invoke all subscribed callbacks
 */
void __kome_runtime_process_events(void) {
    /* Attempt to call onPress() to register recipes */
    void (*on_press)(void) = dlsym(dlopen(NULL, RTLD_LAZY), "onPress");
    if (on_press) {
        fprintf(stderr, "__kome_runtime_process_events: calling onPress() to register recipes\n");
        on_press();
    }

    /* Call all registered subscriptions */
    Sub *cur = subscriptions;
    int count = 0;
    while (cur) {
        if (cur->callback) {
            void (*cb)(void) = (void(*)(void))cur->callback;
            cb();
            count++;
        }
        cur = cur->next;
    }
    fprintf(stderr, "__kome_runtime_process_events: invoked %d callbacks\n", count);
}

__attribute__((constructor))
static void __kome_std_runtime_init(void) {
    pthread_t th;
    if (pthread_create(&th, NULL, event_thread, NULL) == 0) {
        pthread_detach(th);
    }
}

/* Destructor: called when program is shutting down (after __kome_entry returns).
 * This gives us a chance to invoke discovered recipe callbacks so that
 * examples which don't keep a running event loop can still demonstrate
 * state/reactivity before the process exits.
 */
__attribute__((destructor))
static void __kome_std_runtime_shutdown(void) {
    /* small delay to allow finalization elsewhere */
    usleep(50 * 1000);

    /* invoke subscriptions once more at shutdown */
    Sub *cur = subscriptions;
    while (cur) {
        if (cur->callback) {
            void (*cb)(void) = (void(*)(void))cur->callback;
            cb();
        }
        cur = cur->next;
    }
}
