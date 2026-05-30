#include "components.h"
#include <stdlib.h>
#include <string.h>

static void append(char** buf, size_t* cap, size_t* len, const char* s) {
    size_t slen = s ? strlen(s) : 0;
    if (*len + slen + 1 > *cap) {
        size_t ncap = (*cap == 0) ? 256 : *cap;
        while (*len + slen + 1 > ncap) ncap *= 2;
        char* nb = (char*)realloc(*buf, ncap);
        if (!nb) return;
        *buf = nb;
        *cap = ncap;
    }
    if (slen > 0) {
        memcpy(*buf + *len, s, slen);
        *len += slen;
    }
    (*buf)[*len] = '\0';
}

static void append_escaped_json_string(char** buf, size_t* cap, size_t* len, const char* s) {
    if (!s) s = "";
    for (const unsigned char* p = (const unsigned char*)s; *p; p++) {
        unsigned char ch = *p;
        switch (ch) {
            case '\\': append(buf, cap, len, "\\\\"); break;
            case '"':  append(buf, cap, len, "\\\""); break;
            case '\n': append(buf, cap, len, "\\n"); break;
            case '\r': append(buf, cap, len, "\\r"); break;
            case '\t': append(buf, cap, len, "\\t"); break;
            default: {
                char tmp[2] = {(char)ch, 0};
                append(buf, cap, len, tmp);
            } break;
        }
    }
}

char* __kome_viewkit_json_text(const char* value) {
    char* out = NULL;
    size_t cap = 0, len = 0;
    append(&out, &cap, &len, "{\"component\":\"text\",\"props\":{},\"children\":[],\"content\":{\"type\":\"text\",\"value\":\"");
    append_escaped_json_string(&out, &cap, &len, value);
    append(&out, &cap, &len, "\"}}");
    return out;
}

char* __kome_viewkit_json_component(const char* name, const char** children, int len) {
    if (!name) name = "div";
    char* out = NULL;
    size_t cap = 0, olen = 0;
    append(&out, &cap, &olen, "{\"component\":\"");
    append_escaped_json_string(&out, &cap, &olen, name);
    append(&out, &cap, &olen, "\",\"props\":{},\"children\":[");

    int first = 1;
    for (int i = 0; i < len; i++) {
        const char* c = children ? children[i] : NULL;
        if (!c) continue;
        if (!first) append(&out, &cap, &olen, ",");
        first = 0;
        append(&out, &cap, &olen, c);
    }

    append(&out, &cap, &olen, "]}");
    return out;
}

static const char* find_component_name(const char* base, char* tmp, size_t tmpcap) {
    if (!base) return NULL;
    const char* key = "\"component\":\"";
    const char* p = strstr(base, key);
    if (!p) return NULL;
    p += strlen(key);
    const char* end = strchr(p, '"');
    if (!end) return NULL;
    size_t n = (size_t)(end - p);
    if (n + 1 > tmpcap) return NULL;
    memcpy(tmp, p, n);
    tmp[n] = '\0';
    return tmp;
}

char* __kome_viewkit_json_children(const char* base, const char** children, int len) {
    // base は `__kome_viewkit_json_component` が作った JSON を想定する
    // - component 名だけ抜いて再生成する（props/content のマージは後で）
    char namebuf[128];
    const char* name = find_component_name(base, namebuf, sizeof(namebuf));
    if (!name) name = "div";
    return __kome_viewkit_json_component(name, children, len);
}
