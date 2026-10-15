#include "bobe_policy.h"

#include <limits.h>
#include <string.h>

_Static_assert(
    sizeof(bobe_policy_context_t) <= BOBE_POLICY_CONTEXT_MAX_BYTES,
    "Core policy context exceeds its fixed memory budget"
);
_Static_assert(
    sizeof(bobe_policy_event_t) <= BOBE_POLICY_EVENT_MAX_BYTES,
    "Core policy event exceeds its fixed memory budget"
);
_Static_assert(
    sizeof(bobe_policy_effect_t) <= BOBE_POLICY_EFFECT_MAX_BYTES,
    "Core policy effect exceeds its fixed memory budget"
);
_Static_assert(
    sizeof(bobe_policy_effect_batch_t) <=
        BOBE_POLICY_EFFECT_BATCH_MAX_BYTES,
    "Core policy effect batch exceeds its fixed memory budget"
);

static void copy_text(char *destination, size_t capacity, const char *source)
{
    if (capacity == 0) {
        return;
    }
    if (source == NULL) {
        destination[0] = '\0';
        return;
    }
    size_t length = strlen(source);
    if (length >= capacity) {
        length = capacity - 1U;
    }
    memcpy(destination, source, length);
    destination[length] = '\0';
}

static uint64_t deadline_after(uint64_t now_us, uint64_t delay_us)
{
    return UINT64_MAX - now_us < delay_us
               ? UINT64_MAX
               : now_us + delay_us;
}

static bool event_id_after(uint32_t candidate, uint32_t reference)
{
    return (int32_t)(candidate - reference) > 0;
}

static bobe_policy_effect_t *append_effect(
    bobe_policy_effect_batch_t *effects,
    bobe_policy_effect_kind_t kind
)
{
    if (effects->count >= BOBE_POLICY_MAX_EFFECTS) {
        effects->overflowed = true;
        return NULL;
    }
    bobe_policy_effect_t *effect = &effects->items[effects->count++];
    memset(effect, 0, sizeof(*effect));
    effect->kind = kind;
    return effect;
}

static void maybe_start_wake_monitor(
    const bobe_policy_context_t *context,
    bobe_policy_effect_batch_t *effects
)
{
    if (!context->state.microphone_muted &&
        context->state.lifecycle == BOBE_LIFECYCLE_ONLINE &&
        context->transport_connected) {
        (void)append_effect(
            effects,
            BOBE_POLICY_EFFECT_WAKE_MONITOR_START
        );
    }
}

static void emit_update_state(
    bobe_policy_effect_batch_t *effects,
    const char *phase,
    uint8_t progress_percent,
    const char *reason
)
{
    bobe_policy_effect_t *const effect = append_effect(
        effects,
        BOBE_POLICY_EFFECT_TRANSPORT_UPDATE_STATE
    );
    if (effect == NULL) {
        return;
    }
    effect->data.update_state.progress_percent = progress_percent;
    copy_text(
        effect->data.update_state.phase,
        sizeof(effect->data.update_state.phase),
        phase
    );
    copy_text(
        effect->data.update_state.reason,
        sizeof(effect->data.update_state.reason),
        reason
    );
}

static void emit_default_face(
    bobe_policy_context_t *context,
    bobe_policy_effect_batch_t *effects
)
{
    context->face_expiry_us = 0;
    bobe_policy_effect_t *effect = append_effect(
        effects,
        BOBE_POLICY_EFFECT_DISPLAY_FACE
    );
    if (effect == NULL) {
        return;
    }
    effect->data.face.state_version = ++context->local_face_version;
    effect->data.face.expression = bobe_state_default_face(&context->state);
    effect->data.face.accent_rgb = BOBE_FACE_THEME_ACCENT_RGB;
    copy_text(
        effect->data.face.caption,
        sizeof(effect->data.face.caption),
        bobe_state_default_caption(&context->state)
    );
}

static void queue_notice(
    bobe_policy_context_t *context,
    const bobe_notice_t *notice
)
{
    context->pending_notice = *notice;
}

static void queue_control_hint(bobe_policy_context_t *context)
{
    bobe_notice_t notice;
    if (bobe_notice_control_hint(&context->state, &notice)) {
        queue_notice(context, &notice);
    }
}

static bool set_lifecycle(
    bobe_policy_context_t *context,
    bobe_lifecycle_state_t next,
    bobe_policy_effect_batch_t *effects
)
{
    if (context->state.lifecycle == next) {
        return true;
    }
    if (!bobe_state_set_lifecycle(&context->state, next)) {
        return false;
    }
    emit_default_face(context, effects);
    queue_control_hint(context);
    return true;
}

static bool set_turn(
    bobe_policy_context_t *context,
    bobe_turn_state_t next,
    bobe_policy_effect_batch_t *effects
)
{
    if (context->state.turn == next) {
        return true;
    }
    if (!bobe_state_set_turn(&context->state, next)) {
        return false;
    }
    emit_default_face(context, effects);
    return true;
}

static void abort_turn_to_idle(
    bobe_policy_context_t *context,
    bobe_policy_effect_batch_t *effects
)
{
    (void)set_turn(context, BOBE_TURN_ABORTED, effects);
    (void)set_turn(context, BOBE_TURN_IDLE, effects);
}

static void clear_capture_request(bobe_policy_context_t *context)
{
    context->capture_stream_id = 0;
    context->action_capture_press_event_id = 0;
    context->capture_request_id[0] = '\0';
    context->capture_request_deadline_us = 0;
    context->wake_speech_ended_pending = false;
}

static void emit_microphone(
    bobe_policy_effect_batch_t *effects,
    bool active
)
{
    bobe_policy_effect_t *effect = append_effect(
        effects,
        BOBE_POLICY_EFFECT_DISPLAY_MICROPHONE
    );
    if (effect != NULL) {
        effect->data.microphone_active = active;
    }
}

static void emit_notice(
    bobe_policy_effect_batch_t *effects,
    const bobe_notice_t *notice
)
{
    bobe_policy_effect_t *effect = append_effect(
        effects,
        BOBE_POLICY_EFFECT_DISPLAY_NOTICE
    );
    if (effect != NULL) {
        effect->data.notice = *notice;
    }
}

static void flush_pending_notice(
    bobe_policy_context_t *context,
    bobe_policy_effect_batch_t *effects
)
{
    if (context->pending_notice.kind == BOBE_NOTICE_NONE) {
        return;
    }
    emit_notice(effects, &context->pending_notice);
    memset(&context->pending_notice, 0, sizeof(context->pending_notice));
}

static void emit_volume_notice(
    bobe_policy_effect_batch_t *effects,
    uint8_t previous,
    uint8_t next
)
{
    bobe_notice_t notice;
    if (bobe_notice_volume(previous, next, &notice)) {
        emit_notice(effects, &notice);
    }
}

static void emit_stream_effect(
    bobe_policy_effect_batch_t *effects,
    bobe_policy_effect_kind_t kind,
    bobe_policy_operation_t operation,
    uint32_t stream_id
)
{
    bobe_policy_effect_t *effect = append_effect(effects, kind);
    if (effect != NULL) {
        effect->operation = operation;
        effect->data.stream_id = stream_id;
    }
}

static void emit_failure(
    bobe_policy_context_t *context,
    bobe_policy_effect_batch_t *effects,
    bobe_policy_failure_t failure
)
{
    context->transport_connected = false;
    bobe_policy_effect_t *effect = append_effect(
        effects,
        BOBE_POLICY_EFFECT_TRANSPORT_FAIL_SESSION
    );
    if (effect != NULL) {
        effect->data.failure = failure;
    }
}

static void emit_capture_state(
    const bobe_policy_context_t *context,
    bobe_policy_effect_batch_t *effects,
    bool open,
    bobe_policy_reason_t reason,
    bobe_policy_operation_t operation
)
{
    bobe_policy_effect_t *effect = append_effect(
        effects,
        BOBE_POLICY_EFFECT_TRANSPORT_CAPTURE_STATE
    );
    if (effect == NULL) {
        return;
    }
    effect->operation = operation;
    effect->data.capture_state.open = open;
    effect->data.capture_state.stream_id = context->capture_stream_id;
    effect->data.capture_state.reason = reason;
    copy_text(
        effect->data.capture_state.lease_id,
        sizeof(effect->data.capture_state.lease_id),
        context->lease_id
    );
    copy_text(
        effect->data.capture_state.turn_id,
        sizeof(effect->data.capture_state.turn_id),
        context->turn_id
    );
}

static void emit_capture_cancel(
    const bobe_policy_context_t *context,
    bobe_policy_effect_batch_t *effects
)
{
    bobe_policy_effect_t *effect = append_effect(
        effects,
        BOBE_POLICY_EFFECT_TRANSPORT_CAPTURE_CANCEL
    );
    if (effect == NULL) {
        return;
    }
    effect->data.capture_cancel.stream_id = context->capture_stream_id;
    copy_text(
        effect->data.capture_cancel.request_id,
        sizeof(effect->data.capture_cancel.request_id),
        context->capture_request_id
    );
}

static void emit_playback(
    const bobe_policy_context_t *context,
    bobe_policy_effect_batch_t *effects,
    bobe_policy_effect_kind_t kind,
    bobe_policy_operation_t operation,
    uint32_t stream_id,
    bobe_policy_reason_t reason
)
{
    bobe_policy_effect_t *effect = append_effect(effects, kind);
    if (effect == NULL) {
        return;
    }
    effect->operation = operation;
    effect->data.playback.stream_id = stream_id;
    effect->data.playback.reason = reason;
    copy_text(
        effect->data.playback.lease_id,
        sizeof(effect->data.playback.lease_id),
        context->lease_id
    );
    copy_text(
        effect->data.playback.turn_id,
        sizeof(effect->data.playback.turn_id),
        context->turn_id
    );
}

static void flush_pending_button(
    bobe_policy_context_t *context,
    bobe_policy_effect_batch_t *effects
)
{
    if (!context->pending_button_report) {
        return;
    }
    if (context->transport_connected) {
        bobe_policy_effect_t *effect = append_effect(
            effects,
            BOBE_POLICY_EFFECT_TRANSPORT_BUTTON
        );
        if (effect != NULL) {
            effect->data.button = context->pending_button;
        }
    }
    context->pending_button_report = false;
}

static void reset_active_turn(
    bobe_policy_context_t *context,
    bobe_policy_effect_batch_t *effects,
    bobe_policy_reason_t reason,
    bool restart_wake_monitor
)
{
    emit_stream_effect(
        effects,
        BOBE_POLICY_EFFECT_AUDIO_CAPTURE_STOP,
        BOBE_POLICY_OPERATION_NONE,
        context->capture_stream_id
    );
    emit_stream_effect(
        effects,
        BOBE_POLICY_EFFECT_VOICE_STREAM_END,
        BOBE_POLICY_OPERATION_NONE,
        context->capture_stream_id
    );
    (void)append_effect(effects, BOBE_POLICY_EFFECT_WAKE_MONITOR_STOP);
    emit_microphone(effects, false);
    (void)append_effect(effects, BOBE_POLICY_EFFECT_TRANSPORT_DISCARD_AUDIO);
    (void)append_effect(effects, BOBE_POLICY_EFFECT_AUDIO_PLAYBACK_ABORT);
    (void)append_effect(
        effects,
        BOBE_POLICY_EFFECT_TRANSPORT_CLEAR_PLAYBACK_ROUTE
    );
    ++context->playback_route_generation;
    context->playback_stream_id = 0;

    if (context->state.turn != BOBE_TURN_IDLE) {
        if (context->state.turn != BOBE_TURN_ABORTED &&
            context->state.lifecycle == BOBE_LIFECYCLE_ONLINE) {
            (void)set_turn(context, BOBE_TURN_ABORTED, effects);
        }
        if (context->state.turn == BOBE_TURN_ABORTED) {
            (void)set_turn(context, BOBE_TURN_IDLE, effects);
        }
    }
    if (reason != BOBE_POLICY_REASON_NONE &&
        context->capture_stream_id != 0 &&
        context->lease_id[0] != '\0' &&
        context->turn_id[0] != '\0' &&
        context->transport_connected) {
        emit_capture_state(context, effects, false, reason, BOBE_POLICY_OPERATION_NONE);
    }

    clear_capture_request(context);
    context->lease_id[0] = '\0';
    context->turn_id[0] = '\0';
    context->lease_deadline_us = 0;
    context->pending_capture_finish_reason = BOBE_POLICY_REASON_NONE;
    context->pending_playback_stream_id = 0;
    context->pending_operation = BOBE_POLICY_OPERATION_NONE;
    if (restart_wake_monitor) {
        maybe_start_wake_monitor(context, effects);
    }
}

static void return_to_idle(
    bobe_policy_context_t *context,
    bobe_policy_effect_batch_t *effects,
    bobe_policy_reason_t reason
)
{
    reset_active_turn(context, effects, reason, true);
}

static void enter_unprovisioned(
    bobe_policy_context_t *context,
    bobe_policy_effect_batch_t *effects
)
{
    if (context->state.lifecycle == BOBE_LIFECYCLE_CONNECTING) {
        (void)set_lifecycle(
            context,
            BOBE_LIFECYCLE_RECONNECTING,
            effects
        );
    }
    (void)set_lifecycle(
        context,
        BOBE_LIFECYCLE_UNPROVISIONED,
        effects
    );
}

static void shutdown_body(
    bobe_policy_context_t *context,
    bobe_policy_effect_batch_t *effects
)
{
    reset_active_turn(
        context,
        effects,
        BOBE_POLICY_REASON_SHUTDOWN,
        false
    );
    if (context->transport_configured) {
        (void)append_effect(effects, BOBE_POLICY_EFFECT_TRANSPORT_STOP);
    }
    (void)set_lifecycle(
        context,
        BOBE_LIFECYCLE_SHUTTING_DOWN,
        effects
    );
    (void)append_effect(effects, BOBE_POLICY_EFFECT_BOARD_SHUTDOWN);
}

static void begin_operation(
    bobe_policy_context_t *context,
    bobe_policy_effect_t *effect,
    bobe_policy_operation_t operation
)
{
    if (effect == NULL) {
        return;
    }
    context->pending_operation = operation;
    effect->operation = operation;
}

static void request_capture(
    bobe_policy_context_t *context,
    bobe_policy_effect_batch_t *effects,
    bobe_policy_capture_trigger_t trigger,
    uint32_t action_press_event_id,
    const bobe_policy_capture_seed_t *seed
)
{
    if (context->state.microphone_muted ||
        context->state.lifecycle != BOBE_LIFECYCLE_ONLINE ||
        context->state.turn != BOBE_TURN_IDLE ||
        !set_turn(context, BOBE_TURN_REQUESTING_CAPTURE, effects)) {
        return;
    }
    if (seed == NULL || !seed->valid || seed->request_id[0] == '\0') {
        abort_turn_to_idle(context, effects);
        clear_capture_request(context);
        return;
    }

    context->capture_stream_id =
        seed->stream_id == 0 ? UINT32_C(1) : seed->stream_id;
    context->action_capture_press_event_id = action_press_event_id;
    context->wake_speech_ended_pending = false;
    copy_text(
        context->capture_request_id,
        sizeof(context->capture_request_id),
        seed->request_id
    );
    context->lease_id[0] = '\0';
    context->turn_id[0] = '\0';

    bobe_policy_effect_t *effect = append_effect(
        effects,
        BOBE_POLICY_EFFECT_TRANSPORT_CAPTURE_REQUEST
    );
    if (effect == NULL) {
        return;
    }
    effect->data.capture_request.stream_id = context->capture_stream_id;
    effect->data.capture_request.trigger = trigger;
    copy_text(
        effect->data.capture_request.request_id,
        sizeof(effect->data.capture_request.request_id),
        context->capture_request_id
    );
    begin_operation(
        context,
        effect,
        BOBE_POLICY_OPERATION_CAPTURE_REQUEST
    );
}

static void begin_capture_finish(
    bobe_policy_context_t *context,
    bobe_policy_effect_batch_t *effects,
    bobe_policy_reason_t reason
)
{
    if ((context->state.turn != BOBE_TURN_REQUESTING_CAPTURE &&
         context->state.turn != BOBE_TURN_LISTENING) ||
        context->pending_operation != BOBE_POLICY_OPERATION_NONE) {
        return;
    }
    context->pending_capture_finish_reason = reason;
    context->capture_finish_origin = context->state.turn;
    bobe_policy_effect_t *effect = append_effect(
        effects,
        BOBE_POLICY_EFFECT_AUDIO_CAPTURE_STOP
    );
    if (effect != NULL) {
        effect->data.stream_id = context->capture_stream_id;
    }
    begin_operation(
        context,
        effect,
        BOBE_POLICY_OPERATION_CAPTURE_STOP
    );
}

static void handle_action_release(
    bobe_policy_context_t *context,
    bobe_policy_effect_batch_t *effects,
    uint32_t release_event_id
)
{
    if ((context->state.turn != BOBE_TURN_REQUESTING_CAPTURE &&
         context->state.turn != BOBE_TURN_LISTENING) ||
        context->action_capture_press_event_id == 0 ||
        !event_id_after(
            release_event_id,
            context->action_capture_press_event_id
        )) {
        return;
    }
    begin_capture_finish(
        context,
        effects,
        BOBE_POLICY_REASON_BUTTON_RELEASED
    );
}

static void start_transport(
    bobe_policy_context_t *context,
    bobe_policy_effect_batch_t *effects,
    bool retry
)
{
    if (!retry) {
        (void)set_lifecycle(
            context,
            BOBE_LIFECYCLE_CONNECTING,
            effects
        );
    }
    bobe_policy_effect_t *effect = append_effect(
        effects,
        BOBE_POLICY_EFFECT_TRANSPORT_START
    );
    context->pending_transport_start_is_retry = retry;
    begin_operation(
        context,
        effect,
        BOBE_POLICY_OPERATION_TRANSPORT_START
    );
}

static void begin_discovery(
    bobe_policy_context_t *context,
    bobe_policy_effect_batch_t *effects
)
{
    bobe_policy_effect_t *effect = append_effect(
        effects,
        BOBE_POLICY_EFFECT_CONNECTIVITY_DISCOVER_HUB
    );
    if (effect != NULL) {
        effect->data.discovery.network_generation =
            context->network_generation;
    }
    begin_operation(
        context,
        effect,
        BOBE_POLICY_OPERATION_DISCOVERY_START
    );
}

static void finish_provisioning_transition(
    bobe_policy_context_t *context,
    bobe_policy_effect_batch_t *effects,
    bool network_online
)
{
    context->network_online = network_online;
    enter_unprovisioned(context, effects);
    (void)set_lifecycle(
        context,
        BOBE_LIFECYCLE_JOINING_WIFI,
        effects
    );
    if (!network_online) {
        return;
    }
    (void)set_lifecycle(
        context,
        BOBE_LIFECYCLE_DISCOVERING_HUB,
        effects
    );
    start_transport(context, effects, false);
}

static void begin_provisioning_complete(
    bobe_policy_context_t *context,
    bobe_policy_effect_batch_t *effects
)
{
    context->transport_retry_attempt = 0;
    context->transport_retry_deadline_us = 0;
    context->pending_workflow =
        BOBE_POLICY_WORKFLOW_PROVISIONING_COMPLETE;
    if (context->transport_configured) {
        bobe_policy_effect_t *effect = append_effect(
            effects,
            BOBE_POLICY_EFFECT_TRANSPORT_STOP
        );
        begin_operation(
            context,
            effect,
            BOBE_POLICY_OPERATION_TRANSPORT_STOP
        );
        return;
    }
    bobe_policy_effect_t *effect = append_effect(
        effects,
        BOBE_POLICY_EFFECT_TRANSPORT_RELOAD
    );
    begin_operation(
        context,
        effect,
        BOBE_POLICY_OPERATION_TRANSPORT_RELOAD
    );
}

static void handle_provisioning_aborted(
    bobe_policy_context_t *context,
    bobe_policy_effect_batch_t *effects
)
{
    context->transport_retry_attempt = 0;
    context->transport_retry_deadline_us = 0;
    if (!context->transport_configured) {
        enter_unprovisioned(context, effects);
        return;
    }
    finish_provisioning_transition(
        context,
        effects,
        context->network_online
    );
}

static void begin_provisioning_recovery(
    bobe_policy_context_t *context,
    bobe_policy_effect_batch_t *effects
)
{
    if (context->provisioning) {
        return;
    }
    context->transport_retry_attempt = 0;
    context->transport_retry_deadline_us = 0;
    reset_active_turn(
        context,
        effects,
        BOBE_POLICY_REASON_PROVISIONING_RECOVERY,
        false
    );
    context->provisioning_restart_transport =
        context->transport_configured;
    context->pending_workflow =
        BOBE_POLICY_WORKFLOW_PROVISIONING_RECOVERY;
    if (context->provisioning_restart_transport) {
        bobe_policy_effect_t *effect = append_effect(
            effects,
            BOBE_POLICY_EFFECT_TRANSPORT_STOP
        );
        begin_operation(
            context,
            effect,
            BOBE_POLICY_OPERATION_TRANSPORT_STOP
        );
        return;
    }
    enter_unprovisioned(context, effects);
    bobe_policy_effect_t *effect = append_effect(
        effects,
        BOBE_POLICY_EFFECT_CONNECTIVITY_OPEN_PROVISIONING
    );
    begin_operation(
        context,
        effect,
        BOBE_POLICY_OPERATION_PROVISIONING_OPEN
    );
}

static void toggle_microphone_mute(
    bobe_policy_context_t *context,
    bobe_policy_effect_batch_t *effects
)
{
    context->state.microphone_muted =
        !context->state.microphone_muted;
    bobe_policy_effect_t *const mute_effect = append_effect(
        effects,
        BOBE_POLICY_EFFECT_AUDIO_SET_MUTE
    );
    if (mute_effect != NULL) {
        mute_effect->data.mute.muted =
            context->state.microphone_muted;
        mute_effect->data.mute.persist = true;
    }
    if (context->state.microphone_muted) {
        if (context->state.turn == BOBE_TURN_REQUESTING_CAPTURE &&
            context->capture_request_id[0] != '\0') {
            emit_capture_cancel(context, effects);
        }
        reset_active_turn(
            context,
            effects,
            BOBE_POLICY_REASON_MICROPHONE_MUTED,
            false
        );
    } else {
        emit_microphone(effects, false);
        maybe_start_wake_monitor(context, effects);
    }
    emit_default_face(context, effects);
    queue_control_hint(context);
}

static void handle_button(
    bobe_policy_context_t *context,
    const bobe_policy_event_t *event,
    bobe_policy_effect_batch_t *effects
)
{
    const bobe_button_event_t *button = &event->data.button.event;
    context->transport_connected =
        event->data.button.transport_connected;
    context->transport_configured =
        event->data.button.transport_configured;
    context->provisioning = event->data.button.provisioning;
    if (event_id_after(
            event->data.button.latest_action_release_event_id,
            context->latest_action_release_event_id
        )) {
        context->latest_action_release_event_id =
            event->data.button.latest_action_release_event_id;
    }

    if (button->action == BOBE_BUTTON_LONG_PRESS &&
        (button->button == BOBE_BUTTON_ACTION ||
         button->button == BOBE_BUTTON_VOLUME_DOWN) &&
        event->data.button.physical_action_pressed &&
        event->data.button.physical_volume_down_pressed) {
        shutdown_body(context, effects);
        return;
    }

    if (button->action == BOBE_BUTTON_LONG_PRESS &&
        (button->button == BOBE_BUTTON_VOLUME_UP ||
         button->button == BOBE_BUTTON_VOLUME_DOWN) &&
        event->data.button.physical_volume_up_pressed &&
        event->data.button.physical_volume_down_pressed &&
        !context->provisioning) {
        begin_provisioning_recovery(context, effects);
        return;
    }

    if (button->action == BOBE_BUTTON_LONG_PRESS &&
        button->button == BOBE_BUTTON_VOLUME_DOWN &&
        event->data.button.physical_volume_down_pressed &&
        !event->data.button.physical_action_pressed &&
        !event->data.button.physical_volume_up_pressed) {
        toggle_microphone_mute(context, effects);
        return;
    }

    if (button->action != BOBE_BUTTON_PRESSED &&
        button->action != BOBE_BUTTON_RELEASED) {
        return;
    }

    context->pending_button_report = true;
    context->pending_button = *button;
    if (button->button == BOBE_BUTTON_ACTION &&
        button->action == BOBE_BUTTON_RELEASED) {
        if (event_id_after(
                button->event_id,
                context->latest_action_release_event_id
            )) {
            context->latest_action_release_event_id = button->event_id;
        }
        handle_action_release(context, effects, button->event_id);
    }

    if (button->button == BOBE_BUTTON_VOLUME_UP &&
        button->action == BOBE_BUTTON_PRESSED) {
        bobe_policy_effect_t *effect = append_effect(
            effects,
            BOBE_POLICY_EFFECT_AUDIO_SET_VOLUME
        );
        if (effect != NULL) {
            effect->data.volume =
                event->data.button.current_volume <= 95
                    ? (uint8_t)(event->data.button.current_volume + 5U)
                    : UINT8_C(100);
            emit_volume_notice(
                effects,
                event->data.button.current_volume,
                effect->data.volume
            );
        }
    } else if (
        button->button == BOBE_BUTTON_VOLUME_DOWN &&
        button->action == BOBE_BUTTON_PRESSED
    ) {
        bobe_policy_effect_t *effect = append_effect(
            effects,
            BOBE_POLICY_EFFECT_AUDIO_SET_VOLUME
        );
        if (effect != NULL) {
            effect->data.volume =
                event->data.button.current_volume >= 5
                    ? (uint8_t)(event->data.button.current_volume - 5U)
                    : UINT8_C(0);
            emit_volume_notice(
                effects,
                event->data.button.current_volume,
                effect->data.volume
            );
        }
    } else if (
        button->button == BOBE_BUTTON_ACTION &&
        button->action == BOBE_BUTTON_PRESSED &&
        event_id_after(
            button->event_id,
            event->data.button.latest_action_release_event_id
        ) &&
        event->data.button.physical_action_pressed &&
        context->state.lifecycle == BOBE_LIFECYCLE_ONLINE &&
        context->state.turn == BOBE_TURN_IDLE
    ) {
        request_capture(
            context,
            effects,
            BOBE_POLICY_TRIGGER_ACTION_BUTTON,
            button->event_id,
            &event->data.button.capture_seed
        );
    }

    if (context->pending_operation == BOBE_POLICY_OPERATION_NONE) {
        flush_pending_button(context, effects);
    }
}

static void handle_network(
    bobe_policy_context_t *context,
    const bobe_policy_event_t *event,
    bobe_policy_effect_batch_t *effects
)
{
    if (event->data.network.state !=
            BOBE_NETWORK_PROVISIONING_COMPLETE &&
        event->data.network.generation !=
            event->data.network.current_generation) {
        return;
    }
    context->network_generation =
        event->data.network.current_generation;
    context->network_online = event->data.network.network_online;
    context->provisioning = event->data.network.provisioning;
    context->transport_configured =
        event->data.network.transport_configured;

    switch (event->data.network.state) {
    case BOBE_NETWORK_UNPROVISIONED:
        (void)set_lifecycle(
            context,
            BOBE_LIFECYCLE_UNPROVISIONED,
            effects
        );
        break;
    case BOBE_NETWORK_CONNECTING:
        if (context->state.lifecycle == BOBE_LIFECYCLE_BOOTING ||
            context->state.lifecycle == BOBE_LIFECYCLE_UNPROVISIONED) {
            (void)set_lifecycle(
                context,
                BOBE_LIFECYCLE_JOINING_WIFI,
                effects
            );
        }
        break;
    case BOBE_NETWORK_CONNECTED: {
        if (context->provisioning) {
            break;
        }
        context->transport_retry_attempt = 0;
        context->transport_retry_deadline_us = 0;
        if (context->state.lifecycle == BOBE_LIFECYCLE_BOOTING ||
            context->state.lifecycle == BOBE_LIFECYCLE_UNPROVISIONED) {
            (void)set_lifecycle(
                context,
                BOBE_LIFECYCLE_JOINING_WIFI,
                effects
            );
        }
        if (context->state.lifecycle != BOBE_LIFECYCLE_JOINING_WIFI &&
            context->state.lifecycle != BOBE_LIFECYCLE_RECONNECTING) {
            break;
        }
        if (!context->transport_configured) {
            context->pending_workflow =
                BOBE_POLICY_WORKFLOW_NETWORK_CONNECTED;
            bobe_policy_effect_t *effect = append_effect(
                effects,
                BOBE_POLICY_EFFECT_TRANSPORT_CONFIGURE_STORED
            );
            begin_operation(
                context,
                effect,
                BOBE_POLICY_OPERATION_CONFIGURE_STORED_ENDPOINT
            );
            break;
        }
        (void)set_lifecycle(
            context,
            BOBE_LIFECYCLE_DISCOVERING_HUB,
            effects
        );
        start_transport(context, effects, false);
        break;
    }
    case BOBE_NETWORK_PROVISIONING_COMPLETE:
        begin_provisioning_complete(context, effects);
        break;
    case BOBE_NETWORK_PROVISIONING_ABORTED:
        handle_provisioning_aborted(context, effects);
        break;
    case BOBE_NETWORK_DISCONNECTED:
    case BOBE_NETWORK_AUTH_FAILED:
        break;
    }
}

static bool control_route_matches(
    const bobe_policy_context_t *context,
    const char *lease_id,
    const char *turn_id
)
{
    return strcmp(lease_id, context->lease_id) == 0 &&
           strcmp(turn_id, context->turn_id) == 0;
}

static void handle_theme_set(
    bobe_policy_context_t *context,
    const bobe_control_message_t *control,
    bobe_policy_effect_batch_t *effects
)
{
    const bobe_theme_state_t *const theme = &control->data.theme;
    if (theme->state_version < context->theme_version) {
        return;
    }
    if (theme->state_version == context->theme_version) {
        return;
    }
    context->theme_version = theme->state_version;
    if (theme->theme == context->theme) {
        return;
    }
    context->theme = theme->theme;
    bobe_policy_effect_t *const effect =
        append_effect(effects, BOBE_POLICY_EFFECT_DISPLAY_THEME);
    if (effect != NULL) {
        effect->data.theme = *theme;
    }
}

static void handle_capture_control(
    bobe_policy_context_t *context,
    const bobe_control_message_t *control,
    uint64_t now_us,
    bobe_policy_effect_batch_t *effects
)
{
    if (control->kind == BOBE_CONTROL_CAPTURE_GRANTED) {
        if (context->state.turn != BOBE_TURN_REQUESTING_CAPTURE ||
            control->data.capture.stream_id != context->capture_stream_id ||
            strcmp(
                control->data.capture.request_id,
                context->capture_request_id
            ) != 0) {
            return;
        }
        copy_text(
            context->lease_id,
            sizeof(context->lease_id),
            control->data.capture.lease_id
        );
        copy_text(
            context->turn_id,
            sizeof(context->turn_id),
            control->data.capture.turn_id
        );
        context->capture_request_deadline_us = 0;
        context->lease_deadline_us = deadline_after(
            now_us,
            (uint64_t)BOBE_MAX_LEASE_LIFETIME_MS * UINT64_C(1000)
        );
        bobe_policy_effect_t *effect = append_effect(
            effects,
            BOBE_POLICY_EFFECT_TRANSPORT_CAPTURE_STATE
        );
        if (effect != NULL) {
            effect->data.capture_state.open = true;
            effect->data.capture_state.stream_id =
                context->capture_stream_id;
            copy_text(
                effect->data.capture_state.lease_id,
                sizeof(effect->data.capture_state.lease_id),
                context->lease_id
            );
            copy_text(
                effect->data.capture_state.turn_id,
                sizeof(effect->data.capture_state.turn_id),
                context->turn_id
            );
        }
        begin_operation(
            context,
            effect,
            BOBE_POLICY_OPERATION_CAPTURE_OPEN
        );
    } else if (
        control->kind == BOBE_CONTROL_CAPTURE_DENIED &&
        context->state.turn == BOBE_TURN_REQUESTING_CAPTURE &&
        control->data.capture.stream_id == context->capture_stream_id &&
        strcmp(
            control->data.capture.request_id,
            context->capture_request_id
        ) == 0
    ) {
        abort_turn_to_idle(context, effects);
        clear_capture_request(context);
    }
}

static void handle_playback_open(
    bobe_policy_context_t *context,
    const bobe_control_message_t *control,
    bool audio_capturing,
    bobe_policy_effect_batch_t *effects
)
{
    if ((context->state.turn != BOBE_TURN_WAITING_FOR_RUNTIME &&
         context->state.turn != BOBE_TURN_THINKING) ||
        !control_route_matches(
            context,
            control->data.playback_open.lease_id,
            control->data.playback_open.turn_id
        ) ||
        audio_capturing) {
        return;
    }

    (void)append_effect(effects, BOBE_POLICY_EFFECT_WAKE_MONITOR_STOP);
    context->pending_playback_stream_id =
        control->data.playback_open.stream_id;
    bobe_policy_effect_t *effect = append_effect(
        effects,
        BOBE_POLICY_EFFECT_AUDIO_PLAYBACK_OPEN
    );
    if (effect != NULL) {
        effect->data.stream_id =
            control->data.playback_open.stream_id;
    }
    begin_operation(
        context,
        effect,
        BOBE_POLICY_OPERATION_PLAYBACK_OPEN
    );
}

static void handle_playback_close(
    bobe_policy_context_t *context,
    const bobe_control_message_t *control,
    bobe_policy_effect_batch_t *effects
)
{
    if (!control_route_matches(
            context,
            control->data.playback_close.lease_id,
            control->data.playback_close.turn_id
        ) ||
        control->data.playback_close.stream_id !=
            context->playback_stream_id) {
        return;
    }
    context->pending_playback_stream_id =
        context->playback_stream_id;
    bobe_policy_effect_t *effect = append_effect(
        effects,
        BOBE_POLICY_EFFECT_AUDIO_PLAYBACK_CLOSE
    );
    if (effect != NULL) {
        effect->data.stream_id = context->playback_stream_id;
    }
    begin_operation(
        context,
        effect,
        BOBE_POLICY_OPERATION_PLAYBACK_CLOSE
    );
}

static void handle_control(
    bobe_policy_context_t *context,
    const bobe_control_message_t *control,
    uint64_t now_us,
    bool audio_capturing,
    bobe_policy_effect_batch_t *effects
)
{
    switch (control->kind) {
    case BOBE_CONTROL_CAPTURE_GRANTED:
    case BOBE_CONTROL_CAPTURE_DENIED:
        handle_capture_control(context, control, now_us, effects);
        break;
    case BOBE_CONTROL_FACE_SET:
        if (!context->state.microphone_muted &&
            bobe_state_face_targets_turn(
                &control->data.face,
                context->lease_id,
                context->turn_id
            ) &&
            control->data.face.state_version >
                context->remote_face_version &&
            (control->data.face.expression != BOBE_FACE_LISTENING ||
             context->state.turn == BOBE_TURN_LISTENING) &&
            (control->data.face.expression != BOBE_FACE_SPEAKING ||
             context->state.turn == BOBE_TURN_SPEAKING)) {
            context->remote_face_version =
                control->data.face.state_version;
            bobe_policy_effect_t *effect = append_effect(
                effects,
                BOBE_POLICY_EFFECT_DISPLAY_FACE
            );
            if (effect != NULL) {
                effect->data.face = control->data.face;
                effect->data.face.ttl_ms = 0;
            }
            context->face_expiry_us =
                control->data.face.ttl_ms > 0
                    ? deadline_after(
                          now_us,
                          (uint64_t)control->data.face.ttl_ms *
                              UINT64_C(1000)
                      )
                    : 0;
        }
        break;
    case BOBE_CONTROL_THEME_SET:
        handle_theme_set(context, control, effects);
        break;
    case BOBE_CONTROL_PLAYBACK_OPEN:
        handle_playback_open(
            context,
            control,
            audio_capturing,
            effects
        );
        break;
    case BOBE_CONTROL_PLAYBACK_CLOSE:
        handle_playback_close(context, control, effects);
        break;
    case BOBE_CONTROL_TURN_STATE:
        if (!control_route_matches(
                context,
                control->data.turn.lease_id,
                control->data.turn.turn_id
            )) {
            break;
        }
        if ((control->data.turn.state == BOBE_TURN_THINKING &&
             context->state.turn == BOBE_TURN_WAITING_FOR_RUNTIME) ||
            control->data.turn.state == BOBE_TURN_ABORTED) {
            (void)set_turn(context, control->data.turn.state, effects);
            if (control->data.turn.state == BOBE_TURN_ABORTED) {
                return_to_idle(
                    context,
                    effects,
                    BOBE_POLICY_REASON_NONE
                );
            }
        } else if (
            control->data.turn.state == BOBE_TURN_INTERRUPTED &&
            context->state.turn == BOBE_TURN_SPEAKING
        ) {
            (void)append_effect(
                effects,
                BOBE_POLICY_EFFECT_AUDIO_PLAYBACK_ABORT
            );
            (void)append_effect(
                effects,
                BOBE_POLICY_EFFECT_TRANSPORT_CLEAR_PLAYBACK_ROUTE
            );
            ++context->playback_route_generation;
            context->playback_stream_id = 0;
            (void)set_turn(
                context,
                BOBE_TURN_INTERRUPTED,
                effects
            );
        } else if (control->data.turn.state == BOBE_TURN_IDLE) {
            return_to_idle(
                context,
                effects,
                BOBE_POLICY_REASON_NONE
            );
        }
        break;
    case BOBE_CONTROL_UPDATE_OFFER:
        if (context->state.lifecycle != BOBE_LIFECYCLE_ONLINE ||
            context->state.turn != BOBE_TURN_IDLE) {
            emit_update_state(effects, "failed", 0, "busy");
        } else if (!context->battery_valid) {
            emit_update_state(effects, "failed", 0, "battery_unknown");
        } else if (context->battery.percent < 50 &&
                   !context->battery.charging) {
            emit_update_state(effects, "failed", 0, "low_battery");
        } else {
            reset_active_turn(
                context,
                effects,
                BOBE_POLICY_REASON_NONE,
                false
            );
            (void)set_lifecycle(
                context,
                BOBE_LIFECYCLE_UPDATING,
                effects
            );
            emit_update_state(effects, "accepted", 0, NULL);
            bobe_policy_effect_t *const effect = append_effect(
                effects,
                BOBE_POLICY_EFFECT_UPDATE_INSTALL
            );
            if (effect != NULL) {
                effect->data.update = control->data.update;
            }
            begin_operation(
                context,
                effect,
                BOBE_POLICY_OPERATION_UPDATE_INSTALL
            );
        }
        break;
    case BOBE_CONTROL_WELCOME:
    case BOBE_CONTROL_INPUT_RESULT:
    case BOBE_CONTROL_DISPLAY_TEXT:
    case BOBE_CONTROL_SENSOR_ACK:
    case BOBE_CONTROL_UNKNOWN:
    case BOBE_CONTROL_ERROR:
        break;
    }
}

static void handle_transport(
    bobe_policy_context_t *context,
    const bobe_policy_event_t *event,
    bobe_policy_effect_batch_t *effects
)
{
    if (event->data.transport.generation !=
        event->data.transport.current_generation) {
        return;
    }
    context->transport_generation =
        event->data.transport.current_generation;
    switch (event->data.transport.kind) {
    case BOBE_POLICY_TRANSPORT_LINK_CONNECTED:
        break;
    case BOBE_POLICY_TRANSPORT_SESSION_READY:
        context->transport_connected = true;
        context->transport_retry_attempt = 0;
        context->transport_retry_deadline_us = 0;
        context->remote_face_version = 0;
        context->theme_version = 0;
        (void)append_effect(
            effects,
            BOBE_POLICY_EFFECT_CONNECTIVITY_CONFIRM_TIME
        );
        maybe_start_wake_monitor(context, effects);
        if (context->state.lifecycle == BOBE_LIFECYCLE_CONNECTING ||
            context->state.lifecycle == BOBE_LIFECYCLE_RECONNECTING) {
            (void)set_lifecycle(
                context,
                BOBE_LIFECYCLE_ONLINE,
                effects
            );
        }
        if (event->data.transport.ota_pending_verification) {
            bobe_policy_effect_t *const effect = append_effect(
                effects,
                BOBE_POLICY_EFFECT_OTA_CONFIRM
            );
            begin_operation(
                context,
                effect,
                BOBE_POLICY_OPERATION_OTA_CONFIRM
            );
            bobe_notice_t notice;
            if (bobe_notice_update_confirmed(&notice)) {
                queue_notice(context, &notice);
            }
        }
        break;
    case BOBE_POLICY_TRANSPORT_DISCONNECTED:
        break;
    case BOBE_POLICY_TRANSPORT_CONTROL:
        handle_control(
            context,
            &event->data.transport.control,
            event->now_us,
            event->data.transport.audio_capturing,
            effects
        );
        break;
    case BOBE_POLICY_TRANSPORT_ERROR:
        break;
    }
}

static void handle_playback_progress(
    const bobe_policy_context_t *context,
    const bobe_policy_event_t *event,
    bobe_policy_effect_batch_t *effects
)
{
    if (context->state.turn != BOBE_TURN_SPEAKING ||
        event->data.playback_progress.stream_id !=
            context->playback_stream_id ||
        event->data.playback_progress.route_generation !=
            context->playback_route_generation ||
        context->lease_id[0] == '\0' ||
        context->turn_id[0] == '\0') {
        return;
    }
    bobe_policy_effect_t *effect = append_effect(
        effects,
        BOBE_POLICY_EFFECT_TRANSPORT_PLAYBACK_ACK
    );
    if (effect == NULL) {
        return;
    }
    effect->data.playback.stream_id =
        event->data.playback_progress.stream_id;
    effect->data.playback.played_samples =
        event->data.playback_progress.played_samples;
    effect->data.playback.credit_ms =
        event->data.playback_progress.credit_ms;
    copy_text(
        effect->data.playback.lease_id,
        sizeof(effect->data.playback.lease_id),
        context->lease_id
    );
    copy_text(
        effect->data.playback.turn_id,
        sizeof(effect->data.playback.turn_id),
        context->turn_id
    );
}

static void defer_critical(
    bobe_policy_context_t *context,
    const bobe_policy_event_t *event,
    uint32_t flags
)
{
    context->deferred_critical = event->data.critical;
    context->deferred_critical.flags = flags;
}

static void handle_critical(
    bobe_policy_context_t *context,
    const bobe_policy_event_t *event,
    bobe_policy_effect_batch_t *effects
)
{
    uint32_t flags = event->data.critical.flags;
    context->network_online = event->data.critical.network_online;
    context->provisioning = event->data.critical.provisioning;
    context->transport_connected =
        event->data.critical.transport_connected;
    context->transport_configured =
        event->data.critical.transport_configured;

    if ((flags & BOBE_POLICY_CRITICAL_SHUTDOWN) != 0) {
        shutdown_body(context, effects);
        return;
    }
    if ((flags & BOBE_POLICY_CRITICAL_PROVISIONING_COMPLETE) != 0) {
        flags &= ~BOBE_POLICY_CRITICAL_PROVISIONING_COMPLETE;
        begin_provisioning_complete(context, effects);
        if (context->pending_operation != BOBE_POLICY_OPERATION_NONE) {
            defer_critical(context, event, flags);
            return;
        }
    }
    if ((flags & BOBE_POLICY_CRITICAL_PROVISIONING_ABORTED) != 0) {
        flags &= ~BOBE_POLICY_CRITICAL_PROVISIONING_ABORTED;
        handle_provisioning_aborted(context, effects);
        if (context->pending_operation != BOBE_POLICY_OPERATION_NONE) {
            defer_critical(context, event, flags);
            return;
        }
    }
    if ((flags & BOBE_POLICY_CRITICAL_PROVISIONING_RECOVERY) != 0 &&
        !context->provisioning) {
        flags &= ~BOBE_POLICY_CRITICAL_PROVISIONING_RECOVERY;
        begin_provisioning_recovery(context, effects);
        if (context->pending_operation != BOBE_POLICY_OPERATION_NONE) {
            defer_critical(context, event, flags);
            return;
        }
    }
    if ((flags & (BOBE_POLICY_CRITICAL_NETWORK_LOST |
                  BOBE_POLICY_CRITICAL_TRANSPORT_LOST |
                  BOBE_POLICY_CRITICAL_EVENT_OVERFLOW)) != 0) {
        if ((flags & BOBE_POLICY_CRITICAL_EVENT_OVERFLOW) != 0 &&
            context->transport_connected) {
            emit_failure(
                context,
                effects,
                BOBE_POLICY_FAILURE_NO_MEMORY
            );
        }
        const bool network_lost =
            (flags & BOBE_POLICY_CRITICAL_NETWORK_LOST) != 0;
        if (network_lost) {
            context->network_online = false;
        }
        context->transport_connected = false;
        reset_active_turn(
            context,
            effects,
            network_lost
                ? BOBE_POLICY_REASON_NETWORK_LOST
                : BOBE_POLICY_REASON_BODYLINK_DISCONNECTED,
            false
        );
        if (context->provisioning) {
            return;
        }
        if (context->state.lifecycle != BOBE_LIFECYCLE_UNPROVISIONED &&
            context->state.lifecycle != BOBE_LIFECYCLE_SHUTTING_DOWN) {
            (void)set_lifecycle(
                context,
                network_lost ? BOBE_LIFECYCLE_JOINING_WIFI
                             : BOBE_LIFECYCLE_RECONNECTING,
                effects
            );
        }
        if (!network_lost) {
            context->transport_retry_deadline_us = deadline_after(
                event->now_us,
                (uint64_t)BOBE_POLICY_TRANSPORT_RETRY_BASE_MS *
                    UINT64_C(1000)
            );
        }
        return;
    }

    if ((flags & BOBE_POLICY_CRITICAL_PLAYBACK_FAILED) != 0) {
        (void)append_effect(
            effects,
            BOBE_POLICY_EFFECT_AUDIO_PLAYBACK_ABORT
        );
        (void)append_effect(
            effects,
            BOBE_POLICY_EFFECT_TRANSPORT_CLEAR_PLAYBACK_ROUTE
        );
        emit_failure(context, effects, BOBE_POLICY_FAILURE_NO_MEMORY);
        ++context->playback_route_generation;
        context->playback_stream_id = 0;
    }
    if ((flags & BOBE_POLICY_CRITICAL_CAPTURE_FAILED) != 0) {
        reset_active_turn(
            context,
            effects,
            BOBE_POLICY_REASON_UPLINK_BACKPRESSURE,
            true
        );
    }
    if ((flags & BOBE_POLICY_CRITICAL_ACTION_RELEASE) != 0) {
        flags &= ~BOBE_POLICY_CRITICAL_ACTION_RELEASE;
        if (event_id_after(
                event->data.critical.latest_action_release_event_id,
                context->latest_action_release_event_id
            )) {
            context->latest_action_release_event_id =
                event->data.critical.latest_action_release_event_id;
        }
        handle_action_release(
            context,
            effects,
            event->data.critical.latest_action_release_event_id
        );
        if (context->pending_operation != BOBE_POLICY_OPERATION_NONE) {
            defer_critical(context, event, flags);
            return;
        }
    }
    if ((flags & BOBE_POLICY_CRITICAL_WAKE_DETECTED) != 0) {
        request_capture(
            context,
            effects,
            BOBE_POLICY_TRIGGER_WAKE_WORD,
            0,
            &event->data.critical.capture_seed
        );
    }
    if ((flags & BOBE_POLICY_CRITICAL_SPEECH_ENDED) != 0 &&
        context->action_capture_press_event_id == 0) {
        if (context->state.turn == BOBE_TURN_REQUESTING_CAPTURE) {
            context->wake_speech_ended_pending = true;
        } else if (context->state.turn == BOBE_TURN_LISTENING) {
            begin_capture_finish(
                context,
                effects,
                BOBE_POLICY_REASON_VAD_END
            );
        }
    }
}

static void resume_deferred_critical(
    bobe_policy_context_t *context,
    uint64_t now_us,
    bobe_policy_effect_batch_t *effects
)
{
    if (context->pending_operation != BOBE_POLICY_OPERATION_NONE ||
        context->deferred_critical.flags == 0) {
        return;
    }
    bobe_policy_event_t event = {
        .kind = BOBE_POLICY_EVENT_CRITICAL,
        .now_us = now_us,
        .data.critical = context->deferred_critical,
    };
    memset(
        &context->deferred_critical,
        0,
        sizeof(context->deferred_critical)
    );
    handle_critical(context, &event, effects);
}

static void handle_time_tick(
    bobe_policy_context_t *context,
    const bobe_policy_event_t *event,
    bobe_policy_effect_batch_t *effects
)
{
    context->network_online = event->data.tick.network_online;
    context->provisioning = event->data.tick.provisioning;
    if (context->ota_confirm_deadline_us != 0 &&
        event->now_us >= context->ota_confirm_deadline_us) {
        context->ota_confirm_deadline_us = 0;
        (void)append_effect(
            effects,
            BOBE_POLICY_EFFECT_OTA_ROLLBACK
        );
        return;
    }
    if (context->capture_request_deadline_us != 0 &&
        event->now_us >= context->capture_request_deadline_us) {
        emit_capture_cancel(context, effects);
        return_to_idle(
            context,
            effects,
            BOBE_POLICY_REASON_REQUEST_TIMEOUT
        );
    }
    if (context->lease_deadline_us != 0 &&
        event->now_us >= context->lease_deadline_us) {
        if (context->playback_stream_id != 0) {
            context->pending_playback_stream_id =
                context->playback_stream_id;
            emit_playback(
                context,
                effects,
                BOBE_POLICY_EFFECT_TRANSPORT_PLAYBACK_FAILED,
                BOBE_POLICY_OPERATION_PLAYBACK_FAILED,
                context->playback_stream_id,
                BOBE_POLICY_REASON_LOCAL_LEASE_FAILSAFE_EXPIRED
            );
            context->pending_operation =
                BOBE_POLICY_OPERATION_PLAYBACK_FAILED;
            return;
        }
        emit_failure(context, effects, BOBE_POLICY_FAILURE_TIMEOUT);
        return_to_idle(context, effects, BOBE_POLICY_REASON_NONE);
    }
    if (context->face_expiry_us != 0 &&
        event->now_us >= context->face_expiry_us) {
        emit_default_face(context, effects);
    }
    if (context->transport_retry_deadline_us != 0 &&
        event->now_us >= context->transport_retry_deadline_us &&
        context->network_online &&
        !context->provisioning &&
        (context->state.lifecycle == BOBE_LIFECYCLE_CONNECTING ||
         context->state.lifecycle == BOBE_LIFECYCLE_RECONNECTING)) {
        start_transport(context, effects, true);
        return;
    }
    if ((event->data.tick.periodic_flags &
         BOBE_POLICY_TICK_BATTERY) != 0) {
        (void)append_effect(
            effects,
            BOBE_POLICY_EFFECT_READ_BATTERY
        );
    }
    if ((event->data.tick.periodic_flags &
         BOBE_POLICY_TICK_HEALTH) != 0) {
        (void)append_effect(
            effects,
            BOBE_POLICY_EFFECT_REPORT_HEALTH
        );
    }
    flush_pending_notice(context, effects);
}

static void handle_discovery(
    bobe_policy_context_t *context,
    const bobe_policy_event_t *event,
    bobe_policy_effect_batch_t *effects
)
{
    if (!event->data.discovery.network_online ||
        event->data.discovery.network_generation !=
            event->data.discovery.current_network_generation) {
        return;
    }
    context->network_online = true;
    context->network_generation =
        event->data.discovery.current_network_generation;
    context->transport_configured =
        event->data.discovery.transport_configured;
    context->pending_workflow =
        BOBE_POLICY_WORKFLOW_DISCOVERY_RESULT;
    if (event->data.discovery.success) {
        bobe_policy_effect_t *effect = append_effect(
            effects,
            BOBE_POLICY_EFFECT_TRANSPORT_CONFIGURE_ENDPOINT
        );
        if (effect != NULL) {
            copy_text(
                effect->data.endpoint.uri,
                sizeof(effect->data.endpoint.uri),
                event->data.discovery.uri
            );
        }
        begin_operation(
            context,
            effect,
            BOBE_POLICY_OPERATION_CONFIGURE_DISCOVERED_ENDPOINT
        );
    } else if (!context->transport_configured) {
        bobe_policy_effect_t *effect = append_effect(
            effects,
            BOBE_POLICY_EFFECT_TRANSPORT_CONFIGURE_STORED
        );
        begin_operation(
            context,
            effect,
            BOBE_POLICY_OPERATION_CONFIGURE_STORED_ENDPOINT
        );
    } else {
        start_transport(context, effects, false);
    }
}

static void handle_battery(
    bobe_policy_context_t *context,
    const bobe_policy_event_t *event,
    bobe_policy_effect_batch_t *effects
)
{
    if (!event->data.battery.valid) {
        return;
    }
    context->battery_valid = true;
    context->battery = event->data.battery.state;
    const bobe_battery_state_t *battery = &context->battery;
    if (battery->percent == 0 && !battery->charging) {
        if (context->low_battery_samples < UINT8_MAX) {
            ++context->low_battery_samples;
        }
        if (context->low_battery_samples >= 3) {
            if (context->state.lifecycle == BOBE_LIFECYCLE_ONLINE) {
                (void)set_lifecycle(
                    context,
                    BOBE_LIFECYCLE_LOW_BATTERY,
                    effects
                );
            }
            shutdown_body(context, effects);
            return;
        }
    } else if (battery->percent >= 5 || battery->charging) {
        context->low_battery_samples = 0;
    }
    if (event->data.battery.transport_connected) {
        bobe_policy_effect_t *effect = append_effect(
            effects,
            BOBE_POLICY_EFFECT_SEND_BATTERY
        );
        if (effect != NULL) {
            effect->data.battery = *battery;
        }
    }
}

static void finish_capture_stop(
    bobe_policy_context_t *context,
    const bobe_policy_event_t *event,
    bobe_policy_effect_batch_t *effects
)
{
    context->capture_stop_succeeded =
        event->data.effect_result.result == BOBE_POLICY_RESULT_OK;
    emit_microphone(effects, false);
    bobe_policy_effect_t *effect = append_effect(
        effects,
        BOBE_POLICY_EFFECT_VOICE_STREAM_END
    );
    if (effect != NULL) {
        effect->data.stream_id = context->capture_stream_id;
    }
    begin_operation(
        context,
        effect,
        BOBE_POLICY_OPERATION_VOICE_END
    );
}

static void finish_voice_end(
    bobe_policy_context_t *context,
    const bobe_policy_event_t *event,
    bobe_policy_effect_batch_t *effects
)
{
    if (context->capture_finish_origin ==
        BOBE_TURN_REQUESTING_CAPTURE) {
        (void)append_effect(
            effects,
            BOBE_POLICY_EFFECT_TRANSPORT_DISCARD_AUDIO
        );
        emit_capture_cancel(context, effects);
        clear_capture_request(context);
        abort_turn_to_idle(context, effects);
        context->pending_capture_finish_reason =
            BOBE_POLICY_REASON_NONE;
        flush_pending_button(context, effects);
        return;
    }
    if (!context->capture_stop_succeeded) {
        return_to_idle(
            context,
            effects,
            BOBE_POLICY_REASON_CAPTURE_STOP_FAILED
        );
        flush_pending_button(context, effects);
        return;
    }
    if (event->data.effect_result.result != BOBE_POLICY_RESULT_OK) {
        return_to_idle(
            context,
            effects,
            BOBE_POLICY_REASON_CAPTURE_FRONTEND_DRAIN_FAILED
        );
        flush_pending_button(context, effects);
        return;
    }
    bobe_policy_effect_t *effect = append_effect(
        effects,
        BOBE_POLICY_EFFECT_TRANSPORT_FLUSH_AUDIO
    );
    begin_operation(
        context,
        effect,
        BOBE_POLICY_OPERATION_CAPTURE_FLUSH
    );
}

static void handle_transport_start_result(
    bobe_policy_context_t *context,
    const bobe_policy_event_t *event,
    bobe_policy_effect_batch_t *effects
)
{
    if (event->data.effect_result.result == BOBE_POLICY_RESULT_OK) {
        context->transport_retry_deadline_us = 0;
        context->transport_generation =
            event->data.effect_result.transport_generation;
        return;
    }
    if (!context->pending_transport_start_is_retry) {
        (void)set_lifecycle(
            context,
            BOBE_LIFECYCLE_RECONNECTING,
            effects
        );
        context->transport_retry_deadline_us = deadline_after(
            event->now_us,
            (uint64_t)BOBE_POLICY_TRANSPORT_RETRY_BASE_MS *
                UINT64_C(1000)
        );
        return;
    }
    if (context->transport_retry_attempt < 6) {
        ++context->transport_retry_attempt;
    }
    const uint32_t ceiling_ms =
        BOBE_POLICY_TRANSPORT_RETRY_BASE_MS
        << context->transport_retry_attempt;
    const uint32_t bounded_ms =
        ceiling_ms < BOBE_POLICY_TRANSPORT_RETRY_MAX_MS
            ? ceiling_ms
            : BOBE_POLICY_TRANSPORT_RETRY_MAX_MS;
    const uint32_t jitter_span =
        bounded_ms - BOBE_POLICY_TRANSPORT_RETRY_BASE_MS + 1U;
    const uint32_t delay_ms =
        BOBE_POLICY_TRANSPORT_RETRY_BASE_MS +
        event->data.effect_result.retry_random % jitter_span;
    context->transport_retry_deadline_us = deadline_after(
        event->now_us,
        (uint64_t)delay_ms * UINT64_C(1000)
    );
}

static void handle_endpoint_result(
    bobe_policy_context_t *context,
    const bobe_policy_event_t *event,
    bobe_policy_effect_batch_t *effects
)
{
    context->transport_configured =
        event->data.effect_result.transport_configured;
    if (context->pending_workflow ==
        BOBE_POLICY_WORKFLOW_NETWORK_CONNECTED) {
        if (event->data.effect_result.result == BOBE_POLICY_RESULT_OK &&
            context->transport_configured) {
            (void)set_lifecycle(
                context,
                BOBE_LIFECYCLE_DISCOVERING_HUB,
                effects
            );
            context->pending_workflow = BOBE_POLICY_WORKFLOW_NONE;
            start_transport(context, effects, false);
        } else {
            begin_discovery(context, effects);
        }
        return;
    }
    if (context->pending_workflow ==
        BOBE_POLICY_WORKFLOW_DISCOVERY_RESULT) {
        context->pending_workflow = BOBE_POLICY_WORKFLOW_NONE;
        if (event->data.effect_result.result == BOBE_POLICY_RESULT_OK &&
            context->transport_configured) {
            start_transport(context, effects, false);
        } else {
            (void)set_lifecycle(
                context,
                BOBE_LIFECYCLE_UNPROVISIONED,
                effects
            );
        }
    }
}

static void handle_transport_stop_result(
    bobe_policy_context_t *context,
    const bobe_policy_event_t *event,
    bobe_policy_effect_batch_t *effects
)
{
    if (context->pending_workflow ==
        BOBE_POLICY_WORKFLOW_PROVISIONING_COMPLETE) {
        if (event->data.effect_result.result != BOBE_POLICY_RESULT_OK) {
            context->pending_workflow = BOBE_POLICY_WORKFLOW_NONE;
            enter_unprovisioned(context, effects);
            return;
        }
        bobe_policy_effect_t *effect = append_effect(
            effects,
            BOBE_POLICY_EFFECT_TRANSPORT_RELOAD
        );
        begin_operation(
            context,
            effect,
            BOBE_POLICY_OPERATION_TRANSPORT_RELOAD
        );
    } else if (
        context->pending_workflow ==
        BOBE_POLICY_WORKFLOW_PROVISIONING_RECOVERY
    ) {
        if (event->data.effect_result.result != BOBE_POLICY_RESULT_OK) {
            context->pending_workflow = BOBE_POLICY_WORKFLOW_NONE;
            return;
        }
        enter_unprovisioned(context, effects);
        bobe_policy_effect_t *effect = append_effect(
            effects,
            BOBE_POLICY_EFFECT_CONNECTIVITY_OPEN_PROVISIONING
        );
        begin_operation(
            context,
            effect,
            BOBE_POLICY_OPERATION_PROVISIONING_OPEN
        );
    }
}

static void handle_effect_result(
    bobe_policy_context_t *context,
    const bobe_policy_event_t *event,
    bobe_policy_effect_batch_t *effects
)
{
    const bobe_policy_operation_t operation =
        event->data.effect_result.operation;
    if (operation == BOBE_POLICY_OPERATION_NONE ||
        operation != context->pending_operation) {
        return;
    }
    context->transport_connected =
        event->data.effect_result.transport_connected;
    context->transport_configured =
        event->data.effect_result.transport_configured;
    context->network_online = event->data.effect_result.network_online;
    context->pending_operation = BOBE_POLICY_OPERATION_NONE;

    switch (operation) {
    case BOBE_POLICY_OPERATION_CAPTURE_REQUEST:
        if (event->data.effect_result.result == BOBE_POLICY_RESULT_OK) {
            context->capture_request_deadline_us = deadline_after(
                event->now_us,
                BOBE_POLICY_CAPTURE_REQUEST_TIMEOUT_US
            );
        } else {
            abort_turn_to_idle(context, effects);
            clear_capture_request(context);
        }
        flush_pending_button(context, effects);
        break;
    case BOBE_POLICY_OPERATION_CAPTURE_OPEN:
        if (event->data.effect_result.result != BOBE_POLICY_RESULT_OK) {
            return_to_idle(
                context,
                effects,
                BOBE_POLICY_REASON_AUDIO_START_FAILED
            );
            break;
        }
        emit_stream_effect(
            effects,
            BOBE_POLICY_EFFECT_VOICE_STREAM_BEGIN,
            BOBE_POLICY_OPERATION_VOICE_BEGIN,
            context->capture_stream_id
        );
        context->pending_operation = BOBE_POLICY_OPERATION_VOICE_BEGIN;
        break;
    case BOBE_POLICY_OPERATION_VOICE_BEGIN:
        if (event->data.effect_result.result != BOBE_POLICY_RESULT_OK) {
            return_to_idle(
                context,
                effects,
                BOBE_POLICY_REASON_AUDIO_START_FAILED
            );
            break;
        }
        emit_stream_effect(
            effects,
            BOBE_POLICY_EFFECT_AUDIO_CAPTURE_START,
            BOBE_POLICY_OPERATION_CAPTURE_START,
            context->capture_stream_id
        );
        context->pending_operation = BOBE_POLICY_OPERATION_CAPTURE_START;
        break;
    case BOBE_POLICY_OPERATION_CAPTURE_START:
        if (event->data.effect_result.result != BOBE_POLICY_RESULT_OK) {
            return_to_idle(
                context,
                effects,
                BOBE_POLICY_REASON_AUDIO_START_FAILED
            );
            break;
        }
        emit_microphone(effects, true);
        (void)set_turn(context, BOBE_TURN_LISTENING, effects);
        if (context->action_capture_press_event_id == 0 &&
            context->wake_speech_ended_pending) {
            begin_capture_finish(
                context,
                effects,
                BOBE_POLICY_REASON_VAD_END
            );
        }
        break;
    case BOBE_POLICY_OPERATION_CAPTURE_STOP:
        finish_capture_stop(context, event, effects);
        break;
    case BOBE_POLICY_OPERATION_VOICE_END:
        finish_voice_end(context, event, effects);
        break;
    case BOBE_POLICY_OPERATION_CAPTURE_FLUSH:
        if (event->data.effect_result.result != BOBE_POLICY_RESULT_OK) {
            return_to_idle(
                context,
                effects,
                BOBE_POLICY_REASON_UPLINK_BACKPRESSURE
            );
            flush_pending_button(context, effects);
            break;
        }
        emit_capture_state(
            context,
            effects,
            false,
            context->pending_capture_finish_reason,
            BOBE_POLICY_OPERATION_CAPTURE_CLOSE
        );
        context->pending_operation =
            BOBE_POLICY_OPERATION_CAPTURE_CLOSE;
        break;
    case BOBE_POLICY_OPERATION_CAPTURE_CLOSE:
        if (event->data.effect_result.result != BOBE_POLICY_RESULT_OK) {
            return_to_idle(
                context,
                effects,
                BOBE_POLICY_REASON_CAPTURE_CLOSE_FAILED
            );
        } else {
            (void)set_turn(
                context,
                BOBE_TURN_WAITING_FOR_RUNTIME,
                effects
            );
            context->pending_capture_finish_reason =
                BOBE_POLICY_REASON_NONE;
        }
        flush_pending_button(context, effects);
        break;
    case BOBE_POLICY_OPERATION_PLAYBACK_OPEN:
        if (event->data.effect_result.result != BOBE_POLICY_RESULT_OK) {
            emit_failure(context, effects, BOBE_POLICY_FAILURE_GENERIC);
            reset_active_turn(
                context,
                effects,
                BOBE_POLICY_REASON_NONE,
                false
            );
            break;
        }
        ++context->playback_route_generation;
        context->playback_stream_id =
            context->pending_playback_stream_id;
        emit_stream_effect(
            effects,
            BOBE_POLICY_EFFECT_TRANSPORT_SET_PLAYBACK_ROUTE,
            BOBE_POLICY_OPERATION_NONE,
            context->playback_stream_id
        );
        emit_playback(
            context,
            effects,
            BOBE_POLICY_EFFECT_TRANSPORT_PLAYBACK_READY,
            BOBE_POLICY_OPERATION_PLAYBACK_READY,
            context->playback_stream_id,
            BOBE_POLICY_REASON_NONE
        );
        context->pending_operation =
            BOBE_POLICY_OPERATION_PLAYBACK_READY;
        break;
    case BOBE_POLICY_OPERATION_PLAYBACK_READY:
        if (event->data.effect_result.result == BOBE_POLICY_RESULT_OK) {
            (void)set_turn(context, BOBE_TURN_SPEAKING, effects);
            break;
        }
        (void)append_effect(
            effects,
            BOBE_POLICY_EFFECT_AUDIO_PLAYBACK_ABORT
        );
        (void)append_effect(
            effects,
            BOBE_POLICY_EFFECT_TRANSPORT_CLEAR_PLAYBACK_ROUTE
        );
        ++context->playback_route_generation;
        context->playback_stream_id = 0;
        emit_failure(context, effects, BOBE_POLICY_FAILURE_TIMEOUT);
        reset_active_turn(
            context,
            effects,
            BOBE_POLICY_REASON_NONE,
            false
        );
        break;
    case BOBE_POLICY_OPERATION_PLAYBACK_CLOSE: {
        const uint32_t stream_id = context->pending_playback_stream_id;
        (void)append_effect(
            effects,
            BOBE_POLICY_EFFECT_TRANSPORT_CLEAR_PLAYBACK_ROUTE
        );
        context->playback_stream_id = 0;
        if (event->data.effect_result.result == BOBE_POLICY_RESULT_OK) {
            emit_playback(
                context,
                effects,
                BOBE_POLICY_EFFECT_TRANSPORT_PLAYBACK_DRAINED,
                BOBE_POLICY_OPERATION_PLAYBACK_DRAINED,
                stream_id,
                BOBE_POLICY_REASON_NONE
            );
            context->pending_operation =
                BOBE_POLICY_OPERATION_PLAYBACK_DRAINED;
        } else {
            (void)append_effect(
                effects,
                BOBE_POLICY_EFFECT_AUDIO_PLAYBACK_ABORT
            );
            emit_playback(
                context,
                effects,
                BOBE_POLICY_EFFECT_TRANSPORT_PLAYBACK_FAILED,
                BOBE_POLICY_OPERATION_PLAYBACK_FAILED,
                stream_id,
                event->data.effect_result.result ==
                        BOBE_POLICY_RESULT_TIMEOUT
                    ? BOBE_POLICY_REASON_PHYSICAL_PLAYBACK_TIMEOUT
                    : BOBE_POLICY_REASON_PHYSICAL_PLAYBACK_FAILED
            );
            context->pending_operation =
                BOBE_POLICY_OPERATION_PLAYBACK_FAILED;
        }
        break;
    }
    case BOBE_POLICY_OPERATION_PLAYBACK_DRAINED:
        if (event->data.effect_result.result != BOBE_POLICY_RESULT_OK) {
            emit_failure(context, effects, BOBE_POLICY_FAILURE_TIMEOUT);
            return_to_idle(
                context,
                effects,
                BOBE_POLICY_REASON_NONE
            );
        }
        break;
    case BOBE_POLICY_OPERATION_PLAYBACK_FAILED:
        if (event->data.effect_result.result != BOBE_POLICY_RESULT_OK) {
            emit_failure(context, effects, BOBE_POLICY_FAILURE_TIMEOUT);
        }
        (void)set_turn(context, BOBE_TURN_ABORTED, effects);
        return_to_idle(context, effects, BOBE_POLICY_REASON_NONE);
        break;
    case BOBE_POLICY_OPERATION_TRANSPORT_START:
        handle_transport_start_result(context, event, effects);
        context->pending_workflow = BOBE_POLICY_WORKFLOW_NONE;
        break;
    case BOBE_POLICY_OPERATION_TRANSPORT_STOP:
        handle_transport_stop_result(context, event, effects);
        break;
    case BOBE_POLICY_OPERATION_TRANSPORT_RELOAD:
        context->transport_configured =
            event->data.effect_result.transport_configured;
        if (event->data.effect_result.result != BOBE_POLICY_RESULT_OK) {
            context->pending_workflow = BOBE_POLICY_WORKFLOW_NONE;
            enter_unprovisioned(context, effects);
        } else {
            context->pending_workflow = BOBE_POLICY_WORKFLOW_NONE;
            finish_provisioning_transition(
                context,
                effects,
                event->data.effect_result.network_online
            );
        }
        break;
    case BOBE_POLICY_OPERATION_CONFIGURE_STORED_ENDPOINT:
    case BOBE_POLICY_OPERATION_CONFIGURE_DISCOVERED_ENDPOINT:
        handle_endpoint_result(context, event, effects);
        break;
    case BOBE_POLICY_OPERATION_DISCOVERY_START:
        if (event->data.effect_result.result == BOBE_POLICY_RESULT_OK) {
            (void)set_lifecycle(
                context,
                BOBE_LIFECYCLE_DISCOVERING_HUB,
                effects
            );
        } else {
            context->pending_workflow = BOBE_POLICY_WORKFLOW_NONE;
            (void)set_lifecycle(
                context,
                BOBE_LIFECYCLE_UNPROVISIONED,
                effects
            );
        }
        break;
    case BOBE_POLICY_OPERATION_PROVISIONING_OPEN:
        context->pending_workflow = BOBE_POLICY_WORKFLOW_NONE;
        if (event->data.effect_result.result != BOBE_POLICY_RESULT_OK &&
            context->provisioning_restart_transport) {
            (void)set_lifecycle(
                context,
                BOBE_LIFECYCLE_JOINING_WIFI,
                effects
            );
            (void)set_lifecycle(
                context,
                BOBE_LIFECYCLE_DISCOVERING_HUB,
                effects
            );
            start_transport(context, effects, false);
        }
        break;
    case BOBE_POLICY_OPERATION_OTA_CONFIRM:
        if (event->data.effect_result.result == BOBE_POLICY_RESULT_OK) {
            context->ota_confirm_deadline_us = 0;
        }
        break;
    case BOBE_POLICY_OPERATION_UPDATE_INSTALL:
        if (event->data.effect_result.result == BOBE_POLICY_RESULT_OK) {
            emit_update_state(effects, "rebooting", 100, NULL);
            (void)set_lifecycle(
                context,
                BOBE_LIFECYCLE_REBOOTING,
                effects
            );
            (void)append_effect(
                effects,
                BOBE_POLICY_EFFECT_UPDATE_REBOOT
            );
        } else {
            emit_update_state(effects, "failed", 0, "install_failed");
            (void)set_lifecycle(
                context,
                BOBE_LIFECYCLE_ONLINE,
                effects
            );
            maybe_start_wake_monitor(context, effects);
        }
        break;
    case BOBE_POLICY_OPERATION_NONE:
        break;
    }
    resume_deferred_critical(context, event->now_us, effects);
}

void bobe_policy_init(bobe_policy_context_t *context)
{
    if (context == NULL) {
        return;
    }
    memset(context, 0, sizeof(*context));
    bobe_state_init(&context->state);
}

bool bobe_policy_reduce(
    bobe_policy_context_t *context,
    const bobe_policy_event_t *event,
    bobe_policy_effect_batch_t *effects
)
{
    if (context == NULL || event == NULL || effects == NULL) {
        return false;
    }
    memset(effects, 0, sizeof(*effects));
    const bool preemptive_mute_toggle =
        context->pending_operation != BOBE_POLICY_OPERATION_UPDATE_INSTALL &&
        context->pending_operation != BOBE_POLICY_OPERATION_OTA_CONFIRM &&
        event->kind == BOBE_POLICY_EVENT_BUTTON &&
        event->data.button.event.button == BOBE_BUTTON_VOLUME_DOWN &&
        event->data.button.event.action == BOBE_BUTTON_LONG_PRESS &&
        event->data.button.physical_volume_down_pressed &&
        !event->data.button.physical_action_pressed &&
        !event->data.button.physical_volume_up_pressed;
    if (context->pending_operation != BOBE_POLICY_OPERATION_NONE &&
        event->kind != BOBE_POLICY_EVENT_EFFECT_RESULT &&
        !preemptive_mute_toggle) {
        return true;
    }

    switch (event->kind) {
    case BOBE_POLICY_EVENT_BOOT: {
        context->state.microphone_muted =
            event->data.boot.microphone_muted;
        context->ota_confirm_deadline_us =
            event->data.boot.ota_pending_verification
                ? deadline_after(
                      event->now_us,
                      BOBE_POLICY_OTA_CONFIRM_TIMEOUT_US
                  )
                : 0;
        if (context->state.microphone_muted) {
            bobe_policy_effect_t *const effect = append_effect(
                effects,
                BOBE_POLICY_EFFECT_AUDIO_SET_MUTE
            );
            if (effect != NULL) {
                effect->data.mute.muted = true;
                effect->data.mute.persist = false;
            }
        }
        bobe_policy_effect_t *const volume_effect = append_effect(
            effects,
            BOBE_POLICY_EFFECT_AUDIO_SET_VOLUME
        );
        if (volume_effect != NULL) {
            volume_effect->data.volume = event->data.boot.volume;
        }
        context->theme = event->data.boot.theme;
        if (context->theme != BOBE_FACE_THEME_DEFAULT) {
            bobe_policy_effect_t *const theme_effect = append_effect(
                effects,
                BOBE_POLICY_EFFECT_DISPLAY_THEME
            );
            if (theme_effect != NULL) {
                theme_effect->data.theme.theme = context->theme;
                theme_effect->data.theme.state_version = 0;
            }
        }
        emit_default_face(context, effects);
        queue_control_hint(context);
        break;
    }
    case BOBE_POLICY_EVENT_VOICE_READY:
        maybe_start_wake_monitor(context, effects);
        break;
    case BOBE_POLICY_EVENT_BUTTON:
        handle_button(context, event, effects);
        break;
    case BOBE_POLICY_EVENT_NETWORK:
        handle_network(context, event, effects);
        break;
    case BOBE_POLICY_EVENT_TRANSPORT:
        handle_transport(context, event, effects);
        break;
    case BOBE_POLICY_EVENT_PLAYBACK_PROGRESS:
        handle_playback_progress(context, event, effects);
        break;
    case BOBE_POLICY_EVENT_CRITICAL:
        handle_critical(context, event, effects);
        break;
    case BOBE_POLICY_EVENT_TIME_TICK:
        handle_time_tick(context, event, effects);
        break;
    case BOBE_POLICY_EVENT_HUB_DISCOVERY:
        handle_discovery(context, event, effects);
        break;
    case BOBE_POLICY_EVENT_BATTERY:
        handle_battery(context, event, effects);
        break;
    case BOBE_POLICY_EVENT_EFFECT_RESULT:
        handle_effect_result(context, event, effects);
        break;
    }
    return !effects->overflowed;
}

const char *bobe_policy_reason_name(bobe_policy_reason_t reason)
{
    switch (reason) {
    case BOBE_POLICY_REASON_NONE:
        return NULL;
    case BOBE_POLICY_REASON_BUTTON_RELEASED:
        return "button_released";
    case BOBE_POLICY_REASON_VAD_END:
        return "vad_end";
    case BOBE_POLICY_REASON_AUDIO_START_FAILED:
        return "audio_start_failed";
    case BOBE_POLICY_REASON_CAPTURE_STOP_FAILED:
        return "capture_stop_failed";
    case BOBE_POLICY_REASON_CAPTURE_FRONTEND_DRAIN_FAILED:
        return "capture_frontend_drain_failed";
    case BOBE_POLICY_REASON_UPLINK_BACKPRESSURE:
        return "uplink_backpressure";
    case BOBE_POLICY_REASON_CAPTURE_CLOSE_FAILED:
        return "capture_close_failed";
    case BOBE_POLICY_REASON_REQUEST_TIMEOUT:
        return "request_timeout";
    case BOBE_POLICY_REASON_NETWORK_LOST:
        return "network_lost";
    case BOBE_POLICY_REASON_BODYLINK_DISCONNECTED:
        return "bodylink_disconnected";
    case BOBE_POLICY_REASON_MICROPHONE_MUTED:
        return "microphone_muted";
    case BOBE_POLICY_REASON_PROVISIONING_RECOVERY:
        return "provisioning_recovery";
    case BOBE_POLICY_REASON_SHUTDOWN:
        return "shutdown";
    case BOBE_POLICY_REASON_LOCAL_LEASE_FAILSAFE_EXPIRED:
        return "local_lease_failsafe_expired";
    case BOBE_POLICY_REASON_PHYSICAL_PLAYBACK_TIMEOUT:
        return "physical_playback_timeout";
    case BOBE_POLICY_REASON_PHYSICAL_PLAYBACK_FAILED:
        return "physical_playback_failed";
    }
    return NULL;
}
