#pragma once

#include "bobe_state.h"
#include "bobe_types.h"

#include <stdbool.h>
#include <stdint.h>

bool bobe_notice_volume(
    uint8_t previous,
    uint8_t next,
    bobe_notice_t *notice
);
bool bobe_notice_control_hint(
    const bobe_state_machine_t *state,
    bobe_notice_t *notice
);
bool bobe_notice_update_confirmed(bobe_notice_t *notice);
