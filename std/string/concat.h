#pragma once

// 2つの C 文字列を結合して新しい文字列を返す。
// 返り値は `malloc` された領域なので、将来的には GC/arena に寄せたい。
char *__kome_str_concat(const char *a, const char *b);

