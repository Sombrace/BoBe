#pragma once

#include "bobe_update_manifest.h"
#include "esp_err.h"

esp_err_t bobe_update_install(const bobe_update_manifest_t *manifest);
