#pragma once

#include "bobe_types.h"
#include "esp_err.h"
#include "freertos/FreeRTOS.h"

#include <stdbool.h>
#include <stddef.h>

typedef struct {
    bobe_network_state_t state;
    uint32_t generation;
} bobe_network_event_t;

typedef void (*bobe_network_callback_t)(
    const bobe_network_event_t *event,
    void *context
);

typedef struct {
    char uri[256];
    char key_id[64];
} bobe_hub_candidate_t;

typedef enum {
    BOBE_WIFI_CONFIGURED = 0,
    BOBE_WIFI_CONFIG_AUTH_FAILED,
    BOBE_WIFI_CONFIG_AP_NOT_FOUND,
    BOBE_WIFI_CONFIG_TIMEOUT,
    BOBE_WIFI_CONFIG_BUSY,
    BOBE_WIFI_CONFIG_NOT_READY,
    BOBE_WIFI_CONFIG_FAILED,
} bobe_wifi_config_result_t;

esp_err_t bobe_connectivity_init(
    bobe_network_callback_t callback,
    void *context
);
bool bobe_connectivity_is_online(void);
bool bobe_connectivity_is_provisioning(void);
void bobe_connectivity_confirm_authenticated_time(void);
uint32_t bobe_connectivity_generation(void);
int bobe_connectivity_rssi_dbm(void);
bobe_wifi_config_result_t bobe_connectivity_configure_wifi(
    const uint8_t *ssid,
    size_t ssid_length,
    const uint8_t *passphrase,
    size_t passphrase_length,
    TickType_t timeout
);
esp_err_t bobe_connectivity_open_provisioning_window(void);
esp_err_t bobe_connectivity_discover_hub(
    bobe_hub_candidate_t *candidate,
    uint32_t timeout_ms
);
