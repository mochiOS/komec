#ifndef KOME_STD_IO_KEYBOARD_H
#define KOME_STD_IO_KEYBOARD_H

#ifdef __cplusplus
extern "C" {
#endif

/* Keyboard runtime hooks used by Kome std/io/keyboard.kome and runtime.c */
void keyboard_onPress(void* any, void* closure);
void keyboard_scan(void* any, void* closure);

#ifdef __cplusplus
}
#endif

#endif /* KOME_STD_IO_KEYBOARD_H */