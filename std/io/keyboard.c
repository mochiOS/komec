#include <stdio.h>
#include "keyboard.h"

int kome_std_io_keyboard_onPress(char* key) {
    return scanf("%s", key);
}