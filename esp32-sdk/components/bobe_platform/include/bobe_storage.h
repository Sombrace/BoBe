#pragma once

#include "esp_err.h"

#include <stdint.h>

#define BOBE_CLAIM_NAMESPACE "bobe_net"
#define BOBE_TLS_NAMESPACE "bobe_tls"
#define BOBE_SETTINGS_NAMESPACE "bobe_settings"

#define BOBE_MICROPHONE_MUTED_KEY "mic_muted"
#define BOBE_VOLUME_KEY "volume"
#define BOBE_FACE_THEME_KEY "face_theme"

#define BOBE_CLAIM_READY_KEY "claim_ready"
#define BOBE_CLAIM_ACTIVE_SLOT_KEY "active_slot"
#define BOBE_CLAIM_SLOT_COUNT 2U

#define BOBE_PEM_STORAGE_MAX_BYTES 4096U

typedef struct {
    const char *endpoint;
    const char *trust_key_id;
    const char *controller_epoch;
    const char *server_ca;
    const char *client_cert;
    const char *client_key;
} bobe_storage_slot_keys_t;

const bobe_storage_slot_keys_t *bobe_storage_slot_keys(uint8_t slot);
esp_err_t bobe_storage_load_active_slot(uint8_t *slot);
