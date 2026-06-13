#ifndef KOME_STD_RUNTIME_H
#define KOME_STD_RUNTIME_H

void __kome_runtime_subscribe(const char *name, void *callback);
void __kome_runtime_process_events(void);
void __kome_runtime_emit(const char *name);
void __kome_runtime_set_app(void *app);
void* __kome_runtime_get_app(void);

#endif /* KOME_STD_RUNTIME_H */
