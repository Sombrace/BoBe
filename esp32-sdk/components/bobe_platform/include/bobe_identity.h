#pragma once

#include "esp_err.h"
#include <stddef.h>

esp_err_t bobe_identity_device_id(char *output, size_t capacity);
void bobe_identity_request_id(char output[37]);
void bobe_identity_boot_id(char output[33]);
