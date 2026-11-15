/* Minimal Cortex-M startup for running Tulpa firmware under QEMU. Console output and exit go
   through ARM semihosting (newlib's rdimon), so the simulated board HAL (hal/sim.c) works
   unchanged. */
#include <stdlib.h>

#include "tulpa_hal.h"

extern char __bss_start__, __bss_end__, __stack_top;
extern void initialise_monitor_handles(void);

void reset_handler(void) {
    for (char *p = &__bss_start__; p < &__bss_end__;) *p++ = 0;
    initialise_monitor_handles();
    tulpa_entry();
    exit(0);
}

static void fault(void) { abort(); }

__attribute__((section(".vectors"), used)) void *const vectors[16] = {
    &__stack_top, (void *)reset_handler, (void *)fault, (void *)fault,
};
