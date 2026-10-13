#include "bobe_claim.h"

#include <stddef.h>

bobe_claim_plan_result_t bobe_claim_plan_rotation(
    const bobe_claim_state_t *state,
    uint64_t new_epoch,
    bobe_claim_plan_t *plan
)
{
    if (state == NULL || plan == NULL || new_epoch == 0) {
        return BOBE_CLAIM_PLAN_INVALID_ARGUMENT;
    }
    if (state->claim_ready &&
        (!state->active_slot_valid || state->active_slot > 1U ||
         state->active_epoch == 0)) {
        return BOBE_CLAIM_PLAN_INCONSISTENT_STATE;
    }
    if (!state->claim_ready) {
        *plan = (bobe_claim_plan_t){
            .target_slot = 0,
        };
        return BOBE_CLAIM_PLAN_OK;
    }
    if (new_epoch <= state->active_epoch) {
        return BOBE_CLAIM_PLAN_STALE_EPOCH;
    }
    *plan = (bobe_claim_plan_t){
        .target_slot = (uint8_t)(state->active_slot ^ 1U),
        .retire_slot = true,
        .retired_slot = state->active_slot,
    };
    return BOBE_CLAIM_PLAN_OK;
}

const char *bobe_claim_plan_result_name(bobe_claim_plan_result_t result)
{
    static const char *const names[] = {
        "ok",
        "invalid_argument",
        "inconsistent_state",
        "stale_epoch",
    };
    return result >= BOBE_CLAIM_PLAN_OK &&
                   (size_t)result < sizeof(names) / sizeof(names[0])
               ? names[result]
               : "unknown";
}
