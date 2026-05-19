#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "bundle.h"
#include "io/keyboard.h"

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
	(void)any
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


