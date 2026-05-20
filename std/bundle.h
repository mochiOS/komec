#ifndef KOME_STD_BUNDLE_H
#define KOME_STD_BUNDLE_H

void __kome_runtime_start_loop(void);
void __kome_runtime_subscribe(const char *name, void *callback);
void __kome_runtime_process_events(void);

#endif /* KOME_STD_BUNDLE_H */
