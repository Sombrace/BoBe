#include "bobe_power.h"

#include <stdio.h>
#include <stdlib.h>

#define CHECK(c) do { if (!(c)) { fprintf(stderr, "check failed at %s:%d: %s\n", __FILE__, __LINE__, #c); exit(1); } } while (0)

static bobe_power_plan_t plan_for(
    bobe_lifecycle_state_t lifecycle,
    bobe_turn_state_t turn,
    uint64_t idle_us,
    bool microphone,
    bool notice
)
{
    bobe_power_plan_t plan;
    const bobe_power_input_t input = {
        .lifecycle = lifecycle,
        .turn = turn,
        .now_us = idle_us,
        .last_interaction_us = 0,
        .microphone_active = microphone,
        .notice_active = notice,
    };
    bobe_power_plan(&input, &plan);
    return plan;
}

int main(void)
{
    bobe_power_plan_t plan = plan_for(
        BOBE_LIFECYCLE_ONLINE, BOBE_TURN_IDLE,
        BOBE_POWER_IDLE_AFTER_US - 1U, false, false
    );
    CHECK(plan.backlight_percent == BOBE_POWER_ACTIVE_BACKLIGHT);
    CHECK(plan.frame_rate == BOBE_POWER_ACTIVE_FPS);
    plan = plan_for(
        BOBE_LIFECYCLE_ONLINE, BOBE_TURN_IDLE,
        BOBE_POWER_IDLE_AFTER_US, false, false
    );
    CHECK(plan.backlight_percent == BOBE_POWER_IDLE_BACKLIGHT);
    CHECK(plan.frame_rate == BOBE_POWER_IDLE_FPS);
    for (int turn = BOBE_TURN_REQUESTING_CAPTURE;
         turn <= BOBE_TURN_ABORTED; ++turn) {
        plan = plan_for(
            BOBE_LIFECYCLE_ONLINE, (bobe_turn_state_t)turn,
            BOBE_POWER_IDLE_AFTER_US * 2U, false, false
        );
        CHECK(plan.backlight_percent == BOBE_POWER_ACTIVE_BACKLIGHT);
        CHECK(plan.frame_rate == BOBE_POWER_ACTIVE_FPS);
    }
    plan = plan_for(
        BOBE_LIFECYCLE_ONLINE, BOBE_TURN_IDLE,
        BOBE_POWER_IDLE_AFTER_US * 2U, true, false
    );
    CHECK(plan.frame_rate == BOBE_POWER_ACTIVE_FPS);
    plan = plan_for(
        BOBE_LIFECYCLE_ONLINE, BOBE_TURN_IDLE,
        BOBE_POWER_IDLE_AFTER_US * 2U, false, true
    );
    CHECK(plan.frame_rate == BOBE_POWER_ACTIVE_FPS);
    plan = plan_for(
        BOBE_LIFECYCLE_RECONNECTING, BOBE_TURN_IDLE,
        BOBE_POWER_IDLE_AFTER_US, false, false
    );
    CHECK(plan.backlight_percent == BOBE_POWER_STATUS_BACKLIGHT);
    CHECK(plan.frame_rate == BOBE_POWER_STATUS_FPS);
    CHECK(plan.backlight_percent > 0);
    puts("power policy tests passed");
    return 0;
}
