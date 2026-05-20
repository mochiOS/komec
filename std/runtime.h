#ifndef KOME_STD_RUNTIME_H
#define KOME_STD_RUNTIME_H

void __kome_runtime_subscribe(const char *name, void *callback);
void __kome_runtime_process_events(void);
void __kome_runtime_emit(const char *name);

#endif /* KOME_STD_RUNTIME_H */
