#include "std.h"
#include <stdarg.h>
#include <stdio.h>

/// デバッグ出力
void debug(const char *fmt, ...) {
#ifdef DEBUG
    va_list args;
    va_start(args, fmt);
    vfprintf(stderr, fmt, args);
    va_end(args);
#endif /* DEBUG */
}
