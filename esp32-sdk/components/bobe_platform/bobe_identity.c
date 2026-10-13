#include "bobe_identity.h"

#include "esp_random.h"
#include "nvs.h"

#include <stdint.h>
#include <stdio.h>
#include <sys/lock.h>

static _lock_t identity_lock;

void bobe_identity_request_id(char output[37])
{
    if (output == NULL) return;
    uint8_t bytes[16];
    esp_fill_random(bytes, sizeof(bytes));
    bytes[6] = (uint8_t)((bytes[6] & 0x0FU) | 0x40U);
    bytes[8] = (uint8_t)((bytes[8] & 0x3FU) | 0x80U);
    (void)snprintf(output, 37,
        "%02x%02x%02x%02x-%02x%02x-%02x%02x-%02x%02x-%02x%02x%02x%02x%02x%02x",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]);
}

void bobe_identity_boot_id(char output[33])
{
    if (output == NULL) return;
    uint8_t bytes[16];
    esp_fill_random(bytes, sizeof(bytes));
    for (size_t index = 0; index < sizeof(bytes); ++index) {
        (void)snprintf(output + 2U * index, 3, "%02x", bytes[index]);
    }
}

static esp_err_t load_device_id(char *output, size_t capacity)
{
    const char *namespaces[] = {"board", "bobe_identity"};
    const char *keys[] = {"uuid", "device_id"};
    for (size_t index = 0; index < 2; ++index) {
        nvs_handle_t storage;
        const esp_err_t opened = nvs_open(namespaces[index], NVS_READONLY, &storage);
        if (opened == ESP_ERR_NVS_NOT_FOUND) continue;
        if (opened != ESP_OK) return opened;
        size_t length = capacity;
        const esp_err_t result = nvs_get_str(storage, keys[index], output, &length);
        nvs_close(storage);
        if (result == ESP_OK && output[0] != '\0') return ESP_OK;
        if (result != ESP_ERR_NVS_NOT_FOUND) return result == ESP_OK ? ESP_ERR_INVALID_STATE : result;
    }
    nvs_handle_t storage;
    esp_err_t result = nvs_open("bobe_identity", NVS_READWRITE, &storage);
    if (result != ESP_OK) return result;
    bobe_identity_request_id(output);
    result = nvs_set_str(storage, "device_id", output);
    if (result == ESP_OK) result = nvs_commit(storage);
    nvs_close(storage);
    return result;
}

esp_err_t bobe_identity_device_id(char *output, size_t capacity)
{
    if (output == NULL || capacity < 37U) return ESP_ERR_INVALID_SIZE;
    _lock_acquire(&identity_lock);
    const esp_err_t result = load_device_id(output, capacity);
    _lock_release(&identity_lock);
    return result;
}
