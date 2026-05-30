#ifndef KOME_VIEWKIT_H
#define KOME_VIEWKIT_H

void* kome_viewkit_app_create(void);
void  kome_viewkit_app_destroy(void* app_ptr);
void  kome_viewkit_window_create(void* app_ptr, int width, int height, const char* title_ptr, int no_decoration);
int   kome_viewkit_register_component(void* app_ptr, const char* name_ptr, const char* html_ptr);
void  kome_viewkit_update_ui_tree(void* app_ptr, const char* tree_json_ptr);
void  kome_viewkit_app_run(void* app_ptr);
void  kome_viewkit_app_run_async(void* app_ptr);
void  kome_viewkit_set_key_tap_callback_raw(void* app_ptr, void* callback_ptr);
int   kome_viewkit_async_is_running(void);

#endif /* KOME_VIEWKIT_H */
