#pragma once

#include "esp_err.h"

#include <stddef.h>
#include <stdint.h>

typedef struct {
    const char *username;
    const char *softap_password;
    const uint8_t *salt;
    size_t salt_length;
    const uint8_t *verifier;
    size_t verifier_length;
} bobe_provisioning_credentials_t;

esp_err_t bobe_provisioning_credentials_get(
    bobe_provisioning_credentials_t *credentials
);
