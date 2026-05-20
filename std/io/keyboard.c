#include <stdio.h>
#include "io/keyboard.h"
#include "runtime.h"
#include "../std.h"

/// キーボードイベントが発生したときに呼び出される関数
/// std.io.keyboard.onPress
///
/// # Arguments
/// * `any` - 任意のデータ（未使用）
/// * `closure` - コールバック関数へのポインタ
void __kome_std_keyboard_onPress(void *any, void *closure) {
    (void)any;
    debug("keyboard_onPress: closure=%p - invoking subscriptions\n", closure);

    if (closure) {
        void (*cb)(void) = (void(*)(void))closure;
        cb();
    }

    __kome_runtime_invoke_subscriptions();
}

/// キーボードイベントのスキャン関数
/// std.io.keyboard.scan
///
/// # Arguments
/// * `any` - 任意のデータ（未使用）
/// * `closure` - コールバック関数へのポインタ
void __kome_std_keyboard_scan(void *any, void *closure) {
    (void)any;
    if (closure) {
        __kome_runtime_subscribe("keyboard.scan", closure);
        debug("keyboard_scan: subscribed closure %p\n", closure);
    } else {
        debug("keyboard_scan: called with NULL closure\n");
    }
}
