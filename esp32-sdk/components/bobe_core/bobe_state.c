#include "bobe_state.h"

#include <string.h>

static bool lifecycle_transition_allowed(
    bobe_lifecycle_state_t current,
    bobe_lifecycle_state_t next
)
{
    if (current == next) {
        return true;
    }
    if (next == BOBE_LIFECYCLE_FATAL_ERROR ||
        next == BOBE_LIFECYCLE_SHUTTING_DOWN) {
        return true;
    }

    switch (current) {
    case BOBE_LIFECYCLE_BOOTING:
        return next == BOBE_LIFECYCLE_UNPROVISIONED ||
               next == BOBE_LIFECYCLE_JOINING_WIFI;
    case BOBE_LIFECYCLE_UNPROVISIONED:
        return next == BOBE_LIFECYCLE_JOINING_WIFI;
    case BOBE_LIFECYCLE_JOINING_WIFI:
        return next == BOBE_LIFECYCLE_DISCOVERING_HUB ||
               next == BOBE_LIFECYCLE_UNPROVISIONED ||
               next == BOBE_LIFECYCLE_RECONNECTING;
    case BOBE_LIFECYCLE_DISCOVERING_HUB:
        return next == BOBE_LIFECYCLE_JOINING_WIFI ||
               next == BOBE_LIFECYCLE_CONNECTING ||
               next == BOBE_LIFECYCLE_RECONNECTING ||
               next == BOBE_LIFECYCLE_UNPROVISIONED;
    case BOBE_LIFECYCLE_CONNECTING:
        return next == BOBE_LIFECYCLE_JOINING_WIFI ||
               next == BOBE_LIFECYCLE_ONLINE ||
               next == BOBE_LIFECYCLE_RECONNECTING;
    case BOBE_LIFECYCLE_ONLINE:
        return next == BOBE_LIFECYCLE_UNPROVISIONED ||
               next == BOBE_LIFECYCLE_JOINING_WIFI ||
               next == BOBE_LIFECYCLE_RECONNECTING ||
               next == BOBE_LIFECYCLE_UPDATING ||
               next == BOBE_LIFECYCLE_LOW_BATTERY;
    case BOBE_LIFECYCLE_RECONNECTING:
        return next == BOBE_LIFECYCLE_JOINING_WIFI ||
               next == BOBE_LIFECYCLE_DISCOVERING_HUB ||
               next == BOBE_LIFECYCLE_CONNECTING ||
               next == BOBE_LIFECYCLE_ONLINE ||
               next == BOBE_LIFECYCLE_UNPROVISIONED;
    case BOBE_LIFECYCLE_UPDATING:
        return next == BOBE_LIFECYCLE_REBOOTING ||
               next == BOBE_LIFECYCLE_ONLINE;
    case BOBE_LIFECYCLE_REBOOTING:
    case BOBE_LIFECYCLE_SHUTTING_DOWN:
    case BOBE_LIFECYCLE_FATAL_ERROR:
        return false;
    case BOBE_LIFECYCLE_LOW_BATTERY:
        return next == BOBE_LIFECYCLE_ONLINE;
    default:
        return false;
    }
}

static bool turn_transition_allowed(bobe_turn_state_t current, bobe_turn_state_t next)
{
    if (current == next) {
        return true;
    }
    if (next == BOBE_TURN_ABORTED && current != BOBE_TURN_IDLE) {
        return true;
    }

    switch (current) {
    case BOBE_TURN_IDLE:
        return next == BOBE_TURN_REQUESTING_CAPTURE;
    case BOBE_TURN_REQUESTING_CAPTURE:
        return next == BOBE_TURN_LISTENING;
    case BOBE_TURN_LISTENING:
        return next == BOBE_TURN_WAITING_FOR_RUNTIME;
    case BOBE_TURN_WAITING_FOR_RUNTIME:
        return next == BOBE_TURN_THINKING ||
               next == BOBE_TURN_SPEAKING;
    case BOBE_TURN_THINKING:
        return next == BOBE_TURN_SPEAKING;
    case BOBE_TURN_SPEAKING:
        return next == BOBE_TURN_IDLE ||
               next == BOBE_TURN_INTERRUPTED;
    case BOBE_TURN_INTERRUPTED:
        return next == BOBE_TURN_LISTENING ||
               next == BOBE_TURN_IDLE;
    case BOBE_TURN_ABORTED:
        return next == BOBE_TURN_IDLE;
    default:
        return false;
    }
}

void bobe_state_init(bobe_state_machine_t *state)
{
    if (state != NULL) {
        state->lifecycle = BOBE_LIFECYCLE_BOOTING;
        state->turn = BOBE_TURN_IDLE;
        state->microphone_muted = false;
    }
}

bool bobe_state_set_lifecycle(
    bobe_state_machine_t *state,
    bobe_lifecycle_state_t next
)
{
    if (state == NULL || !lifecycle_transition_allowed(state->lifecycle, next)) {
        return false;
    }
    state->lifecycle = next;
    if (next != BOBE_LIFECYCLE_ONLINE && state->turn != BOBE_TURN_IDLE) {
        state->turn = BOBE_TURN_ABORTED;
    }
    return true;
}

bool bobe_state_set_turn(bobe_state_machine_t *state, bobe_turn_state_t next)
{
    if (state == NULL ||
        (state->lifecycle != BOBE_LIFECYCLE_ONLINE &&
         !(state->turn == BOBE_TURN_ABORTED && next == BOBE_TURN_IDLE)) ||
        !turn_transition_allowed(state->turn, next)) {
        return false;
    }
    state->turn = next;
    return true;
}

bobe_face_expression_t bobe_state_default_face(const bobe_state_machine_t *state)
{
    if (state == NULL) {
        return BOBE_FACE_ERROR;
    }

    if (state->lifecycle != BOBE_LIFECYCLE_ONLINE) {
        switch (state->lifecycle) {
        case BOBE_LIFECYCLE_BOOTING:
            return BOBE_FACE_BOOTING;
        case BOBE_LIFECYCLE_UNPROVISIONED:
            return BOBE_FACE_UNPROVISIONED;
        case BOBE_LIFECYCLE_JOINING_WIFI:
        case BOBE_LIFECYCLE_DISCOVERING_HUB:
        case BOBE_LIFECYCLE_CONNECTING:
        case BOBE_LIFECYCLE_RECONNECTING:
            return BOBE_FACE_CONNECTING;
        case BOBE_LIFECYCLE_UPDATING:
        case BOBE_LIFECYCLE_REBOOTING:
            return BOBE_FACE_UPDATING;
        case BOBE_LIFECYCLE_LOW_BATTERY:
            return BOBE_FACE_CONCERNED;
        case BOBE_LIFECYCLE_SHUTTING_DOWN:
            return BOBE_FACE_SLEEPING;
        case BOBE_LIFECYCLE_FATAL_ERROR:
            return BOBE_FACE_ERROR;
        case BOBE_LIFECYCLE_ONLINE:
            break;
        }
    }

    if (state->microphone_muted) {
        return BOBE_FACE_MUTED;
    }

    switch (state->turn) {
    case BOBE_TURN_REQUESTING_CAPTURE:
        return BOBE_FACE_ATTENTIVE;
    case BOBE_TURN_LISTENING:
        return BOBE_FACE_LISTENING;
    case BOBE_TURN_WAITING_FOR_RUNTIME:
    case BOBE_TURN_THINKING:
        return BOBE_FACE_THINKING;
    case BOBE_TURN_SPEAKING:
        return BOBE_FACE_SPEAKING;
    case BOBE_TURN_INTERRUPTED:
        return BOBE_FACE_ATTENTIVE;
    case BOBE_TURN_ABORTED:
        return BOBE_FACE_CONCERNED;
    case BOBE_TURN_IDLE:
    default:
        return BOBE_FACE_IDLE;
    }
}

const char *bobe_state_default_caption(const bobe_state_machine_t *state)
{
    if (state == NULL) {
        return "Needs help";
    }
    if (state->lifecycle == BOBE_LIFECYCLE_ONLINE &&
        state->microphone_muted) {
        return "Microphone muted";
    }
    switch (state->lifecycle) {
    case BOBE_LIFECYCLE_BOOTING:
        return "Starting";
    case BOBE_LIFECYCLE_UNPROVISIONED:
        return "Ready to set up";
    case BOBE_LIFECYCLE_JOINING_WIFI:
        return "Joining Wi-Fi";
    case BOBE_LIFECYCLE_DISCOVERING_HUB:
        return "Finding BoBe";
    case BOBE_LIFECYCLE_CONNECTING:
        return "Connecting to BoBe";
    case BOBE_LIFECYCLE_RECONNECTING:
        return "Reconnecting to BoBe";
    case BOBE_LIFECYCLE_UPDATING:
        return "Updating";
    case BOBE_LIFECYCLE_REBOOTING:
        return "Restarting";
    case BOBE_LIFECYCLE_LOW_BATTERY:
        return "Low battery";
    case BOBE_LIFECYCLE_SHUTTING_DOWN:
        return "Good night";
    case BOBE_LIFECYCLE_FATAL_ERROR:
        return "Needs help";
    case BOBE_LIFECYCLE_ONLINE:
        return "BoBe";
    }
    return "BoBe";
}

bool bobe_state_face_targets_turn(
    const bobe_face_state_t *face,
    const char *lease_id,
    const char *turn_id
)
{
    if (face == NULL || lease_id == NULL || turn_id == NULL) {
        return false;
    }
    return face->lease_id[0] == '\0' ||
           (strcmp(face->lease_id, lease_id) == 0 &&
            strcmp(face->turn_id, turn_id) == 0);
}
