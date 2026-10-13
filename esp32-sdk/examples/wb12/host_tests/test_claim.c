#include "bobe_claim.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define CHECK(condition)                                                     \
    do {                                                                     \
        if (!(condition)) {                                                  \
            fprintf(stderr, "check failed at %s:%d: %s\n",                 \
                    __FILE__, __LINE__, #condition);                         \
            exit(1);                                                         \
        }                                                                    \
    } while (0)

int main(void)
{
    bobe_claim_plan_t plan;
    bobe_claim_state_t state = {0};
    CHECK(bobe_claim_plan_rotation(&state, 1, &plan) == BOBE_CLAIM_PLAN_OK);
    CHECK(plan.target_slot == 0);
    CHECK(!plan.retire_slot);

    state = (bobe_claim_state_t){
        .claim_ready = true,
        .active_slot_valid = true,
        .active_slot = 0,
        .active_epoch = 7,
    };
    CHECK(bobe_claim_plan_rotation(&state, 8, &plan) == BOBE_CLAIM_PLAN_OK);
    CHECK(plan.target_slot == 1);
    CHECK(plan.retire_slot && plan.retired_slot == 0);
    CHECK(bobe_claim_plan_rotation(&state, 7, &plan) ==
          BOBE_CLAIM_PLAN_STALE_EPOCH);
    CHECK(bobe_claim_plan_rotation(&state, 6, &plan) ==
          BOBE_CLAIM_PLAN_STALE_EPOCH);

    state.active_slot = 1;
    state.active_epoch = 9;
    CHECK(bobe_claim_plan_rotation(&state, 10, &plan) == BOBE_CLAIM_PLAN_OK);
    CHECK(plan.target_slot == 0 && plan.retired_slot == 1);

    state.active_slot_valid = false;
    CHECK(bobe_claim_plan_rotation(&state, 10, &plan) ==
          BOBE_CLAIM_PLAN_INCONSISTENT_STATE);

    CHECK(strcmp(
              bobe_claim_plan_result_name(BOBE_CLAIM_PLAN_STALE_EPOCH),
              "stale_epoch"
          ) == 0);
    puts("BoBe claim lifecycle tests passed.");
    return 0;
}
