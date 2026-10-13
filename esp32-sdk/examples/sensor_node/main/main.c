#include "esp_bobe_client.h"
#include "driver/gpio.h"
#include "esp_event.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "esp_netif.h"
#include "esp_netif_sntp.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "nvs_flash.h"
#include "protocol_examples_common.h"
#include "sdkconfig.h"

#if __has_include("bobe_credentials.h")
#include "bobe_credentials.h"
#define HAVE_BOBE_CREDENTIALS 1
#else
#define HAVE_BOBE_CREDENTIALS 0
#endif

static const char *TAG = "bobe_sensor";

static void on_event(const esp_bobe_event_t *event, void *context)
{
    (void)context;
    if (event->kind == ESP_BOBE_CONNECTED) ESP_LOGI(TAG, "Authenticated sensor connection ready");
    else if (event->kind == ESP_BOBE_DISCONNECTED) ESP_LOGW(TAG, "Disconnected");
    else if (event->kind == ESP_BOBE_ERROR) ESP_LOGW(TAG, "Connection error: %s", esp_err_to_name(event->error));
    else if (event->control != NULL && event->control->kind == BOBE_CONTROL_INPUT_RESULT) {
        ESP_LOGI(TAG, "Button request: %s %s", event->control->data.input_result.status, event->control->data.input_result.code);
    }
}

void app_main(void)
{
    const esp_err_t storage = nvs_flash_init();
    if (storage != ESP_OK) {
        ESP_LOGE(TAG, "NVS unavailable: %s; stored data was preserved", esp_err_to_name(storage));
        return;
    }
    char id[BOBE_DEVICE_ID_MAX_BYTES];
    ESP_ERROR_CHECK(esp_bobe_client_device_id(id, sizeof(id)));
    ESP_LOGI(TAG, "device_id=%s", id);
    const int button_gpio = CONFIG_BOBE_EXAMPLE_BUTTON_GPIO;
#if CONFIG_BOBE_EXAMPLE_BUTTON_GPIO >= 0
    if (button_gpio >= 0) {
        if (!GPIO_IS_VALID_GPIO(button_gpio)) {
            ESP_LOGE(TAG, "Configured button GPIO is unavailable on this chip");
            return;
        }
        const gpio_config_t button = {.pin_bit_mask = UINT64_C(1) << button_gpio,
            .mode = GPIO_MODE_INPUT, .pull_up_en = GPIO_PULLUP_ENABLE};
        ESP_ERROR_CHECK(gpio_config(&button));
    }
#endif
    ESP_ERROR_CHECK(esp_netif_init());
    ESP_ERROR_CHECK(esp_event_loop_create_default());
    const esp_bobe_client_config_t config = {
#if HAVE_BOBE_CREDENTIALS
        .device_id = BOBE_PROVISIONED_DEVICE_ID,
#endif
        .hardware_model = "esp-bobe-sensor-node",
        .capabilities = {.sensors = true, .actions = button_gpio >= 0},
#if HAVE_BOBE_CREDENTIALS
        .credentials = &bobe_credentials,
#endif
        .on_event = on_event,
    };
    ESP_ERROR_CHECK(esp_bobe_client_init(&config));
    if (!esp_bobe_client_is_configured()) {
        ESP_LOGW(TAG, "Provision device %s before connecting", id);
        return;
    }
    ESP_ERROR_CHECK(example_connect());
    esp_sntp_config_t time_config = ESP_NETIF_SNTP_DEFAULT_CONFIG("pool.ntp.org");
    ESP_ERROR_CHECK(esp_netif_sntp_init(&time_config));
    if (esp_netif_sntp_sync_wait(pdMS_TO_TICKS(30000)) != ESP_OK) {
        ESP_LOGE(TAG, "TLS requires a valid clock; configure a reachable time source");
        return;
    }
    ESP_ERROR_CHECK(esp_bobe_client_start());
    uint32_t sequence = 0;
#if CONFIG_BOBE_EXAMPLE_BUTTON_GPIO >= 0
    int stable = 1, sampled = 1;
    unsigned count = 0;
#endif
    int64_t next_report = 0;
    for (;;) {
        if (esp_bobe_client_is_connected() && esp_timer_get_time() >= next_report) {
            const double uptime = (double)esp_timer_get_time() / 1000000.0;
            (void)esp_bobe_report_number("uptime", ++sequence, uptime, "s");
            (void)esp_bobe_report_number("internal_heap_free", ++sequence,
                (double)heap_caps_get_free_size(MALLOC_CAP_INTERNAL), "bytes");
            next_report = esp_timer_get_time() + INT64_C(10000000);
        }
#if CONFIG_BOBE_EXAMPLE_BUTTON_GPIO >= 0
        if (button_gpio >= 0) {
            const int level = gpio_get_level((gpio_num_t)button_gpio);
            if (level != sampled) { sampled = level; count = 0; }
            else if (count < 3) ++count;
            if (count == 3 && stable != sampled) {
                stable = sampled;
                if (stable == 0 && esp_bobe_client_is_connected()) {
                    char request_id[37];
                    esp_bobe_request_id(request_id);
                    const esp_err_t result = esp_bobe_send_action(request_id, "button.press", CONFIG_BOBE_EXAMPLE_BUTTON_LABEL);
                    if (result != ESP_OK) ESP_LOGW(TAG, "Button was not queued: %s", esp_err_to_name(result));
                }
            }
        }
#endif
        vTaskDelay(pdMS_TO_TICKS(30));
    }
}
