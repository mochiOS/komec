#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "std.h"
#include "bundle.h"
#include "runtime.h"

typedef struct Sub {
    const char *name;
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
    Sub *s = (Sub*)malloc(sizeof(Sub));
    // name は JIT 側のグローバル文字列（NUL終端）を指すことがある。
    // ここで strdup すると、実装やメモリ属性次第で落ちることがあったので、
    // ポインタをそのまま保持する（std 側の関数名は不変なので問題ない）。
    s->name = name ? name : "(null)";
    s->callback = callback;
    s->next = subscriptions;
    subscriptions = s;

    const char *dbg = getenv("KOME_DEBUG_RUNTIME");
    if (dbg && dbg[0] == '1') {
        fprintf(stderr, "__kome_runtime_subscribe(name=%p \"%s\", cb=%p)\n", (void*)s->name, s->name, callback);
        fflush(stderr);
    }

    debug("__kome_runtime_subscribe: %s -> %p\n", s->name, callback);
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

/// 定期的に呼び出される関数。
///
/// 以前はここで `onPress()` を `dlsym` してレシピ登録を行っていたが、
/// それは「ソースを解析して関数を探す」などの強いハードコードを伴い、セグフォの温床になっていた。
///
/// 現在はコンパイラが `__kome_register()` を生成し、Rust 側でそれを呼ぶことで登録を行う。
/// そのためここは互換のための no-op として残す。
void __kome_runtime_process_events(void) {
    (void)0;
}
