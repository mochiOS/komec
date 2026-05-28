#include <stdio.h>
#include "printfv.h"

// printf は C の可変長引数なので、配列をそのまま渡せないからlenに応じて引数を展開して呼ぶ
//
// TODO: i32以外も対応する
// NOTE: 一旦は std 側の都合で最大 8 個までに制限している（len > 8 はエラー扱い）。
int __kome_printf_i32v(const char *fmt, const int *data, int len) {
    if (!fmt) return -1;
    if (!data) len = 0;

    int r = -1;
    switch (len) {
        case 0:  r = printf("%s", fmt); break;
        case 1:  r = printf(fmt, data[0]); break;
        case 2:  r = printf(fmt, data[0], data[1]); break;
        case 3:  r = printf(fmt, data[0], data[1], data[2]); break;
        case 4:  r = printf(fmt, data[0], data[1], data[2], data[3]); break;
        case 5:  r = printf(fmt, data[0], data[1], data[2], data[3], data[4]); break;
        case 6:  r = printf(fmt, data[0], data[1], data[2], data[3], data[4], data[5]); break;
        case 7:  r = printf(fmt, data[0], data[1], data[2], data[3], data[4], data[5], data[6]); break;
        case 8:  r = printf(fmt, data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7]); break;
        default:
            r = -1;
            break;
    }
    fflush(stdout);
    return r;
}
