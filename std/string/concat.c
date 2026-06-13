#include "concat.h"
#include <stdlib.h>
#include <string.h>

char *__kome_str_concat(const char *a, const char *b) {
    if (!a) a = "";
    if (!b) b = "";
    size_t alen = strlen(a);
    size_t blen = strlen(b);
    size_t total = alen + blen + 1;
    char *out = (char *)malloc(total);
    if (!out) return NULL;
    memcpy(out, a, alen);
    memcpy(out + alen, b, blen);
    out[alen + blen] = '\0';
    return out;
}

char *concat(const char *a, const char *b) {
    return __kome_str_concat(a, b);
}
