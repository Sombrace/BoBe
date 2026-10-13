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

static esp_err_t erase_key_if_present(nvs_handle_t nvs, const char *key)
{
    const esp_err_t result = nvs_erase_key(nvs, key);
    return result == ESP_ERR_NVS_NOT_FOUND ? ESP_OK : result;
}

static esp_err_t erase_namespace_keys(
    const char *namespace_name,
    const char *const *keys,
    size_t count
)
{
    nvs_handle_t nvs;
    const esp_err_t open_result =
        nvs_open(namespace_name, NVS_READWRITE, &nvs);
    if (open_result == ESP_ERR_NVS_NOT_FOUND) {
        return ESP_OK;
    }
    if (open_result != ESP_OK) {
        return open_result;
    }
    esp_err_t result = ESP_OK;
    for (size_t index = 0; index < count && result == ESP_OK; ++index) {
        result = erase_key_if_present(nvs, keys[index]);
    }
    if (result == ESP_OK) {
        result = nvs_commit(nvs);
    }
    nvs_close(nvs);
    return result;
}

esp_err_t bobe_storage_invalidate_claim(void)
{
    static const char *const keys[] = {
        BOBE_CLAIM_ACTIVE_SLOT_KEY,
        BOBE_CLAIM_READY_KEY,
    };
    return erase_namespace_keys(
        BOBE_CLAIM_NAMESPACE,
        keys,
        sizeof(keys) / sizeof(keys[0])
    );
}

esp_err_t bobe_storage_erase_claim_secrets(void)
{
    const char *claim_keys[3U * BOBE_CLAIM_SLOT_COUNT];
    const char *tls_keys[3U * BOBE_CLAIM_SLOT_COUNT];
    size_t claim_count = 0;
    size_t tls_count = 0;
    for (uint8_t slot = 0; slot < BOBE_CLAIM_SLOT_COUNT; ++slot) {
        const bobe_storage_slot_keys_t *const keys = &SLOT_KEYS[slot];
        claim_keys[claim_count++] = keys->endpoint;
        claim_keys[claim_count++] = keys->trust_key_id;
        claim_keys[claim_count++] = keys->controller_epoch;
        tls_keys[tls_count++] = keys->server_ca;
        tls_keys[tls_count++] = keys->client_cert;
        tls_keys[tls_count++] = keys->client_key;
    }

    esp_err_t result = erase_namespace_keys(
        BOBE_CLAIM_NAMESPACE,
        claim_keys,
        claim_count
    );
    if (result == ESP_OK) {
        result = erase_namespace_keys(
            BOBE_TLS_NAMESPACE,
            tls_keys,
            tls_count
        );
    }
    return result;
}
