#include <stdint.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <pthread.h>
#include <stdatomic.h>
#include <unistd.h>

// デバッグ用（環境変数で制御）
static int kome_viewkit_trace_enabled(void) {
    static int cached = -1;
    if (cached != -1) return cached;
    const char* v = getenv("KOME_VIEWKIT_TRACE");
    cached = (v && v[0] == '1') ? 1 : 0;
    return cached;
}

extern void* viewkit_app_create(void);
extern void  viewkit_app_destroy(void* app_ptr);
extern void  viewkit_window_create(void* app_ptr, uint32_t width, uint32_t height, const char* title_ptr, bool no_decoration);
extern bool  viewkit_register_component(void* app_ptr, const char* name_ptr, const char* html_ptr);
extern void  viewkit_update_ui_tree(void* app_ptr, const char* tree_json_ptr);
extern void  viewkit_set_key_tap_callback(void* app_ptr, void (*callback)(uint32_t key_code));
extern void  viewkit_app_run(void* app_ptr);

void* kome_viewkit_app_create(void) {
    if (kome_viewkit_trace_enabled()) {
        fprintf(stderr, "[viewkit_shim] app_create\n");
        fflush(stderr);
    }
    void* app = viewkit_app_create();
    if (kome_viewkit_trace_enabled()) {
        fprintf(stderr, "[viewkit_shim] app_create -> %p\n", app);
        fflush(stderr);
    }
    return app;
}

void kome_viewkit_app_destroy(void* app_ptr) {
    viewkit_app_destroy(app_ptr);
}

void kome_viewkit_window_create(void* app_ptr, int width, int height, const char* title_ptr, int no_decoration) {
    if (!app_ptr) return;

    if (kome_viewkit_trace_enabled()) {
        fprintf(stderr, "[viewkit_shim] window_create\n");
        fflush(stderr);
    }
    uint32_t w = (width  < 0) ? 0u : (uint32_t)width;
    uint32_t h = (height < 0) ? 0u : (uint32_t)height;
    bool nd = no_decoration ? true : false;
    viewkit_window_create(app_ptr, w, h, title_ptr, nd);
}

int kome_viewkit_register_component(void* app_ptr, const char* name_ptr, const char* html_ptr) {
    if (!app_ptr) return 0;
    return viewkit_register_component(app_ptr, name_ptr, html_ptr) ? 1 : 0;
}

void kome_viewkit_update_ui_tree(void* app_ptr, const char* tree_json_ptr) {
    if (!app_ptr) return;
    if (kome_viewkit_trace_enabled()) {
        fprintf(stderr, "[viewkit_shim] update_ui_tree\n");
        fflush(stderr);
    }
    viewkit_update_ui_tree(app_ptr, tree_json_ptr);
}

void kome_viewkit_app_run(void* app_ptr) {
    if (!app_ptr) return;
    if (kome_viewkit_trace_enabled()) {
        fprintf(stderr, "[viewkit_shim] app_run\n");
        fflush(stderr);
    }
    viewkit_app_run(app_ptr);
}

// run_loop はブロッキングなので、別スレッドで回す
static void* kome_viewkit_run_thread(void* arg) {
    void* app_ptr = arg;
    viewkit_app_run(app_ptr);
    return NULL;
}

static atomic_int kome_viewkit_async_running = 0;

void kome_viewkit_app_run_async(void* app_ptr) {
    if (!app_ptr) return;
    pthread_t tid;
    if (pthread_create(&tid, NULL, kome_viewkit_run_thread, app_ptr) == 0) {
        atomic_store(&kome_viewkit_async_running, 1);
        pthread_detach(tid);
    }
}

int kome_viewkit_async_is_running(void) {
    return atomic_load(&kome_viewkit_async_running) ? 1 : 0;
}

void kome_viewkit_set_key_tap_callback_raw(void* app_ptr, void* callback_ptr) {
    if (!app_ptr) return;
    if (!callback_ptr) return;
    void (*cb)(uint32_t) = (void (*)(uint32_t))callback_ptr;
    viewkit_set_key_tap_callback(app_ptr, cb);
}
