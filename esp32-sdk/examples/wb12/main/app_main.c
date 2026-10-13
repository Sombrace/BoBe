#include "esp_bobe_sdk.h"
#include "esp_log.h"

void app_main(void)
{
    const esp_err_t result = esp_bobe_start();
    if (result != ESP_OK) {
        ESP_LOGE("wb12", "BoBe startup failed: %s", esp_err_to_name(result));
    }
}
