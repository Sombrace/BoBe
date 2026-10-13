#include "bobe_protocol.h"
#include "esp_log.h"

void app_main(void)
{
    char message[1024];
    const bobe_hello_info_t hello = {
        .device_id = "codec-example",
        .boot_id = "0123456789abcdef0123456789abcdef",
        .firmware_version = "0.2.0",
        .hardware_revision = "protocol-example",
        .capabilities = {.sensors = true},
    };
    if (bobe_protocol_build_hello(message, sizeof(message), &hello) == BOBE_PROTOCOL_OK) {
        ESP_LOGI("protocol", "%s", message);
    }
    if (bobe_protocol_build_sensor_number(message, sizeof(message), "example.value", 1, 12.5, "units") == BOBE_PROTOCOL_OK) {
        ESP_LOGI("protocol", "%s", message);
    }
    ESP_LOGI("protocol", "Codec demonstration only; no device connection or sensor hardware is claimed");
}
