#pragma once

#include "bobe_claim.h"
#include "esp_err.h"

#include <stdint.h>

typedef struct {
    const char *endpoint;
    const char *trust_key_id;
    uint64_t controller_epoch;
    const char *server_ca_pem;
    const char *client_cert_pem;
    const char *client_key_pem;
} bobe_claim_material_t;

esp_err_t bobe_claim_store_load_state(bobe_claim_state_t *state);
esp_err_t bobe_claim_store_plan(
    uint64_t controller_epoch,
    bobe_claim_plan_t *plan
);
esp_err_t bobe_claim_store_commit(
    const bobe_claim_plan_t *plan,
    const bobe_claim_material_t *material
);
esp_err_t bobe_claim_store_retire_inactive(void);
