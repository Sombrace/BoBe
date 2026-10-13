#include "bobe_device_config.h"
#include "bobe_usb_onboarding.h"

#include "bobe_connectivity.h"
#include "bobe_task_config.h"
#include "bobe_usb_onboarding_protocol.h"
#include "driver/usb_serial_jtag.h"
#include "driver/usb_serial_jtag_vfs.h"
#include "esp_check.h"
#include "esp_mac.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

#include <stdio.h>
#include <string.h>
#include <unistd.h>

static void write_response(const char *response)
{
    (void)write(STDOUT_FILENO, response, strlen(response));
}

static const char *parse_error(bobe_usb_parse_result_t result)
{
    switch (result) {
    case BOBE_USB_PARSE_SSID_LENGTH:
        return "SSID_LENGTH";
    case BOBE_USB_PARSE_PASSPHRASE_LENGTH:
        return "PASSPHRASE_LENGTH";
    case BOBE_USB_PARSE_MALFORMED:
    default:
        return "MALFORMED";
    }
}

static const char *configure_error(bobe_wifi_config_result_t result)
{
    switch (result) {
    case BOBE_WIFI_CONFIG_AUTH_FAILED:
        return "AUTH";
    case BOBE_WIFI_CONFIG_AP_NOT_FOUND:
        return "NO_AP";
    case BOBE_WIFI_CONFIG_TIMEOUT:
        return "TIMEOUT";
    case BOBE_WIFI_CONFIG_BUSY:
        return "BUSY";
    case BOBE_WIFI_CONFIG_NOT_READY:
        return "NOT_READY";
    case BOBE_WIFI_CONFIG_FAILED:
    default:
        return "WIFI_CONFIG";
    }
}

static void handle_line(const char *line, size_t length)
{
    bobe_usb_request_t request;
    const bobe_usb_parse_result_t parsed =
        bobe_usb_onboarding_parse(line, length, &request);
    if (parsed == BOBE_USB_PARSE_IGNORED) {
        return;
    }

    char response[160];
    if (parsed != BOBE_USB_PARSE_OK) {
        const int written = snprintf(
            response,
            sizeof(response),
            "@BOBE-WIFI/1 ERR - %s\n",
            parse_error(parsed)
        );
        if (written > 0 && (size_t)written < sizeof(response)) {
            write_response(response);
        }
        return;
    }

    if (request.kind == BOBE_USB_REQUEST_INFO) {
        uint8_t mac[6];
        if (esp_read_mac(mac, ESP_MAC_WIFI_STA) != ESP_OK) {
            return;
        }
        const int written = snprintf(
            response,
            sizeof(response),
            "@BOBE-WIFI/1 ID %s %02x%02x%02x%02x%02x%02x %s\n",
            request.nonce,
            mac[0],
            mac[1],
            mac[2],
            mac[3],
            mac[4],
            mac[5],
            BOBE_DEVICE_LABEL
        );
        if (written > 0 && (size_t)written < sizeof(response)) {
            write_response(response);
        }
        return;
    }

    const int acknowledged = snprintf(
        response,
        sizeof(response),
        "@BOBE-WIFI/1 ACK %s\n",
        request.nonce
    );
    if (acknowledged > 0 && (size_t)acknowledged < sizeof(response)) {
        write_response(response);
    }
    const bobe_wifi_config_result_t result =
        bobe_connectivity_configure_wifi(
            request.ssid,
            request.ssid_length,
            request.passphrase,
            request.passphrase_length,
            pdMS_TO_TICKS(30000)
        );
    const int written =
        result == BOBE_WIFI_CONFIGURED
            ? snprintf(
                  response,
                  sizeof(response),
                  "@BOBE-WIFI/1 OK %s\n",
                  request.nonce
              )
            : snprintf(
                  response,
                  sizeof(response),
                  "@BOBE-WIFI/1 ERR %s %s\n",
                  request.nonce,
                  configure_error(result)
              );
    if (written > 0 && (size_t)written < sizeof(response)) {
        write_response(response);
    }
    memset(&request, 0, sizeof(request));
}

static void onboarding_task(void *unused)
{
    (void)unused;
    char line[BOBE_USB_ONBOARDING_LINE_MAX + 1U];
    size_t used = 0;
    bool overflow = false;
    uint8_t input[64];
    for (;;) {
        const int count = usb_serial_jtag_read_bytes(
            input,
            sizeof(input),
            pdMS_TO_TICKS(250)
        );
        for (int index = 0; index < count; ++index) {
            const char character = (char)input[index];
            if (character == '\n') {
                if (!overflow && used > 0) {
                    handle_line(line, used);
                }
                used = 0;
                overflow = false;
            } else if (!overflow) {
                if (used < BOBE_USB_ONBOARDING_LINE_MAX) {
                    line[used++] = character;
                } else {
                    overflow = true;
                }
            }
        }
    }
}

esp_err_t bobe_usb_onboarding_init(void)
{
    usb_serial_jtag_driver_config_t config = {
        .tx_buffer_size = 2048,
        .rx_buffer_size = 512,
    };
    ESP_RETURN_ON_ERROR(
        usb_serial_jtag_driver_install(&config),
        "bobe_usb",
        "install USB serial driver"
    );
    usb_serial_jtag_vfs_use_driver();
    return xTaskCreatePinnedToCore(
               onboarding_task,
               "bobe_usb_setup",
               6144,
               NULL,
               BOBE_TASK_PRIORITY_ONBOARDING,
               NULL,
               BOBE_NETWORK_CORE
           ) == pdPASS
               ? ESP_OK
               : ESP_ERR_NO_MEM;
}
