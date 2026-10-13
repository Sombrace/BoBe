#pragma once

#include "bobe_types.h"

#include <stdbool.h>

typedef struct {
    bobe_lifecycle_state_t lifecycle;
    bobe_turn_state_t turn;
    bool microphone_muted;
} bobe_state_machine_t;

void bobe_state_init(bobe_state_machine_t *state);
bool bobe_state_set_lifecycle(
    bobe_state_machine_t *state,
    bobe_lifecycle_state_t next
);
bool bobe_state_set_turn(bobe_state_machine_t *state, bobe_turn_state_t next);
bobe_face_expression_t bobe_state_default_face(const bobe_state_machine_t *state);
const char *bobe_state_default_caption(const bobe_state_machine_t *state);
bool bobe_state_face_targets_turn(
    const bobe_face_state_t *face,
    const char *lease_id,
    const char *turn_id
);
