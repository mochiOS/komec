#include "process.h"
#include <stdlib.h>

int __kome_process_exec(const char* command_ptr) {
    if (!command_ptr) return -1;
    return system(command_ptr);
}

void __kome_process_exit(int code) {
    exit(code);
}
