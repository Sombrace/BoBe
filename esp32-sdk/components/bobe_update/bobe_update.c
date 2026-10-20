#include "bobe_device_config.h"
#include "bobe_update.h"

#include "bobe_storage.h"
#include "esp_app_format.h"
#include "esp_heap_caps.h"
#include "esp_http_client.h"
#include "esp_ota_ops.h"
#include "nvs.h"
#include "psa/crypto.h"

#include <stdlib.h>
#include <string.h>

#define BOBE_UPDATE_BUFFER_BYTES 4096U

static char *load_server_ca(void)
{
    uint8_t active_slot = 0;
    const esp_err_t slot_result = bobe_storage_load_active_slot(&active_slot);
    if (slot_result != ESP_OK) {
        return NULL;
    }
    const bobe_storage_slot_keys_t *const keys =
        bobe_storage_slot_keys(active_slot);
    if (keys == NULL) {
        return NULL;
    }
    const char *const key = keys->server_ca;
    nvs_handle_t nvs;
    if (nvs_open(BOBE_TLS_NAMESPACE, NVS_READONLY, &nvs) != ESP_OK) {
        return NULL;
    }
    size_t length = 0;
    if (nvs_get_str(nvs, key, NULL, &length) != ESP_OK ||
        length < 2U || length > BOBE_PEM_STORAGE_MAX_BYTES) {
        nvs_close(nvs);
        return NULL;
    }
    char *const value = heap_caps_malloc(
        length,
        MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT
    );
    if (value == NULL || nvs_get_str(nvs, key, value, &length) != ESP_OK) {
        free(value);
        nvs_close(nvs);
        return NULL;
    }
    nvs_close(nvs);
    return value;
}

esp_err_t bobe_update_install(const bobe_update_manifest_t *manifest)
{
    if (manifest == NULL || manifest->image_size == 0U ||
        manifest->image_size > BOBE_UPDATE_IMAGE_SIZE_MAX ||
        strncmp(manifest->url, "https://", 8U) != 0) {
        return ESP_ERR_INVALID_ARG;
    }
    char *const server_ca = load_server_ca();
    if (server_ca == NULL) {
        return ESP_ERR_INVALID_STATE;
    }
    const esp_http_client_config_t config = {
        .url = manifest->url,
        .cert_pem = server_ca,
        .timeout_ms = 15000,
        .buffer_size = BOBE_UPDATE_BUFFER_BYTES,
        .keep_alive_enable = true,
    };
    esp_http_client_handle_t client = esp_http_client_init(&config);
    if (client == NULL) {
        free(server_ca);
        return ESP_ERR_NO_MEM;
    }
    esp_err_t result = esp_http_client_open(client, 0);
    if (result != ESP_OK) {
        goto cleanup_client;
    }
    const int64_t content_length = esp_http_client_fetch_headers(client);
    if (content_length != manifest->image_size ||
        esp_http_client_get_status_code(client) != 200) {
        result = ESP_ERR_INVALID_SIZE;
        goto cleanup_client;
    }

    const esp_partition_t *const running = esp_ota_get_running_partition();
    const esp_partition_t *const target = esp_ota_get_next_update_partition(NULL);
    if (running == NULL || target == NULL || target == running ||
        target->size < manifest->image_size) {
        result = ESP_ERR_INVALID_STATE;
        goto cleanup_client;
    }
    esp_ota_handle_t ota = 0;
    result = esp_ota_begin(
        target,
        manifest->image_size,
        &ota
    );
    if (result != ESP_OK) {
        goto cleanup_client;
    }

    psa_hash_operation_t hash = PSA_HASH_OPERATION_INIT;
    if (psa_hash_setup(&hash, PSA_ALG_SHA_256) != PSA_SUCCESS) {
        result = ESP_FAIL;
        esp_ota_abort(ota);
        goto cleanup_client;
    }
    unsigned char *const buffer = heap_caps_malloc(
        BOBE_UPDATE_BUFFER_BYTES,
        MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT
    );
    if (buffer == NULL) {
        result = ESP_ERR_NO_MEM;
        psa_hash_abort(&hash);
        esp_ota_abort(ota);
        goto cleanup_client;
    }
    uint32_t written = 0;
    while (written < manifest->image_size) {
        const int count = esp_http_client_read(
            client,
            (char *)buffer,
            BOBE_UPDATE_BUFFER_BYTES
        );
        if (count <= 0 || (uint32_t)count > manifest->image_size - written) {
            result = ESP_ERR_INVALID_SIZE;
            break;
        }
        if (psa_hash_update(&hash, buffer, (size_t)count) != PSA_SUCCESS) {
            result = ESP_FAIL;
            break;
        }
        result = esp_ota_write(ota, buffer, (size_t)count);
        if (result != ESP_OK) {
            break;
        }
        written += (uint32_t)count;
    }
    free(buffer);

    unsigned char digest[BOBE_UPDATE_SHA256_BYTES];
    size_t digest_length = 0;
    if (result == ESP_OK && written == manifest->image_size &&
        psa_hash_finish(
            &hash,
            digest,
            sizeof(digest),
            &digest_length
        ) == PSA_SUCCESS &&
        digest_length == sizeof(digest) &&
        memcmp(digest, manifest->image_sha256, sizeof(digest)) == 0) {
        result = esp_ota_end(ota);
        if (result == ESP_OK) {
            result = esp_ota_set_boot_partition_skip_validate(target);
        }
    } else {
        psa_hash_abort(&hash);
        if (result == ESP_OK) {
            result = ESP_ERR_INVALID_CRC;
        }
    }
    if (result != ESP_OK) {
        esp_ota_abort(ota);
    }

cleanup_client:
    esp_http_client_close(client);
    esp_http_client_cleanup(client);
    free(server_ca);
    return result;
}
