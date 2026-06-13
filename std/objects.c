#include "objects.h"
#include <ctype.h>
#include <dirent.h>
#include <errno.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>

typedef struct {
    size_t len;
    size_t cap;
    char** items;
} KomeList;

typedef struct KomeTomlPair {
    char* key;
    char* value;
    struct KomeTomlPair* next;
} KomeTomlPair;

typedef struct {
    KomeTomlPair* head;
} KomeToml;

static char* kome_strdup(const char* s) {
    if (!s) return NULL;
    size_t len = strlen(s);
    char* out = (char*)malloc(len + 1);
    if (!out) return NULL;
    memcpy(out, s, len + 1);
    return out;
}

static char* kome_strndup(const char* s, size_t len) {
    char* out = (char*)malloc(len + 1);
    if (!out) return NULL;
    memcpy(out, s, len);
    out[len] = '\0';
    return out;
}

static char* json_escape(const char* s) {
    if (!s) s = "";
    size_t cap = strlen(s) * 2 + 1;
    char* out = (char*)malloc(cap);
    if (!out) return NULL;
    size_t len = 0;
    for (const unsigned char* p = (const unsigned char*)s; *p; p++) {
        unsigned char ch = *p;
        if (len + 2 >= cap) {
            cap *= 2;
            char* next = (char*)realloc(out, cap);
            if (!next) {
                free(out);
                return NULL;
            }
            out = next;
        }
        switch (ch) {
            case '\\': out[len++] = '\\'; out[len++] = '\\'; break;
            case '"': out[len++] = '\\'; out[len++] = '"'; break;
            case '\n': out[len++] = '\\'; out[len++] = 'n'; break;
            case '\r': out[len++] = '\\'; out[len++] = 'r'; break;
            case '\t': out[len++] = '\\'; out[len++] = 't'; break;
            default: out[len++] = (char)ch; break;
        }
    }
    out[len] = '\0';
    return out;
}

static char* insert_before_last_char(const char* base, char target, const char* insertion) {
    if (!base) return kome_strdup(insertion);
    const char* pos = strrchr(base, target);
    if (!pos) return kome_strdup(base);
    size_t prefix_len = (size_t)(pos - base);
    size_t ins_len = strlen(insertion);
    size_t suffix_len = strlen(pos);
    char* out = (char*)malloc(prefix_len + ins_len + suffix_len + 1);
    if (!out) return NULL;
    memcpy(out, base, prefix_len);
    memcpy(out + prefix_len, insertion, ins_len);
    memcpy(out + prefix_len + ins_len, pos, suffix_len + 1);
    return out;
}

static char* insert_after_field_open(const char* base, const char* field, const char* insertion) {
    if (!base) return kome_strdup(insertion);
    char pattern[64];
    snprintf(pattern, sizeof(pattern), "\"%s\":{", field);
    const char* pos = strstr(base, pattern);
    if (!pos) return kome_strdup(base);
    pos += strlen(pattern);
    size_t prefix_len = (size_t)(pos - base);
    size_t ins_len = strlen(insertion);
    size_t suffix_len = strlen(pos);
    char* out = (char*)malloc(prefix_len + ins_len + suffix_len + 1);
    if (!out) return NULL;
    memcpy(out, base, prefix_len);
    memcpy(out + prefix_len, insertion, ins_len);
    memcpy(out + prefix_len + ins_len, pos, suffix_len + 1);
    return out;
}

static void list_push(KomeList* list, char* item) {
    if (!list) return;
    if (list->len + 1 > list->cap) {
        size_t next_cap = list->cap == 0 ? 8 : list->cap * 2;
        char** next_items = (char**)realloc(list->items, next_cap * sizeof(char*));
        if (!next_items) return;
        list->items = next_items;
        list->cap = next_cap;
    }
    list->items[list->len++] = item;
}

static char* join_path(const char* base, const char* name) {
    size_t a = strlen(base);
    size_t b = strlen(name);
    bool need_slash = a > 0 && base[a - 1] != '/';
    char* out = (char*)malloc(a + b + (need_slash ? 2 : 1));
    if (!out) return NULL;
    memcpy(out, base, a);
    size_t pos = a;
    if (need_slash) out[pos++] = '/';
    memcpy(out + pos, name, b);
    out[pos + b] = '\0';
    return out;
}

void* __kome_value_map(void* list_ptr, void* closure_ptr) {
    if (!list_ptr) return NULL;
    KomeList* list = (KomeList*)list_ptr;
    KomeList* out = (KomeList*)calloc(1, sizeof(KomeList));
    if (!out) return NULL;
    void* (*cb)(void*, int) = (void* (*)(void*, int))closure_ptr;
    for (size_t i = 0; i < list->len; i++) {
        void* value = cb ? cb(list->items[i], (int)i) : NULL;
        list_push(out, (char*)value);
    }
    return out;
}

void* __kome_value_filter(void* list_ptr, void* closure_ptr) {
    if (!list_ptr) return NULL;
    KomeList* list = (KomeList*)list_ptr;
    KomeList* out = (KomeList*)calloc(1, sizeof(KomeList));
    if (!out) return NULL;
    int (*cb)(void*, int) = (int (*)(void*, int))closure_ptr;
    for (size_t i = 0; i < list->len; i++) {
        int keep = cb ? cb(list->items[i], (int)i) : 0;
        if (keep) {
            list_push(out, list->items[i]);
        }
    }
    return out;
}

int __kome_value_len(void* list_ptr) {
    if (!list_ptr) return 0;
    return (int)((KomeList*)list_ptr)->len;
}

void* __kome_value_index(void* list_ptr, int index) {
    if (!list_ptr) return NULL;
    KomeList* list = (KomeList*)list_ptr;
    if (index < 0 || (size_t)index >= list->len) return NULL;
    return list->items[index];
}

char* __kome_value_name(const char* path_ptr) {
    if (!path_ptr) return NULL;
    const char* end = path_ptr + strlen(path_ptr);
    while (end > path_ptr && end[-1] == '/') end--;
    const char* start = end;
    while (start > path_ptr && start[-1] != '/') start--;
    return kome_strdup(start);
}

bool __kome_value_isDir(const char* path_ptr) {
    if (!path_ptr) return false;
    struct stat st;
    return stat(path_ptr, &st) == 0 && S_ISDIR(st.st_mode);
}

bool __kome_value_hasSuffix(const char* value_ptr, const char* suffix_ptr) {
    if (!value_ptr || !suffix_ptr) return false;
    size_t a = strlen(value_ptr);
    size_t b = strlen(suffix_ptr);
    if (b > a) return false;
    return memcmp(value_ptr + (a - b), suffix_ptr, b) == 0;
}

char* __kome_value_trimSuffix(const char* value_ptr, const char* suffix_ptr) {
    if (!value_ptr || !suffix_ptr) return kome_strdup(value_ptr);
    size_t a = strlen(value_ptr);
    size_t b = strlen(suffix_ptr);
    if (b <= a && memcmp(value_ptr + (a - b), suffix_ptr, b) == 0) {
        char* out = (char*)malloc(a - b + 1);
        if (!out) return NULL;
        memcpy(out, value_ptr, a - b);
        out[a - b] = '\0';
        return out;
    }
    return kome_strdup(value_ptr);
}

static const char* toml_get(KomeToml* toml, const char* key) {
    if (!toml || !key) return NULL;
    for (KomeTomlPair* cur = toml->head; cur; cur = cur->next) {
        if (strcmp(cur->key, key) == 0) return cur->value;
    }
    return NULL;
}

char* __kome_value_entry(void* toml_ptr) {
    return kome_strdup(toml_get((KomeToml*)toml_ptr, "entry"));
}

char* __kome_value_icon(void* toml_ptr) {
    return kome_strdup(toml_get((KomeToml*)toml_ptr, "icon"));
}

char* __kome_i32_to_string(int value) {
    char buf[32];
    snprintf(buf, sizeof(buf), "%d", value);
    return kome_strdup(buf);
}

char* __kome_bool_to_string(bool value) {
    return kome_strdup(value ? "true" : "false");
}

char* __kome_record_field(const char* record_ptr, const char* key_ptr) {
    if (!record_ptr || !key_ptr || !*key_ptr) return NULL;
    size_t key_len = strlen(key_ptr);
    const char* p = record_ptr;
    while (*p) {
        const char* key_start = strstr(p, key_ptr);
        if (!key_start) return NULL;
        if (key_start != p && key_start[-1] != ';' && key_start[-1] != '{') {
            p = key_start + 1;
            continue;
        }
        const char* eq = key_start + key_len;
        if (*eq != '=') {
            p = key_start + 1;
            continue;
        }
        const char* value_start = eq + 1;
        const char* value_end = strchr(value_start, ';');
        if (!value_end) value_end = value_start + strlen(value_start);
        if (value_end == value_start) return NULL;
        return kome_strndup(value_start, (size_t)(value_end - value_start));
    }
    return NULL;
}

char* __kome_value_image(const char* base_ptr, ...) {
    va_list ap;
    va_start(ap, base_ptr);
    const char* path_ptr = va_arg(ap, const char*);
    va_end(ap);

    if (!base_ptr) return NULL;
    if (!path_ptr) return kome_strdup(base_ptr);

    char* escaped = json_escape(path_ptr);
    if (!escaped) return kome_strdup(base_ptr);
    char insertion[512];
    snprintf(insertion, sizeof(insertion), ",\"content\":{\"type\":\"Image\",\"value\":\"%s\"}}", escaped);
    free(escaped);
    return insert_before_last_char(base_ptr, '}', insertion);
}

char* __kome_value_selected(const char* base_ptr, ...) {
    va_list ap;
    va_start(ap, base_ptr);
    int enabled = va_arg(ap, int);
    va_end(ap);

    if (!base_ptr || !enabled) return kome_strdup(base_ptr);
    const char* props_pos = strstr(base_ptr, "\"props\":{");
    if (props_pos && props_pos[strlen("\"props\":{")] == '}') {
        return insert_after_field_open(base_ptr, "props", "\"selected\":true");
    }
    return insert_after_field_open(base_ptr, "props", "\"selected\":true,");
}

static KomeList* list_from_dir(const char* path_ptr) {
    DIR* dir = opendir(path_ptr);
    if (!dir) return NULL;
    KomeList* list = (KomeList*)calloc(1, sizeof(KomeList));
    if (!list) {
        closedir(dir);
        return NULL;
    }
    struct dirent* entry;
    while ((entry = readdir(dir)) != NULL) {
        if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0) continue;
        char* full = join_path(path_ptr, entry->d_name);
        list_push(list, full);
    }
    closedir(dir);
    return list;
}

void* __kome_fs_list(const char* path_ptr) {
    return list_from_dir(path_ptr);
}

char* __kome_fs_read(const char* path_ptr) {
    if (!path_ptr) return NULL;
    FILE* f = fopen(path_ptr, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long len = ftell(f);
    fseek(f, 0, SEEK_SET);
    if (len < 0) {
        fclose(f);
        return NULL;
    }
    char* out = (char*)malloc((size_t)len + 1);
    if (!out) {
        fclose(f);
        return NULL;
    }
    size_t read_len = fread(out, 1, (size_t)len, f);
    out[read_len] = '\0';
    fclose(f);
    return out;
}

char* __kome_toml_parse(const char* text_ptr) {
    if (!text_ptr) return NULL;
    KomeToml* toml = (KomeToml*)calloc(1, sizeof(KomeToml));
    if (!toml) return NULL;
    char* copy = kome_strdup(text_ptr);
    if (!copy) return NULL;

    for (char* line = strtok(copy, "\n"); line; line = strtok(NULL, "\n")) {
        while (*line && isspace((unsigned char)*line)) line++;
        if (*line == '\0' || *line == '#') continue;
        char* eq = strchr(line, '=');
        if (!eq) continue;
        *eq = '\0';
        char* key = line;
        while (*key && isspace((unsigned char)*key)) key++;
        char* key_end = key + strlen(key);
        while (key_end > key && isspace((unsigned char)key_end[-1])) *--key_end = '\0';

        char* value = eq + 1;
        while (*value && isspace((unsigned char)*value)) value++;
        char* value_end = value + strlen(value);
        while (value_end > value && isspace((unsigned char)value_end[-1])) *--value_end = '\0';
        if (*value == '"' && value_end > value + 1 && value_end[-1] == '"') {
            value++;
            value_end[-1] = '\0';
        }

        KomeTomlPair* pair = (KomeTomlPair*)calloc(1, sizeof(KomeTomlPair));
        if (!pair) continue;
        pair->key = kome_strdup(key);
        pair->value = kome_strdup(value);
        pair->next = toml->head;
        toml->head = pair;
    }

    free(copy);
    return (char*)toml;
}
