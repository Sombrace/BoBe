#pragma once

#include "esp_err.h"

#include <stdbool.h>

typedef enum {
    BOBE_PROVISIONING_COMPLETED = 0,
    BOBE_PROVISIONING_ABORTED,
} bobe_provisioning_outcome_t;

typedef void (*bobe_provisioning_finished_callback_t)(
    bobe_provisioning_outcome_t outcome,
    void *context
);

esp_err_t bobe_provisioning_start(
    bobe_provisioning_finished_callback_t callback,
    void *context
);
bool bobe_provisioning_is_running(void);
bool bobe_provisioning_has_complete_claim(void);
esp_err_t bobe_provisioning_stop(void);
