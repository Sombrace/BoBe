#include "bobe_power.h"

#include <string.h>

void bobe_power_plan(
    const bobe_power_input_t *input,
    bobe_power_plan_t *plan
)
{
    if (plan == NULL) {
        return;
    }
    *plan = (bobe_power_plan_t){
        .backlight_percent = BOBE_POWER_ACTIVE_BACKLIGHT,
        .frame_rate = BOBE_POWER_ACTIVE_FPS,
    };
    if (input == NULL || input->turn != BOBE_TURN_IDLE ||
        input->microphone_active || input->notice_active) {
        return;
    }
    const uint64_t idle_us = input->now_us >= input->last_interaction_us
                                 ? input->now_us - input->last_interaction_us
                                 : 0;
    if (input->lifecycle == BOBE_LIFECYCLE_ONLINE) {
        if (idle_us >= BOBE_POWER_IDLE_AFTER_US) {
            plan->backlight_percent = BOBE_POWER_IDLE_BACKLIGHT;
            plan->frame_rate = BOBE_POWER_IDLE_FPS;
        }
        return;
    }
    switch (input->lifecycle) {
    case BOBE_LIFECYCLE_UNPROVISIONED:
    case BOBE_LIFECYCLE_JOINING_WIFI:
    case BOBE_LIFECYCLE_DISCOVERING_HUB:
    case BOBE_LIFECYCLE_CONNECTING:
    case BOBE_LIFECYCLE_RECONNECTING:
    case BOBE_LIFECYCLE_LOW_BATTERY:
        if (idle_us >= BOBE_POWER_IDLE_AFTER_US) {
            plan->backlight_percent = BOBE_POWER_STATUS_BACKLIGHT;
            plan->frame_rate = BOBE_POWER_STATUS_FPS;
        }
        break;
    default:
        break;
    }
}
