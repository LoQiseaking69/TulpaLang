/* Boots the board, then runs the Tulpa program (app.rav). */
#include "pico/stdlib.h"
#include "tulpa_hal.h"

int main(void) {
    stdio_init_all();
    sleep_ms(1500); /* give a USB serial monitor time to attach */
    tulpa_entry();
    for (;;) tight_loop_contents(); /* the program returned: idle */
}
