#include "bobe_claim_store.h"

#include "bobe_storage.h"
#include "esp_check.h"
#include "esp_log.h"
#include "nvs.h"

#include <stdbool.h>
#include <stddef.h>
#include <string.h>

static const char *TAG = "bobe_claim";

esp_err_t bobe_claim_store_load_state(bobe_claim_state_t *state)
{
    if (state == NULL) {
        return ESP_ERR_INVALID_ARG;
    }
    memset(state, 0, sizeof(*state));
    nvs_handle_t nvs;
    const esp_err_t open_result =
        nvs_open(BOBE_CLAIM_NAMESPACE, NVS_READONLY, &nvs);
    if (open_result == ESP_ERR_NVS_NOT_FOUND) {
        return ESP_OK;
    }
    if (open_result != ESP_OK) {
        return open_result;
    }
    uint8_t ready = 0;
    const esp_err_t ready_result =
        nvs_get_u8(nvs, BOBE_CLAIM_READY_KEY, &ready);
    nvs_close(nvs);
    if (ready_result == ESP_ERR_NVS_NOT_FOUND || ready != 1) {
        return ready_result == ESP_ERR_NVS_NOT_FOUND ? ESP_OK : ESP_ERR_INVALID_STATE;
    }
    state->claim_ready = true;
    uint8_t slot = 0;
    const esp_err_t slot_result = bobe_storage_load_active_slot(&slot);
    if (slot_result != ESP_OK || bobe_storage_slot_keys(slot) == NULL) {
        return ESP_ERR_INVALID_STATE;
    }
    state->active_slot_valid = true;
    state->active_slot = slot;
    const bobe_storage_slot_keys_t *const keys = bobe_storage_slot_keys(slot);
    if (nvs_open(BOBE_CLAIM_NAMESPACE, NVS_READONLY, &nvs) != ESP_OK) {
        return ESP_ERR_INVALID_STATE;
    }
    const esp_err_t epoch_result = nvs_get_u64(
        nvs,
        keys->controller_epoch,
        &state->active_epoch
    );
    nvs_close(nvs);
    return epoch_result;
}

static esp_err_t erase_keys(
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
        result = nvs_erase_key(nvs, keys[index]);
        if (result == ESP_ERR_NVS_NOT_FOUND) {
            result = ESP_OK;
        }
    }
    if (result == ESP_OK) {
        result = nvs_commit(nvs);
    }
    nvs_close(nvs);
    return result;
}

static esp_err_t retire_claim_material(const bobe_claim_plan_t *plan)
{
    if (!plan->retire_slot) {
        return ESP_OK;
    }
    const bobe_storage_slot_keys_t *const retired =
        bobe_storage_slot_keys(plan->retired_slot);
    if (retired == NULL) {
        return ESP_ERR_INVALID_STATE;
    }
    const char *const claim_keys[] = {
        retired->endpoint,
        retired->trust_key_id,
        retired->controller_epoch,
    };
    ESP_RETURN_ON_ERROR(
        erase_keys(
            BOBE_CLAIM_NAMESPACE,
            claim_keys,
            sizeof(claim_keys) / sizeof(claim_keys[0])
        ),
        TAG,
        "retire previous claim slot"
    );
    const char *const tls_keys[] = {
        retired->server_ca,
        retired->client_cert,
        retired->client_key,
    };
    ESP_RETURN_ON_ERROR(
        erase_keys(
            BOBE_TLS_NAMESPACE,
            tls_keys,
            sizeof(tls_keys) / sizeof(tls_keys[0])
        ),
        TAG,
        "retire previous TLS slot"
    );
    return ESP_OK;
}

static esp_err_t save_claim_slot(
    const bobe_storage_slot_keys_t *keys,
    const bobe_claim_material_t *material
)
{
    nvs_handle_t nvs;
    ESP_RETURN_ON_ERROR(
        nvs_open(BOBE_CLAIM_NAMESPACE, NVS_READWRITE, &nvs),
        TAG,
        "open claim storage"
    );
    esp_err_t result = nvs_set_str(nvs, keys->endpoint, material->endpoint);
    if (result == ESP_OK) {
        result = nvs_set_str(nvs, keys->trust_key_id, material->trust_key_id);
    }
    if (result == ESP_OK) {
        result = nvs_set_u64(
            nvs,
            keys->controller_epoch,
            material->controller_epoch
        );
    }
    if (result == ESP_OK) {
        result = nvs_commit(nvs);
    }
    nvs_close(nvs);
    return result;
}

static esp_err_t save_tls_credentials(
    const bobe_storage_slot_keys_t *keys,
    const bobe_claim_material_t *material
)
{
    nvs_handle_t nvs;
    ESP_RETURN_ON_ERROR(
        nvs_open(BOBE_TLS_NAMESPACE, NVS_READWRITE, &nvs),
        TAG,
        "open TLS storage"
    );
    esp_err_t result = nvs_set_str(nvs, keys->server_ca, material->server_ca_pem);
    if (result == ESP_OK) {
        result = nvs_set_str(nvs, keys->client_cert, material->client_cert_pem);
    }
    if (result == ESP_OK) {
        result = nvs_set_str(nvs, keys->client_key, material->client_key_pem);
    }
    if (result == ESP_OK) {
        result = nvs_commit(nvs);
    }
    nvs_close(nvs);
    return result;
}

static esp_err_t activate_claim_slot(uint8_t slot)
{
    nvs_handle_t nvs;
    ESP_RETURN_ON_ERROR(
        nvs_open(BOBE_CLAIM_NAMESPACE, NVS_READWRITE, &nvs),
        TAG,
        "open claim activation storage"
    );
    esp_err_t result =
        nvs_set_u8(nvs, BOBE_CLAIM_ACTIVE_SLOT_KEY, slot);
    if (result == ESP_OK) {
        result = nvs_set_u8(nvs, BOBE_CLAIM_READY_KEY, 1);
    }
    if (result == ESP_OK) {
        result = nvs_commit(nvs);
    }
    nvs_close(nvs);
    return result;
}

static bool claim_slot_is_active(uint8_t slot, uint64_t controller_epoch)
{
    bobe_claim_state_t state;
    return bobe_claim_store_load_state(&state) == ESP_OK &&
           state.claim_ready && state.active_slot_valid &&
           state.active_slot == slot &&
           state.active_epoch == controller_epoch;
}

esp_err_t bobe_claim_store_plan(
    uint64_t controller_epoch,
    bobe_claim_plan_t *plan
)
{
    if (plan == NULL) {
        return ESP_ERR_INVALID_ARG;
    }
    bobe_claim_state_t state;
    ESP_RETURN_ON_ERROR(
        bobe_claim_store_load_state(&state),
        TAG,
        "load claim state"
    );
    const bobe_claim_plan_result_t result =
        bobe_claim_plan_rotation(&state, controller_epoch, plan);
    if (result == BOBE_CLAIM_PLAN_OK) {
        return ESP_OK;
    }
    ESP_LOGW(
        TAG,
        "claim rotation rejected: %s",
        bobe_claim_plan_result_name(result)
    );
    return result == BOBE_CLAIM_PLAN_STALE_EPOCH ? ESP_ERR_INVALID_VERSION
                                                 : ESP_ERR_INVALID_STATE;
}

esp_err_t bobe_claim_store_commit(
    const bobe_claim_plan_t *plan,
    const bobe_claim_material_t *material
)
{
    if (plan == NULL || material == NULL) {
        return ESP_ERR_INVALID_ARG;
    }
    const bobe_storage_slot_keys_t *const keys =
        bobe_storage_slot_keys(plan->target_slot);
    if (keys == NULL) {
        return ESP_ERR_INVALID_ARG;
    }
    esp_err_t result = save_tls_credentials(keys, material);
    if (result == ESP_OK) {
        result = save_claim_slot(keys, material);
    }
    if (result == ESP_OK) {
        result = activate_claim_slot(plan->target_slot);
        if (result != ESP_OK &&
            claim_slot_is_active(
                plan->target_slot,
                material->controller_epoch
            )) {
            ESP_LOGW(
                TAG,
                "claim slot %u is active although activation reported %s",
                (unsigned int)plan->target_slot,
                esp_err_to_name(result)
            );
            result = ESP_OK;
        }
    }
    if (result != ESP_OK) {
        return result;
    }
    const esp_err_t retirement = retire_claim_material(plan);
    if (retirement != ESP_OK) {
        ESP_LOGW(
            TAG,
            "previous claim slot retirement deferred: %s",
            esp_err_to_name(retirement)
        );
    }
    return ESP_OK;
}

esp_err_t bobe_claim_store_retire_inactive(void)
{
    bobe_claim_state_t state;
    ESP_RETURN_ON_ERROR(
        bobe_claim_store_load_state(&state),
        TAG,
        "load claim state for retirement"
    );
    if (!state.claim_ready) {
        return ESP_OK;
    }
    const bobe_claim_plan_t plan = {
        .retire_slot = true,
        .retired_slot = (uint8_t)(state.active_slot ^ 1U),
    };
    return retire_claim_material(&plan);
}
