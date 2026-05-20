#include <stdio.h>
#include <pthread.h>
#include <unistd.h>
#include <termios.h>
#include <sys/select.h>
#include <signal.h>
#include "bundle.h"

extern void __kome_runtime_process_events(void);
extern void __kome_std_keyboard_onPress(void *any, void *closure);

static int g_loop_started = 0;
static volatile sig_atomic_t g_keep_running = 1;
static struct termios g_orig_termios;
static int g_tc_ok = 0;

/// スタンダードバンドルのメインループ
///
/// 1. イベント処理を呼び出す（レシピ登録などの処理）
/// 2. stdin をポーリングしてキー入力を検出
/// 3. キー入力を検出したらruntimeのキーハンドラを呼ぶ
static void *bundle_main_loop(void *arg) {
    (void)arg;

    /* Setup terminal raw mode if possible and store original settings globally so
     * signal handler can restore them. */
    if (isatty(STDIN_FILENO)) {
        if (tcgetattr(STDIN_FILENO, &g_orig_termios) == 0) {
            struct termios raw = g_orig_termios;
            raw.c_lflag &= ~(ICANON | ECHO);
            raw.c_cc[VMIN] = 1;
            raw.c_cc[VTIME] = 0;
            if (tcsetattr(STDIN_FILENO, TCSANOW, &raw) == 0) {
                g_tc_ok = 1;
            }
        }
    }

    while (g_keep_running) {
        /* 呼び出し開始と定期イベント処理 */
        __kome_runtime_process_events();

        /* stdin をポーリングしてキー入力を検出 */
        fd_set rfds;
        FD_ZERO(&rfds);
        FD_SET(STDIN_FILENO, &rfds);
        struct timeval tv;
        tv.tv_sec = 0;
        tv.tv_usec = 16 * 1000; /* 16ms */

        int ret = select(STDIN_FILENO + 1, &rfds, NULL, NULL, &tv);
        if (ret > 0 && FD_ISSET(STDIN_FILENO, &rfds)) {
            char buf[64];
            ssize_t n = read(STDIN_FILENO, buf, sizeof(buf));
            if (n > 0) {
                __kome_std_keyboard_onPress(NULL, NULL);
            }
        }
    }

    if (g_tc_ok) {
        tcsetattr(STDIN_FILENO, TCSANOW, &g_orig_termios);
        g_tc_ok = 0;
    }

    return NULL;
}

/// スタンダードバンドルのメインループを開始する関数
///
/// この関数は main() の中で一度だけ呼び出すべきで、複数回呼び出すとエラーになる。
/// この関数は新しいスレッドを作成してbundle_main_loopを実行する。
static void signal_handler(int signo) {
    (void)signo;
    g_keep_running = 0;
}

void __kome_runtime_start_loop(void) {
    if (g_loop_started) {
        fprintf(stderr, "__kome_runtime_start_loop: already running\n");
        return;
    }

    g_loop_started = 1;
    fprintf(stderr, "__kome_runtime_start_loop: std bundle main loop (blocking) started\n");

    /* Install simple signal handlers to ensure terminal is restored on Ctrl+C */
    signal(SIGINT, signal_handler);
    signal(SIGTERM, signal_handler);

    /* Run the main loop in this thread (blocking) */
    bundle_main_loop(NULL);

    /* cleanup already handled inside bundle_main_loop */
}
