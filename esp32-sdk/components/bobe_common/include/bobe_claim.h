#pragma once

#include <stdbool.h>
#include <stdint.h>

typedef enum {
    BOBE_CLAIM_PLAN_OK = 0,
    BOBE_CLAIM_PLAN_INVALID_ARGUMENT,
    BOBE_CLAIM_PLAN_INCONSISTENT_STATE,
    BOBE_CLAIM_PLAN_STALE_EPOCH,
} bobe_claim_plan_result_t;

typedef struct {
    bool claim_ready;
    bool active_slot_valid;
    uint8_t active_slot;
    uint64_t active_epoch;
} bobe_claim_state_t;

typedef struct {
    uint8_t target_slot;
    bool retire_slot;
    uint8_t retired_slot;
} bobe_claim_plan_t;

bobe_claim_plan_result_t bobe_claim_plan_rotation(
    const bobe_claim_state_t *state,
    uint64_t new_epoch,
    bobe_claim_plan_t *plan
);

const char *bobe_claim_plan_result_name(bobe_claim_plan_result_t result);
