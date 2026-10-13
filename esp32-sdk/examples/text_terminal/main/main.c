#include "esp_bobe_client.h"
#include "esp_event.h"
#include "esp_log.h"
#include "esp_netif.h"
#include "esp_netif_sntp.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "nvs_flash.h"
#include "protocol_examples_common.h"

#include <stdio.h>
#include <string.h>

#if __has_include("bobe_credentials.h")
#include "bobe_credentials.h"
#define HAVE_BOBE_CREDENTIALS 1
#else
#define HAVE_BOBE_CREDENTIALS 0
#endif

static const char *TAG = "bobe_terminal";

static void on_event(const esp_bobe_event_t *event, void *context)
{
    (void)context;
    if (event->kind == ESP_BOBE_CONNECTED) {
        ESP_LOGI(TAG, "Connected. Enter a message, /action ID LABEL, or /sensor NAME VALUE UNIT");
    } else if (event->kind == ESP_BOBE_DISCONNECTED) {
        ESP_LOGW(TAG, "Disconnected; queued user input is not replayed automatically");
    } else if (event->kind == ESP_BOBE_ERROR) {
        ESP_LOGW(TAG, "Connection error: %s", esp_err_to_name(event->error));
    } else if (event->kind == ESP_BOBE_CONTROL && event->control != NULL) {
        const bobe_control_message_t *message = event->control;
        if (message->kind == BOBE_CONTROL_DISPLAY_TEXT) {
            if (message->data.display_text.clear) fputs("\nBoBe: ", stdout);
            for (const unsigned char *p = (const unsigned char *)message->data.display_text.text; *p != 0; ++p) {
                if (*p >= 32U || *p == '\n' || *p == '\t') fputc(*p, stdout);
            }
            if (message->data.display_text.final) fputc('\n', stdout);
            fflush(stdout);
        } else if (message->kind == BOBE_CONTROL_INPUT_RESULT) {
            ESP_LOGI(TAG, "Request %.8s: %s %s", message->data.input_result.request_id,
                     message->data.input_result.status, message->data.input_result.code);
        } else if (message->kind == BOBE_CONTROL_SENSOR_ACK) {
            ESP_LOGI(TAG, "Sensor %s acknowledged at sequence %lu", message->data.sensor_ack.name,
                     (unsigned long)message->data.sensor_ack.sequence);
        }
    }
}

void app_main(void)
{
    const esp_err_t storage = nvs_flash_init();
    if (storage != ESP_OK) {
        ESP_LOGE(TAG, "NVS initialization failed: %s; application data was preserved", esp_err_to_name(storage));
        return;
    }
    char device_id[BOBE_DEVICE_ID_MAX_BYTES];
    if (esp_bobe_client_device_id(device_id, sizeof(device_id)) != ESP_OK) return;
    ESP_LOGI(TAG, "device_id=%s", device_id);
#if HAVE_BOBE_CREDENTIALS
    const esp_bobe_credentials_t *credentials = &bobe_credentials;
#else
    const esp_bobe_credentials_t *credentials = NULL;
    ESP_LOGI(TAG, "Using an existing provisioned NVS claim; see the README to provision a new device");
#endif
    ESP_ERROR_CHECK(esp_netif_init());
    ESP_ERROR_CHECK(esp_event_loop_create_default());
    const esp_bobe_client_config_t config = {
#if HAVE_BOBE_CREDENTIALS
        .device_id = BOBE_PROVISIONED_DEVICE_ID,
#endif
        .hardware_model = "esp-bobe-text-terminal",
        .capabilities = {.text_input = true, .text_output = true, .actions = true, .sensors = true},
        .credentials = credentials,
        .on_event = on_event,
    };
    ESP_ERROR_CHECK(esp_bobe_client_init(&config));
    if (!esp_bobe_client_is_configured()) {
        ESP_LOGW(TAG, "No endpoint credentials. Provision device %s before connecting", device_id);
        return;
    }
    ESP_ERROR_CHECK(example_connect());
    esp_sntp_config_t time_config = ESP_NETIF_SNTP_DEFAULT_CONFIG("pool.ntp.org");
    ESP_ERROR_CHECK(esp_netif_sntp_init(&time_config));
    if (esp_netif_sntp_sync_wait(pdMS_TO_TICKS(30000)) != ESP_OK) {
        ESP_LOGE(TAG, "Time synchronization failed; TLS certificate validation requires a valid clock");
        return;
    }
    const esp_err_t started = esp_bobe_client_start();
    if (started != ESP_OK) {
        ESP_LOGE(TAG, "Client start failed: %s; provision the endpoint and rebuild", esp_err_to_name(started));
        return;
    }
    char line[BOBE_TEXT_INPUT_MAX_BYTES + 2U];
    uint32_t sensor_sequence = 0;
    for (;;) {
        if (fgets(line, sizeof(line), stdin) == NULL) {
            clearerr(stdin);
            vTaskDelay(pdMS_TO_TICKS(20));
            continue;
        }
        const size_t end = strcspn(line, "\r\n");
        if (end == sizeof(line) - 1U) {
            int byte;
            do { byte = getchar(); } while (byte != '\n' && byte != EOF);
            ESP_LOGW(TAG, "Input exceeds %u bytes", (unsigned)BOBE_TEXT_INPUT_MAX_BYTES);
            continue;
        }
        line[end] = '\0';
        if (end == 0) continue;
        char request_id[37];
        esp_bobe_request_id(request_id);
        esp_err_t result;
        if (strncmp(line, "/action ", 8) == 0) {
            char action[BOBE_ACTION_ID_MAX_BYTES + 1U], label[BOBE_ACTION_LABEL_MAX_BYTES + 1U];
            if (sscanf(line + 8, "%64s %128[^\n]", action, label) != 2) {
                ESP_LOGW(TAG, "Usage: /action ID LABEL");
                continue;
            }
            result = esp_bobe_send_action(request_id, action, label);
        } else if (strncmp(line, "/sensor ", 8) == 0) {
            char name[BOBE_SENSOR_NAME_MAX_BYTES + 1U], unit[BOBE_SENSOR_UNIT_MAX_BYTES + 1U];
            double value;
            if (sscanf(line + 8, "%32s %lf %16s", name, &value, unit) != 3) {
                ESP_LOGW(TAG, "Usage: /sensor NAME VALUE UNIT (manual sample)");
                continue;
            }
            result = esp_bobe_report_number(name, ++sensor_sequence, value, unit);
        } else {
            result = esp_bobe_send_text(request_id, line);
        }
        if (result != ESP_OK) ESP_LOGW(TAG, "Input was not queued: %s", esp_err_to_name(result));
        else ESP_LOGI(TAG, "Queued; awaiting device acknowledgement");
    }
}
