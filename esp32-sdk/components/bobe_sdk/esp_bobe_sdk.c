#include "bobe_device_config.h"
#include "esp_bobe_sdk.h"
#include "bobe_board.h"
#include "bobe_core.h"
#include "bobe_task_config.h"
#include "bobe_usb_onboarding.h"
#include "esp_err.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "sdkconfig.h"

#include <stdatomic.h>

#if !defined(CONFIG_SPIRAM) || defined(CONFIG_FREERTOS_UNICORE)
#error "The BoBe voice runtime requires dual-core ESP32-S3 with PSRAM"
#endif

static const char *TAG = "esp_bobe_sdk";
static atomic_bool started;

static void start_body(void *unused)
{
    (void)unused;
    const esp_err_t result = bobe_core_start();
    if (result == ESP_OK) {
        const esp_err_t onboarding = bobe_usb_onboarding_init();
        if (onboarding != ESP_OK) {
            ESP_LOGW(
                TAG,
                "USB onboarding unavailable: %s",
                esp_err_to_name(onboarding)
            );
        }
        ESP_LOGI(TAG, "BoBe body startup completed on CPU%d", xPortGetCoreID());
        vTaskDelete(NULL);
        return;
    }

    ESP_LOGE(
        TAG,
        "BoBe startup stopped in diagnostic mode: %s",
        esp_err_to_name(result)
    );
    for (;;) {
        vTaskDelay(pdMS_TO_TICKS(5000));
    }
}

esp_err_t esp_bobe_start(void)
{
    if (atomic_exchange(&started, true)) {
        return ESP_ERR_INVALID_STATE;
    }
    const esp_err_t result = bobe_board_early_init();
    if (result != ESP_OK) {
        ESP_LOGE(TAG, "Board early initialization failed: %s", esp_err_to_name(result));
        atomic_store(&started, false);
        return result;
    }
    ESP_LOGI(TAG, "Starting BoBe %s body", BOBE_DEVICE_LABEL);
    if (xTaskCreatePinnedToCore(
            start_body,
            "bobe_start",
            8192,
            NULL,
            BOBE_TASK_PRIORITY_STARTUP,
            NULL,
            BOBE_REALTIME_CORE
        ) != pdPASS) {
        ESP_LOGE(TAG, "could not create startup task");
        atomic_store(&started, false);
        return ESP_ERR_NO_MEM;
    }
    return ESP_OK;
}
