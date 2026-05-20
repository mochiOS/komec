#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "std.h"
#include "bundle.h"
#include "io/keyboard.h"
#include "runtime.h"
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
static int g_onpress_called = 0;

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

    debug("__kome_runtime_subscribe: %s -> %p\n", copy, callback);
}

void __kome_runtime_invoke_subscriptions(void) {
    Sub *cur = subscriptions;
    while (cur) {
        if (cur->callback) {
            void (*cb2)(void) = (void(*)(void))cur->callback;
            cb2();
        }
        cur = cur->next;
    }
}

void __kome_runtime_emit(const char *name) {
    if (!name) return;
    Sub *cur = subscriptions;
    while (cur) {
        if (cur->callback && cur->name && strcmp(cur->name, name) == 0) {
            void (*cb2)(void) = (void(*)(void))cur->callback;
            cb2();
        }
        cur = cur->next;
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
                            debug("runtime: discovered recipe function: %s -> %p\n", funcname, sym);
                        }
                    }
                }
            }
            fclose(sf);
        }
        free(source_file);
    }

    const char *sim = getenv("KOME_STD_SIMULATE_KEYS");
    if (sim && sim[0] == '1') {
        for (int i = 0; i < 5; ++i) {
            debug("runtime: simulating keyboard press %d\n", i+1);
            for (int j = 0; j < found_count; ++j) {
                found_funcs[j]();
            }
            __kome_std_keyboard_onPress(NULL, NULL);
            usleep(200 * 1000);
        }
    }

    return NULL;
}

/// 定期的に呼び出される関数。ここでは、onPress()を呼び出してレシピを登録し、その後、保存されたコールバックをすべて呼び出す。
void __kome_runtime_process_events(void) {
    /* Attempt to call onPress() to register recipes */
    if (!g_onpress_called) {
        g_onpress_called = 1;
        void (*on_press)(void) = dlsym(dlopen(NULL, RTLD_LAZY), "onPress");
        if (on_press) {
            debug("__kome_runtime_process_events: calling onPress() to register recipes\n");
            on_press();
        }
    }
}

__attribute__((constructor))
static void __kome_std_runtime_init(void) {
    pthread_t th;
    if (pthread_create(&th, NULL, event_thread, NULL) == 0) {
        pthread_detach(th);
    }
}

/// プログラム終了時に呼び出される関数。少し待ってから保存されたコールバックをすべて呼び出す。
__attribute__((destructor))
static void __kome_std_runtime_shutdown(void) {
    (void)0;
}
