#include "bobe_storage.h"

#include "nvs.h"

static const bobe_storage_slot_keys_t SLOT_KEYS[BOBE_CLAIM_SLOT_COUNT] = {
    {
        .endpoint = "endpoint_0",
        .trust_key_id = "trust_kid_0",
        .controller_epoch = "ctrl_epoch_0",
        .server_ca = "server_ca_0",
        .client_cert = "client_cert_0",
        .client_key = "client_key_0",
    },
    {
        .endpoint = "endpoint_1",
        .trust_key_id = "trust_kid_1",
        .controller_epoch = "ctrl_epoch_1",
        .server_ca = "server_ca_1",
        .client_cert = "client_cert_1",
        .client_key = "client_key_1",
    },
};

const bobe_storage_slot_keys_t *bobe_storage_slot_keys(uint8_t slot)
{
    return slot < BOBE_CLAIM_SLOT_COUNT ? &SLOT_KEYS[slot] : NULL;
}

esp_err_t bobe_storage_load_active_slot(uint8_t *slot)
{
    if (slot == NULL) {
        return ESP_ERR_INVALID_ARG;
    }
    nvs_handle_t nvs;
    const esp_err_t open_result =
        nvs_open(BOBE_CLAIM_NAMESPACE, NVS_READONLY, &nvs);
    if (open_result != ESP_OK) {
        return open_result;
    }
    const esp_err_t result =
        nvs_get_u8(nvs, BOBE_CLAIM_ACTIVE_SLOT_KEY, slot);
    nvs_close(nvs);
    return result == ESP_OK && *slot >= BOBE_CLAIM_SLOT_COUNT
               ? ESP_ERR_INVALID_STATE
               : result;
}
