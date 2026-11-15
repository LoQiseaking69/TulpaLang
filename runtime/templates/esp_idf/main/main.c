/* Runs the Tulpa program (app.rav) in the main task. */
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "tulpa_hal.h"

void app_main(void) {
    tulpa_entry();
    for (;;) vTaskDelay(portMAX_DELAY); /* the program returned: idle */
}
