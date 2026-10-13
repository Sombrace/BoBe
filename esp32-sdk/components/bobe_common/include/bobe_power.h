#pragma once

#include "bobe_types.h"

#include <stdbool.h>
#include <stdint.h>

#define BOBE_POWER_ACTIVE_BACKLIGHT 75U
#define BOBE_POWER_STATUS_BACKLIGHT 50U
#define BOBE_POWER_IDLE_BACKLIGHT 35U
#define BOBE_POWER_ACTIVE_FPS 30U
#define BOBE_POWER_STATUS_FPS 10U
#define BOBE_POWER_IDLE_FPS 5U
#define BOBE_POWER_IDLE_AFTER_US UINT64_C(60000000)

typedef struct {
    bobe_lifecycle_state_t lifecycle;
    bobe_turn_state_t turn;
    uint64_t now_us;
    uint64_t last_interaction_us;
    bool microphone_active;
    bool notice_active;
} bobe_power_input_t;

typedef struct {
    uint8_t backlight_percent;
    uint8_t frame_rate;
} bobe_power_plan_t;

void bobe_power_plan(
    const bobe_power_input_t *input,
    bobe_power_plan_t *plan
);
