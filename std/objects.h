#ifndef KOME_OBJECTS_H
#define KOME_OBJECTS_H

#include <stdbool.h>
#include <stdint.h>

void* __kome_value_map(void* list_ptr, void* closure_ptr);
void* __kome_value_filter(void* list_ptr, void* closure_ptr);
int __kome_value_len(void* list_ptr);
void* __kome_value_index(void* list_ptr, int index);

char* __kome_value_name(const char* path_ptr);
bool __kome_value_isDir(const char* path_ptr);
bool __kome_value_hasSuffix(const char* value_ptr, const char* suffix_ptr);
char* __kome_value_trimSuffix(const char* value_ptr, const char* suffix_ptr);

char* __kome_value_entry(void* toml_ptr);
char* __kome_value_icon(void* toml_ptr);
char* __kome_i32_to_string(int value);
char* __kome_bool_to_string(bool value);
char* __kome_record_field(const char* record_ptr, const char* key_ptr);
char* __kome_value_image(const char* base_ptr, ...);
char* __kome_value_selected(const char* base_ptr, ...);

#endif /* KOME_OBJECTS_H */
