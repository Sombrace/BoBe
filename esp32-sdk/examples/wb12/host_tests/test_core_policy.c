#include "bobe_policy.h"

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define CHECK(condition)                                                     \
    do {                                                                     \
        if (!(condition)) {                                                  \
            fprintf(                                                        \
                stderr,                                                     \
                "check failed at %s:%d: %s\n",                              \
                __FILE__,                                                    \
                __LINE__,                                                    \
                #condition                                                   \
            );                                                               \
            exit(1);                                                         \
        }                                                                    \
    } while (0)

static void reduce_event(
    bobe_policy_context_t *context,
    const bobe_policy_event_t *event,
    bobe_policy_effect_batch_t *effects
)
{
    CHECK(bobe_policy_reduce(context, event, effects));
    CHECK(!effects->overflowed);
}

static void check_result_effect_is_last(
    const bobe_policy_effect_batch_t *effects
)
{
    size_t result_count = 0;
    size_t result_index = 0;
    for (size_t index = 0; index < effects->count; ++index) {
        if (effects->items[index].operation !=
            BOBE_POLICY_OPERATION_NONE) {
            ++result_count;
            result_index = index;
        }
    }
    CHECK(result_count <= 1);
    CHECK(result_count == 0 || result_index + 1U == effects->count);
}

static bool has_effect(
    const bobe_policy_effect_batch_t *effects,
    bobe_policy_effect_kind_t kind
)
{
    for (size_t index = 0; index < effects->count; ++index) {
        if (effects->items[index].kind == kind) {
            return true;
        }
    }
    return false;
}

static void init_online(bobe_policy_context_t *context)
{
    bobe_policy_init(context);
    context->state.lifecycle = BOBE_LIFECYCLE_ONLINE;
    context->state.turn = BOBE_TURN_IDLE;
    context->network_online = true;
    context->transport_connected = true;
    context->transport_configured = true;
    context->network_generation = 3;
    context->transport_generation = 7;
}

static bobe_policy_event_t action_button(
    bobe_button_action_t action,
    uint32_t event_id,
    uint32_t latest_release
)
{
    bobe_policy_event_t event = {
        .kind = BOBE_POLICY_EVENT_BUTTON,
        .now_us = UINT64_C(1000),
        .data.button = {
            .event = {
                .button = BOBE_BUTTON_ACTION,
                .action = action,
                .event_id = event_id,
                .monotonic_us = UINT64_C(900),
            },
            .physical_action_pressed = action != BOBE_BUTTON_RELEASED,
            .transport_connected = true,
            .transport_configured = true,
            .latest_action_release_event_id = latest_release,
            .capture_seed = {
                .valid = true,
                .stream_id = 41,
                .request_id = "request-41",
            },
        },
    };
    return event;
}

static bobe_policy_event_t volume_down_long_press(uint32_t event_id)
{
    bobe_policy_event_t event = {
        .kind = BOBE_POLICY_EVENT_BUTTON,
        .now_us = UINT64_C(1000),
        .data.button = {
            .event = {
                .button = BOBE_BUTTON_VOLUME_DOWN,
                .action = BOBE_BUTTON_LONG_PRESS,
                .event_id = event_id,
                .monotonic_us = UINT64_C(900),
            },
            .physical_volume_down_pressed = true,
            .transport_connected = true,
            .transport_configured = true,
        },
    };
    return event;
}

static bobe_policy_event_t control_event(
    bobe_control_message_t control,
    uint64_t now_us
)
{
    bobe_policy_event_t event = {
        .kind = BOBE_POLICY_EVENT_TRANSPORT,
        .now_us = now_us,
        .data.transport = {
            .kind = BOBE_POLICY_TRANSPORT_CONTROL,
            .generation = 7,
            .current_generation = 7,
            .control = control,
        },
    };
    return event;
}

static void complete_operation(
    bobe_policy_context_t *context,
    bobe_policy_result_t result,
    uint64_t now_us,
    uint32_t retry_random,
    bobe_policy_effect_batch_t *effects
)
{
    const bobe_policy_operation_t operation =
        context->pending_operation;
    CHECK(operation != BOBE_POLICY_OPERATION_NONE);
    const bobe_policy_event_t event = {
        .kind = BOBE_POLICY_EVENT_EFFECT_RESULT,
        .now_us = now_us,
        .data.effect_result = {
            .operation = operation,
            .result = result,
            .retry_random = retry_random,
            .transport_generation = 7,
            .transport_connected = true,
            .transport_configured = true,
            .network_online = true,
        },
    };
    reduce_event(context, &event, effects);
    check_result_effect_is_last(effects);
}

static void begin_button_capture(
    bobe_policy_context_t *context,
    bobe_policy_effect_batch_t *effects
)
{
    bobe_policy_event_t press = action_button(BOBE_BUTTON_PRESSED, 10, 0);
    reduce_event(context, &press, effects);
    CHECK(context->state.turn == BOBE_TURN_REQUESTING_CAPTURE);
    CHECK(
        context->pending_operation ==
        BOBE_POLICY_OPERATION_CAPTURE_REQUEST
    );
    CHECK(effects->count == 2);
    CHECK(effects->items[0].kind == BOBE_POLICY_EFFECT_DISPLAY_FACE);
    CHECK(
        effects->items[1].kind ==
        BOBE_POLICY_EFFECT_TRANSPORT_CAPTURE_REQUEST
    );
    check_result_effect_is_last(effects);

    complete_operation(
        context,
        BOBE_POLICY_RESULT_OK,
        UINT64_C(1100),
        0,
        effects
    );
    CHECK(
        context->capture_request_deadline_us ==
        UINT64_C(10001100)
    );
    CHECK(effects->count == 1);
    CHECK(
        effects->items[0].kind ==
        BOBE_POLICY_EFFECT_TRANSPORT_BUTTON
    );
}

static bobe_control_message_t capture_control(
    bobe_control_kind_t kind,
    uint32_t stream_id,
    const char *request_id
)
{
    bobe_control_message_t control = {
        .kind = kind,
        .data.capture = {
            .stream_id = stream_id,
        },
    };
    strcpy(control.data.capture.request_id, request_id);
    strcpy(control.data.capture.lease_id, "lease-1");
    strcpy(control.data.capture.turn_id, "turn-1");
    return control;
}

static void grant_capture(
    bobe_policy_context_t *context,
    bobe_policy_effect_batch_t *effects
)
{
    const bool finishes_immediately =
        context->wake_speech_ended_pending;
    const bobe_control_message_t grant = capture_control(
        BOBE_CONTROL_CAPTURE_GRANTED,
        41,
        "request-41"
    );
    const bobe_policy_event_t event =
        control_event(grant, UINT64_C(2000));
    reduce_event(context, &event, effects);
    CHECK(
        context->pending_operation ==
        BOBE_POLICY_OPERATION_CAPTURE_OPEN
    );
    CHECK(effects->count == 1);
    CHECK(
        effects->items[0].kind ==
        BOBE_POLICY_EFFECT_TRANSPORT_CAPTURE_STATE
    );
    CHECK(effects->items[0].data.capture_state.open);
    CHECK(
        context->lease_deadline_us ==
        UINT64_C(600002000)
    );

    complete_operation(
        context,
        BOBE_POLICY_RESULT_OK,
        UINT64_C(2010),
        0,
        effects
    );
    CHECK(
        context->pending_operation ==
        BOBE_POLICY_OPERATION_VOICE_BEGIN
    );
    CHECK(
        effects->items[0].kind ==
        BOBE_POLICY_EFFECT_VOICE_STREAM_BEGIN
    );

    complete_operation(
        context,
        BOBE_POLICY_RESULT_OK,
        UINT64_C(2020),
        0,
        effects
    );
    CHECK(
        context->pending_operation ==
        BOBE_POLICY_OPERATION_CAPTURE_START
    );
    CHECK(
        effects->items[0].kind ==
        BOBE_POLICY_EFFECT_AUDIO_CAPTURE_START
    );

    complete_operation(
        context,
        BOBE_POLICY_RESULT_OK,
        UINT64_C(2030),
        0,
        effects
    );
    CHECK(context->state.turn == BOBE_TURN_LISTENING);
    CHECK(effects->count == (finishes_immediately ? 3U : 2U));
    CHECK(
        effects->items[0].kind ==
        BOBE_POLICY_EFFECT_DISPLAY_MICROPHONE
    );
    CHECK(effects->items[0].data.microphone_active);
    CHECK(effects->items[1].kind == BOBE_POLICY_EFFECT_DISPLAY_FACE);
}

static void test_capture_grant_deny_release(void)
{
    bobe_policy_context_t context;
    bobe_policy_effect_batch_t effects;
    init_online(&context);
    begin_button_capture(&context, &effects);

    const struct {
        uint32_t stream_id;
        const char *request_id;
        bool accepted;
    } grants[] = {
        {99, "request-41", false},
        {41, "stale-request", false},
        {41, "request-41", true},
    };
    for (size_t index = 0;
         index < sizeof(grants) / sizeof(grants[0]);
         ++index) {
        const bobe_control_message_t grant = capture_control(
            BOBE_CONTROL_CAPTURE_GRANTED,
            grants[index].stream_id,
            grants[index].request_id
        );
        const bobe_policy_event_t event =
            control_event(grant, UINT64_C(2000));
        reduce_event(&context, &event, &effects);
        CHECK((effects.count != 0) == grants[index].accepted);
    }
    CHECK(
        context.pending_operation ==
        BOBE_POLICY_OPERATION_CAPTURE_OPEN
    );

    complete_operation(
        &context,
        BOBE_POLICY_RESULT_OK,
        UINT64_C(2010),
        0,
        &effects
    );
    complete_operation(
        &context,
        BOBE_POLICY_RESULT_OK,
        UINT64_C(2020),
        0,
        &effects
    );
    complete_operation(
        &context,
        BOBE_POLICY_RESULT_OK,
        UINT64_C(2030),
        0,
        &effects
    );
    CHECK(context.state.turn == BOBE_TURN_LISTENING);

    bobe_policy_event_t release =
        action_button(BOBE_BUTTON_RELEASED, 11, 11);
    release.data.button.physical_action_pressed = false;
    reduce_event(&context, &release, &effects);
    CHECK(
        context.pending_operation ==
        BOBE_POLICY_OPERATION_CAPTURE_STOP
    );
    CHECK(effects.count == 1);
    CHECK(
        effects.items[0].kind ==
        BOBE_POLICY_EFFECT_AUDIO_CAPTURE_STOP
    );

    complete_operation(
        &context,
        BOBE_POLICY_RESULT_OK,
        UINT64_C(2100),
        0,
        &effects
    );
    CHECK(effects.count == 2);
    CHECK(
        effects.items[0].kind ==
        BOBE_POLICY_EFFECT_DISPLAY_MICROPHONE
    );
    CHECK(
        effects.items[1].kind ==
        BOBE_POLICY_EFFECT_VOICE_STREAM_END
    );
    complete_operation(
        &context,
        BOBE_POLICY_RESULT_OK,
        UINT64_C(2110),
        0,
        &effects
    );
    CHECK(
        effects.items[0].kind ==
        BOBE_POLICY_EFFECT_TRANSPORT_FLUSH_AUDIO
    );
    complete_operation(
        &context,
        BOBE_POLICY_RESULT_OK,
        UINT64_C(2120),
        0,
        &effects
    );
    CHECK(
        effects.items[0].kind ==
        BOBE_POLICY_EFFECT_TRANSPORT_CAPTURE_STATE
    );
    CHECK(!effects.items[0].data.capture_state.open);
    CHECK(
        effects.items[0].data.capture_state.reason ==
        BOBE_POLICY_REASON_BUTTON_RELEASED
    );
    complete_operation(
        &context,
        BOBE_POLICY_RESULT_OK,
        UINT64_C(2130),
        0,
        &effects
    );
    CHECK(context.state.turn == BOBE_TURN_WAITING_FOR_RUNTIME);
    CHECK(effects.count == 2);
    CHECK(effects.items[0].kind == BOBE_POLICY_EFFECT_DISPLAY_FACE);
    CHECK(
        effects.items[1].kind ==
        BOBE_POLICY_EFFECT_TRANSPORT_BUTTON
    );

    init_online(&context);
    begin_button_capture(&context, &effects);
    const bobe_control_message_t denied = capture_control(
        BOBE_CONTROL_CAPTURE_DENIED,
        41,
        "request-41"
    );
    const bobe_policy_event_t deny_event =
        control_event(denied, UINT64_C(3000));
    reduce_event(&context, &deny_event, &effects);
    CHECK(context.state.turn == BOBE_TURN_IDLE);
    CHECK(context.capture_stream_id == 0);
    CHECK(effects.count == 2);
}

static void init_waiting_turn(bobe_policy_context_t *context)
{
    init_online(context);
    context->state.turn = BOBE_TURN_WAITING_FOR_RUNTIME;
    context->capture_stream_id = 41;
    strcpy(context->lease_id, "lease-1");
    strcpy(context->turn_id, "turn-1");
}

static bobe_control_message_t playback_open_control(void)
{
    bobe_control_message_t control = {
        .kind = BOBE_CONTROL_PLAYBACK_OPEN,
        .data.playback_open = {
            .stream_id = 77,
        },
    };
    strcpy(control.data.playback_open.lease_id, "lease-1");
    strcpy(control.data.playback_open.turn_id, "turn-1");
    return control;
}

static void test_playback_open_ack_close(void)
{
    bobe_policy_context_t context;
    bobe_policy_effect_batch_t effects;
    init_waiting_turn(&context);
    const bobe_control_message_t open = playback_open_control();
    bobe_policy_event_t event = control_event(open, UINT64_C(4000));
    reduce_event(&context, &event, &effects);
    CHECK(effects.count == 2);
    CHECK(
        effects.items[0].kind ==
        BOBE_POLICY_EFFECT_WAKE_MONITOR_STOP
    );
    CHECK(
        effects.items[1].kind ==
        BOBE_POLICY_EFFECT_AUDIO_PLAYBACK_OPEN
    );
    CHECK(
        context.pending_operation ==
        BOBE_POLICY_OPERATION_PLAYBACK_OPEN
    );

    complete_operation(
        &context,
        BOBE_POLICY_RESULT_OK,
        UINT64_C(4010),
        0,
        &effects
    );
    CHECK(context.playback_stream_id == 77);
    CHECK(context.playback_route_generation == 1);
    CHECK(effects.count == 2);
    CHECK(
        effects.items[0].kind ==
        BOBE_POLICY_EFFECT_TRANSPORT_SET_PLAYBACK_ROUTE
    );
    CHECK(
        effects.items[1].kind ==
        BOBE_POLICY_EFFECT_TRANSPORT_PLAYBACK_READY
    );
    complete_operation(
        &context,
        BOBE_POLICY_RESULT_OK,
        UINT64_C(4020),
        0,
        &effects
    );
    CHECK(context.state.turn == BOBE_TURN_SPEAKING);

    event = (bobe_policy_event_t){
        .kind = BOBE_POLICY_EVENT_PLAYBACK_PROGRESS,
        .now_us = UINT64_C(4100),
        .data.playback_progress = {
            .stream_id = 77,
            .route_generation = 1,
            .played_samples = UINT64_C(2400),
            .credit_ms = 80,
        },
    };
    reduce_event(&context, &event, &effects);
    CHECK(effects.count == 1);
    CHECK(
        effects.items[0].kind ==
        BOBE_POLICY_EFFECT_TRANSPORT_PLAYBACK_ACK
    );
    CHECK(effects.items[0].data.playback.played_samples == 2400);

    event.data.playback_progress.route_generation = 0;
    reduce_event(&context, &event, &effects);
    CHECK(effects.count == 0);

    bobe_control_message_t close = {
        .kind = BOBE_CONTROL_PLAYBACK_CLOSE,
        .data.playback_close = {
            .stream_id = 77,
        },
    };
    strcpy(close.data.playback_close.lease_id, "lease-1");
    strcpy(close.data.playback_close.turn_id, "turn-1");
    event = control_event(close, UINT64_C(4200));
    reduce_event(&context, &event, &effects);
    CHECK(
        context.pending_operation ==
        BOBE_POLICY_OPERATION_PLAYBACK_CLOSE
    );
    CHECK(
        effects.items[0].kind ==
        BOBE_POLICY_EFFECT_AUDIO_PLAYBACK_CLOSE
    );
    complete_operation(
        &context,
        BOBE_POLICY_RESULT_OK,
        UINT64_C(4210),
        0,
        &effects
    );
    CHECK(context.playback_stream_id == 0);
    CHECK(effects.count == 2);
    CHECK(
        effects.items[0].kind ==
        BOBE_POLICY_EFFECT_TRANSPORT_CLEAR_PLAYBACK_ROUTE
    );
    CHECK(
        effects.items[1].kind ==
        BOBE_POLICY_EFFECT_TRANSPORT_PLAYBACK_DRAINED
    );
    complete_operation(
        &context,
        BOBE_POLICY_RESULT_OK,
        UINT64_C(4220),
        0,
        &effects
    );
    CHECK(effects.count == 0);
    CHECK(context.state.turn == BOBE_TURN_SPEAKING);

    init_waiting_turn(&context);
    event = control_event(open, UINT64_C(4300));
    event.data.transport.audio_capturing = true;
    reduce_event(&context, &event, &effects);
    CHECK(effects.count == 0);
    CHECK(context.pending_operation == BOBE_POLICY_OPERATION_NONE);
}

static void test_timeouts_and_stale_generations(void)
{
    bobe_policy_context_t context;
    bobe_policy_effect_batch_t effects;
    init_online(&context);
    begin_button_capture(&context, &effects);
    const bobe_policy_event_t tick = {
        .kind = BOBE_POLICY_EVENT_TIME_TICK,
        .now_us = context.capture_request_deadline_us,
        .data.tick = {
            .network_online = true,
        },
    };
    reduce_event(&context, &tick, &effects);
    CHECK(context.state.turn == BOBE_TURN_IDLE);
    CHECK(context.capture_stream_id == 0);
    CHECK(
        effects.items[0].kind ==
        BOBE_POLICY_EFFECT_TRANSPORT_CAPTURE_CANCEL
    );
    CHECK(has_effect(&effects, BOBE_POLICY_EFFECT_WAKE_MONITOR_START));

    const struct {
        uint32_t generation;
        uint32_t current_generation;
        bool accepted;
    } transport_cases[] = {
        {6, 7, false},
        {7, 7, true},
    };
    for (size_t index = 0;
         index < sizeof(transport_cases) / sizeof(transport_cases[0]);
         ++index) {
        init_online(&context);
        bobe_control_message_t volume = {
            .kind = BOBE_CONTROL_VOLUME_SET,
            .data.volume = {.percent = 55},
        };
        bobe_policy_event_t event =
            control_event(volume, UINT64_C(5000));
        event.data.transport.generation =
            transport_cases[index].generation;
        event.data.transport.current_generation =
            transport_cases[index].current_generation;
        reduce_event(&context, &event, &effects);
        CHECK(
            has_effect(
                &effects,
                BOBE_POLICY_EFFECT_AUDIO_SET_VOLUME
            ) == transport_cases[index].accepted
        );
    }

    bobe_policy_init(&context);
    const bobe_policy_event_t stale_network = {
        .kind = BOBE_POLICY_EVENT_NETWORK,
        .now_us = UINT64_C(5100),
        .data.network = {
            .state = BOBE_NETWORK_CONNECTED,
            .generation = 2,
            .current_generation = 3,
            .network_online = true,
            .transport_configured = true,
        },
    };
    reduce_event(&context, &stale_network, &effects);
    CHECK(context.state.lifecycle == BOBE_LIFECYCLE_BOOTING);
    CHECK(effects.count == 0);
}

static void test_button_release_fencing(void)
{
    bobe_policy_context_t context;
    bobe_policy_effect_batch_t effects;
    init_online(&context);

    bobe_policy_event_t fenced_press =
        action_button(BOBE_BUTTON_PRESSED, 10, 11);
    reduce_event(&context, &fenced_press, &effects);
    CHECK(context.state.turn == BOBE_TURN_IDLE);
    CHECK(
        !has_effect(
            &effects,
            BOBE_POLICY_EFFECT_TRANSPORT_CAPTURE_REQUEST
        )
    );

    init_online(&context);
    begin_button_capture(&context, &effects);
    const bobe_policy_event_t stale_release = {
        .kind = BOBE_POLICY_EVENT_CRITICAL,
        .now_us = UINT64_C(5200),
        .data.critical = {
            .flags = BOBE_POLICY_CRITICAL_ACTION_RELEASE,
            .latest_action_release_event_id = 9,
            .network_online = true,
            .transport_connected = true,
            .transport_configured = true,
        },
    };
    reduce_event(&context, &stale_release, &effects);
    CHECK(context.state.turn == BOBE_TURN_REQUESTING_CAPTURE);
    CHECK(context.pending_operation == BOBE_POLICY_OPERATION_NONE);
    CHECK(effects.count == 0);

    bobe_policy_event_t release_then_wake = {
        .kind = BOBE_POLICY_EVENT_CRITICAL,
        .now_us = UINT64_C(5300),
        .data.critical = {
            .flags = BOBE_POLICY_CRITICAL_ACTION_RELEASE |
                     BOBE_POLICY_CRITICAL_WAKE_DETECTED,
            .latest_action_release_event_id = 11,
            .network_online = true,
            .transport_connected = true,
            .transport_configured = true,
            .capture_seed = {
                .valid = true,
                .stream_id = 42,
                .request_id = "request-42",
            },
        },
    };
    reduce_event(&context, &release_then_wake, &effects);
    CHECK(
        context.pending_operation ==
        BOBE_POLICY_OPERATION_CAPTURE_STOP
    );
    CHECK(
        context.deferred_critical.flags ==
        BOBE_POLICY_CRITICAL_WAKE_DETECTED
    );
    complete_operation(
        &context,
        BOBE_POLICY_RESULT_OK,
        UINT64_C(5310),
        0,
        &effects
    );
    complete_operation(
        &context,
        BOBE_POLICY_RESULT_OK,
        UINT64_C(5320),
        0,
        &effects
    );
    CHECK(context.state.turn == BOBE_TURN_REQUESTING_CAPTURE);
    CHECK(context.capture_stream_id == 42);
    CHECK(
        context.pending_operation ==
        BOBE_POLICY_OPERATION_CAPTURE_REQUEST
    );
    CHECK(
        effects.items[effects.count - 1U].kind ==
        BOBE_POLICY_EFFECT_TRANSPORT_CAPTURE_REQUEST
    );
}

static void test_retry_jitter_boundaries(void)
{
    const struct {
        uint8_t attempt;
        uint32_t random;
        uint8_t expected_attempt;
        uint32_t expected_delay_ms;
    } cases[] = {
        {0, 0, 1, 500},
        {0, 500, 1, 1000},
        {5, 0, 6, 500},
        {5, 29500, 6, 30000},
        {6, 29500, 6, 30000},
    };
    for (size_t index = 0;
         index < sizeof(cases) / sizeof(cases[0]);
         ++index) {
        bobe_policy_context_t context;
        bobe_policy_effect_batch_t effects;
        init_online(&context);
        context.state.lifecycle = BOBE_LIFECYCLE_RECONNECTING;
        context.transport_retry_attempt = cases[index].attempt;
        context.transport_retry_deadline_us = UINT64_C(6000);
        const bobe_policy_event_t tick = {
            .kind = BOBE_POLICY_EVENT_TIME_TICK,
            .now_us = UINT64_C(6000),
            .data.tick = {
                .network_online = true,
            },
        };
        reduce_event(&context, &tick, &effects);
        CHECK(
            context.pending_operation ==
            BOBE_POLICY_OPERATION_TRANSPORT_START
        );
        CHECK(
            effects.items[effects.count - 1U].kind ==
            BOBE_POLICY_EFFECT_TRANSPORT_START
        );
        complete_operation(
            &context,
            BOBE_POLICY_RESULT_FAILED,
            UINT64_C(6000),
            cases[index].random,
            &effects
        );
        CHECK(
            context.transport_retry_attempt ==
            cases[index].expected_attempt
        );
        CHECK(
            context.transport_retry_deadline_us ==
            UINT64_C(6000) +
                (uint64_t)cases[index].expected_delay_ms * UINT64_C(1000)
        );
    }
}

static void test_destructive_chord_and_critical_order(void)
{
    bobe_policy_context_t context;
    bobe_policy_effect_batch_t effects;
    init_online(&context);
    bobe_policy_event_t chord = action_button(
        BOBE_BUTTON_LONG_PRESS,
        20,
        0
    );
    chord.data.button.physical_volume_down_pressed = true;
    chord.data.button.physical_action_pressed = false;
    reduce_event(&context, &chord, &effects);
    CHECK(effects.count == 0);
    CHECK(context.state.lifecycle == BOBE_LIFECYCLE_ONLINE);

    chord.data.button.physical_action_pressed = true;
    reduce_event(&context, &chord, &effects);
    CHECK(
        context.state.lifecycle ==
        BOBE_LIFECYCLE_SHUTTING_DOWN
    );
    CHECK(has_effect(&effects, BOBE_POLICY_EFFECT_BOARD_SHUTDOWN));
    CHECK(
        !has_effect(
            &effects,
            BOBE_POLICY_EFFECT_TRANSPORT_CAPTURE_REQUEST
        )
    );

    init_online(&context);
    const bobe_policy_event_t authoritative_shutdown = {
        .kind = BOBE_POLICY_EVENT_CRITICAL,
        .now_us = UINT64_C(7050),
        .data.critical = {
            .flags = BOBE_POLICY_CRITICAL_SHUTDOWN,
            .physical_action_pressed = false,
            .physical_volume_down_pressed = false,
            .network_online = true,
            .transport_connected = true,
            .transport_configured = true,
        },
    };
    reduce_event(&context, &authoritative_shutdown, &effects);
    CHECK(
        context.state.lifecycle ==
        BOBE_LIFECYCLE_SHUTTING_DOWN
    );
    CHECK(has_effect(&effects, BOBE_POLICY_EFFECT_BOARD_SHUTDOWN));

    init_online(&context);
    const bobe_policy_event_t network_before_wake = {
        .kind = BOBE_POLICY_EVENT_CRITICAL,
        .now_us = UINT64_C(7000),
        .data.critical = {
            .flags = BOBE_POLICY_CRITICAL_NETWORK_LOST |
                     BOBE_POLICY_CRITICAL_WAKE_DETECTED,
            .network_online = false,
            .transport_connected = true,
            .transport_configured = true,
            .capture_seed = {
                .valid = true,
                .stream_id = 90,
                .request_id = "wake-90",
            },
        },
    };
    reduce_event(&context, &network_before_wake, &effects);
    CHECK(context.state.lifecycle == BOBE_LIFECYCLE_JOINING_WIFI);
    CHECK(context.state.turn == BOBE_TURN_IDLE);
    CHECK(
        !has_effect(
            &effects,
            BOBE_POLICY_EFFECT_TRANSPORT_CAPTURE_REQUEST
        )
    );
}

static void test_effect_failure_refreshes_transport_state(void)
{
    bobe_policy_context_t context;
    bobe_policy_effect_batch_t effects;
    init_online(&context);
    begin_button_capture(&context, &effects);
    const bobe_control_message_t grant = capture_control(
        BOBE_CONTROL_CAPTURE_GRANTED,
        41,
        "request-41"
    );
    const bobe_policy_event_t grant_event =
        control_event(grant, UINT64_C(7100));
    reduce_event(&context, &grant_event, &effects);
    CHECK(
        context.pending_operation ==
        BOBE_POLICY_OPERATION_CAPTURE_OPEN
    );

    const bobe_policy_event_t failed = {
        .kind = BOBE_POLICY_EVENT_EFFECT_RESULT,
        .now_us = UINT64_C(7110),
        .data.effect_result = {
            .operation = BOBE_POLICY_OPERATION_CAPTURE_OPEN,
            .result = BOBE_POLICY_RESULT_FAILED,
            .transport_generation = 7,
            .transport_connected = false,
            .transport_configured = true,
            .network_online = true,
        },
    };
    reduce_event(&context, &failed, &effects);
    CHECK(!context.transport_connected);
    CHECK(!has_effect(&effects, BOBE_POLICY_EFFECT_WAKE_MONITOR_START));
}

static void test_wake_vad_ordering(void)
{
    bobe_policy_context_t context;
    bobe_policy_effect_batch_t effects;
    init_online(&context);
    const bobe_policy_event_t wake_and_end = {
        .kind = BOBE_POLICY_EVENT_CRITICAL,
        .now_us = UINT64_C(8000),
        .data.critical = {
            .flags = BOBE_POLICY_CRITICAL_WAKE_DETECTED |
                     BOBE_POLICY_CRITICAL_SPEECH_ENDED,
            .network_online = true,
            .transport_connected = true,
            .transport_configured = true,
            .capture_seed = {
                .valid = true,
                .stream_id = 41,
                .request_id = "request-41",
            },
        },
    };
    reduce_event(&context, &wake_and_end, &effects);
    CHECK(context.state.turn == BOBE_TURN_REQUESTING_CAPTURE);
    CHECK(context.wake_speech_ended_pending);
    CHECK(
        effects.items[effects.count - 1U].kind ==
        BOBE_POLICY_EFFECT_TRANSPORT_CAPTURE_REQUEST
    );

    complete_operation(
        &context,
        BOBE_POLICY_RESULT_OK,
        UINT64_C(8010),
        0,
        &effects
    );
    grant_capture(&context, &effects);
    CHECK(context.state.turn == BOBE_TURN_LISTENING);
    CHECK(
        context.pending_operation ==
        BOBE_POLICY_OPERATION_CAPTURE_STOP
    );
    CHECK(effects.count == 3);
    CHECK(
        effects.items[0].kind ==
        BOBE_POLICY_EFFECT_DISPLAY_MICROPHONE
    );
    CHECK(effects.items[1].kind == BOBE_POLICY_EFFECT_DISPLAY_FACE);
    CHECK(
        effects.items[2].kind ==
        BOBE_POLICY_EFFECT_AUDIO_CAPTURE_STOP
    );
}

static void test_network_and_provisioning_cancellation(void)
{
    bobe_policy_context_t context;
    bobe_policy_effect_batch_t effects;
    init_online(&context);
    context.transport_retry_deadline_us = UINT64_C(9000);
    context.transport_configured = false;
    const bobe_policy_event_t recovery = {
        .kind = BOBE_POLICY_EVENT_CRITICAL,
        .now_us = UINT64_C(8500),
        .data.critical = {
            .flags = BOBE_POLICY_CRITICAL_PROVISIONING_RECOVERY,
            .physical_volume_down_pressed = true,
            .physical_volume_up_pressed = true,
            .network_online = true,
            .transport_connected = true,
            .transport_configured = false,
        },
    };
    reduce_event(&context, &recovery, &effects);
    CHECK(context.transport_retry_deadline_us == 0);
    CHECK(context.state.lifecycle == BOBE_LIFECYCLE_UNPROVISIONED);
    CHECK(
        context.pending_operation ==
        BOBE_POLICY_OPERATION_PROVISIONING_OPEN
    );
    CHECK(
        effects.items[effects.count - 1U].kind ==
        BOBE_POLICY_EFFECT_CONNECTIVITY_OPEN_PROVISIONING
    );

    complete_operation(
        &context,
        BOBE_POLICY_RESULT_OK,
        UINT64_C(8510),
        0,
        &effects
    );
    CHECK(context.pending_operation == BOBE_POLICY_OPERATION_NONE);

    init_online(&context);
    context.state.turn = BOBE_TURN_LISTENING;
    context.capture_stream_id = 5;
    strcpy(context.lease_id, "lease");
    strcpy(context.turn_id, "turn");
    const bobe_policy_event_t lost_during_provisioning = {
        .kind = BOBE_POLICY_EVENT_CRITICAL,
        .now_us = UINT64_C(8600),
        .data.critical = {
            .flags = BOBE_POLICY_CRITICAL_NETWORK_LOST,
            .provisioning = true,
            .transport_connected = true,
            .transport_configured = true,
        },
    };
    reduce_event(&context, &lost_during_provisioning, &effects);
    CHECK(context.state.turn == BOBE_TURN_IDLE);
    CHECK(context.state.lifecycle == BOBE_LIFECYCLE_ONLINE);
    CHECK(context.capture_stream_id == 0);
}

static void test_queue_and_effect_bounds(void)
{
    CHECK(BOBE_POLICY_EVENT_QUEUE_CAPACITY == 16U);
    CHECK(BOBE_POLICY_MAX_EFFECTS == 16U);
    CHECK(
        sizeof(bobe_policy_event_t) <= BOBE_POLICY_EVENT_MAX_BYTES
    );
    CHECK(
        sizeof(bobe_policy_effect_t) <= BOBE_POLICY_EFFECT_MAX_BYTES
    );
    CHECK(
        sizeof(bobe_policy_context_t) <=
        BOBE_POLICY_CONTEXT_MAX_BYTES
    );
    CHECK(
        sizeof(bobe_policy_effect_batch_t) <=
        BOBE_POLICY_EFFECT_BATCH_MAX_BYTES
    );

    bobe_policy_context_t context;
    bobe_policy_effect_batch_t effects;
    init_online(&context);
    context.state.turn = BOBE_TURN_SPEAKING;
    context.capture_stream_id = 1;
    context.playback_stream_id = 2;
    strcpy(context.lease_id, "lease");
    strcpy(context.turn_id, "turn");
    const bobe_policy_event_t shutdown = {
        .kind = BOBE_POLICY_EVENT_CRITICAL,
        .now_us = UINT64_C(9000),
        .data.critical = {
            .flags = BOBE_POLICY_CRITICAL_SHUTDOWN |
                     BOBE_POLICY_CRITICAL_WAKE_DETECTED |
                     BOBE_POLICY_CRITICAL_SPEECH_ENDED,
            .physical_action_pressed = true,
            .physical_volume_down_pressed = true,
            .network_online = true,
            .transport_connected = true,
            .transport_configured = true,
            .capture_seed = {
                .valid = true,
                .stream_id = 3,
                .request_id = "ignored",
            },
        },
    };
    reduce_event(&context, &shutdown, &effects);
    CHECK(effects.count <= BOBE_POLICY_MAX_EFFECTS);
    CHECK(has_effect(&effects, BOBE_POLICY_EFFECT_BOARD_SHUTDOWN));
    CHECK(
        !has_effect(
            &effects,
            BOBE_POLICY_EFFECT_TRANSPORT_CAPTURE_REQUEST
        )
    );
    check_result_effect_is_last(&effects);

    init_online(&context);
    context.state.turn = BOBE_TURN_LISTENING;
    context.capture_stream_id = 1;
    context.playback_stream_id = 2;
    strcpy(context.lease_id, "lease");
    strcpy(context.turn_id, "turn");
    const bobe_policy_event_t maximum_batch = {
        .kind = BOBE_POLICY_EVENT_CRITICAL,
        .now_us = UINT64_C(9100),
        .data.critical = {
            .flags = BOBE_POLICY_CRITICAL_PLAYBACK_FAILED |
                     BOBE_POLICY_CRITICAL_CAPTURE_FAILED |
                     BOBE_POLICY_CRITICAL_WAKE_DETECTED |
                     BOBE_POLICY_CRITICAL_SPEECH_ENDED,
            .network_online = true,
            .transport_connected = true,
            .transport_configured = true,
            .capture_seed = {
                .valid = true,
                .stream_id = 3,
                .request_id = "request-3",
            },
        },
    };
    reduce_event(&context, &maximum_batch, &effects);
    CHECK(effects.count < BOBE_POLICY_MAX_EFFECTS);
    CHECK(
        !has_effect(
            &effects,
            BOBE_POLICY_EFFECT_WAKE_MONITOR_START
        )
    );
    CHECK(
        effects.items[effects.count - 1U].kind ==
        BOBE_POLICY_EFFECT_TRANSPORT_CAPTURE_REQUEST
    );
    check_result_effect_is_last(&effects);
}

static bobe_control_message_t theme_control(
    bobe_face_theme_id_t theme,
    uint64_t state_version
)
{
    bobe_control_message_t control = {
        .kind = BOBE_CONTROL_THEME_SET,
        .data.theme = {
            .state_version = state_version,
            .theme = theme,
        },
    };
    return control;
}

static const bobe_policy_effect_t *find_effect(
    const bobe_policy_effect_batch_t *effects,
    bobe_policy_effect_kind_t kind
)
{
    for (size_t index = 0; index < effects->count; ++index) {
        if (effects->items[index].kind == kind) {
            return &effects->items[index];
        }
    }
    return NULL;
}

static bobe_control_message_t face_control(
    bobe_face_expression_t expression,
    uint64_t state_version
)
{
    bobe_control_message_t control = {
        .kind = BOBE_CONTROL_FACE_SET,
        .data.face = {
            .expression = expression,
            .state_version = state_version,
            .accent_rgb = BOBE_FACE_THEME_ACCENT_RGB,
        },
    };
    return control;
}

static void test_microphone_mute_toggle(void)
{
    bobe_policy_context_t context;
    bobe_policy_effect_batch_t effects;
    init_online(&context);

    bobe_policy_event_t hold = volume_down_long_press(11);
    reduce_event(&context, &hold, &effects);
    CHECK(context.state.microphone_muted);
    CHECK(context.state.turn == BOBE_TURN_IDLE);
    const bobe_policy_effect_t *effect =
        find_effect(&effects, BOBE_POLICY_EFFECT_AUDIO_SET_MUTE);
    CHECK(effect != NULL);
    CHECK(effect->data.mute.muted);
    CHECK(effect->data.mute.persist);
    CHECK(has_effect(&effects, BOBE_POLICY_EFFECT_WAKE_MONITOR_STOP));
    CHECK(!has_effect(&effects, BOBE_POLICY_EFFECT_WAKE_MONITOR_START));
    effect = find_effect(&effects, BOBE_POLICY_EFFECT_DISPLAY_FACE);
    CHECK(effect != NULL);
    CHECK(effect->data.face.expression == BOBE_FACE_MUTED);
    CHECK(strcmp(effect->data.face.caption, "Microphone muted") == 0);

    bobe_policy_event_t press =
        action_button(BOBE_BUTTON_PRESSED, 12, 0);
    reduce_event(&context, &press, &effects);
    CHECK(context.state.turn == BOBE_TURN_IDLE);
    CHECK(!has_effect(
        &effects,
        BOBE_POLICY_EFFECT_TRANSPORT_CAPTURE_REQUEST
    ));

    bobe_policy_event_t wake = {
        .kind = BOBE_POLICY_EVENT_CRITICAL,
        .data.critical = {
            .flags = BOBE_POLICY_CRITICAL_WAKE_DETECTED,
            .transport_connected = true,
            .transport_configured = true,
            .capture_seed = {
                .valid = true,
                .stream_id = 42,
                .request_id = "wake-request",
            },
        },
    };
    reduce_event(&context, &wake, &effects);
    CHECK(context.state.turn == BOBE_TURN_IDLE);
    CHECK(!has_effect(
        &effects,
        BOBE_POLICY_EFFECT_TRANSPORT_CAPTURE_REQUEST
    ));

    bobe_policy_event_t remote = control_event(
        face_control(BOBE_FACE_HAPPY, 4),
        UINT64_C(2000)
    );
    reduce_event(&context, &remote, &effects);
    CHECK(!has_effect(&effects, BOBE_POLICY_EFFECT_DISPLAY_FACE));

    reduce_event(&context, &hold, &effects);
    CHECK(!context.state.microphone_muted);
    effect = find_effect(&effects, BOBE_POLICY_EFFECT_AUDIO_SET_MUTE);
    CHECK(effect != NULL);
    CHECK(!effect->data.mute.muted);
    CHECK(effect->data.mute.persist);
    CHECK(has_effect(&effects, BOBE_POLICY_EFFECT_WAKE_MONITOR_START));
    effect = find_effect(&effects, BOBE_POLICY_EFFECT_DISPLAY_FACE);
    CHECK(effect != NULL);
    CHECK(effect->data.face.expression == BOBE_FACE_IDLE);

    press = action_button(BOBE_BUTTON_PRESSED, 13, 0);
    reduce_event(&context, &press, &effects);
    CHECK(context.state.turn == BOBE_TURN_REQUESTING_CAPTURE);
    CHECK(has_effect(
        &effects,
        BOBE_POLICY_EFFECT_TRANSPORT_CAPTURE_REQUEST
    ));

    hold = volume_down_long_press(14);
    reduce_event(&context, &hold, &effects);
    CHECK(context.state.microphone_muted);
    CHECK(context.state.turn == BOBE_TURN_IDLE);
    CHECK(context.pending_operation == BOBE_POLICY_OPERATION_NONE);
    CHECK(has_effect(&effects, BOBE_POLICY_EFFECT_AUDIO_CAPTURE_STOP));
    CHECK(has_effect(&effects, BOBE_POLICY_EFFECT_VOICE_STREAM_END));
    CHECK(has_effect(&effects, BOBE_POLICY_EFFECT_TRANSPORT_CAPTURE_CANCEL));
    effect = find_effect(&effects, BOBE_POLICY_EFFECT_DISPLAY_FACE);
    CHECK(effect != NULL);
    CHECK(effect->data.face.expression == BOBE_FACE_MUTED);

    init_online(&context);
    hold = volume_down_long_press(15);
    hold.data.button.event.button = BOBE_BUTTON_ACTION;
    hold.data.button.physical_action_pressed = true;
    reduce_event(&context, &hold, &effects);
    CHECK(!context.state.microphone_muted);
    CHECK(context.state.lifecycle == BOBE_LIFECYCLE_SHUTTING_DOWN);
    CHECK(has_effect(&effects, BOBE_POLICY_EFFECT_BOARD_SHUTDOWN));
}

static void test_persisted_microphone_mute_boot(void)
{
    bobe_policy_context_t context;
    bobe_policy_effect_batch_t effects;
    bobe_policy_init(&context);
    const bobe_policy_event_t muted_boot = {
        .kind = BOBE_POLICY_EVENT_BOOT,
        .data.boot.microphone_muted = true,
    };
    reduce_event(&context, &muted_boot, &effects);
    CHECK(context.state.microphone_muted);
    const bobe_policy_effect_t *effect =
        find_effect(&effects, BOBE_POLICY_EFFECT_AUDIO_SET_MUTE);
    CHECK(effect != NULL);
    CHECK(effect->data.mute.muted);
    CHECK(!effect->data.mute.persist);
    effect = find_effect(&effects, BOBE_POLICY_EFFECT_DISPLAY_FACE);
    CHECK(effect != NULL);
    CHECK(effect->data.face.expression == BOBE_FACE_BOOTING);

    const bobe_policy_event_t voice_ready = {
        .kind = BOBE_POLICY_EVENT_VOICE_READY,
    };
    reduce_event(&context, &voice_ready, &effects);
    CHECK(!has_effect(&effects, BOBE_POLICY_EFFECT_WAKE_MONITOR_START));

    bobe_policy_init(&context);
    const bobe_policy_event_t unmuted_boot = {
        .kind = BOBE_POLICY_EVENT_BOOT,
    };
    reduce_event(&context, &unmuted_boot, &effects);
    CHECK(!context.state.microphone_muted);
    CHECK(!has_effect(&effects, BOBE_POLICY_EFFECT_AUDIO_SET_MUTE));
}

static void test_ota_confirmation_deadline(void)
{
    bobe_policy_context_t context;
    bobe_policy_effect_batch_t effects;
    bobe_policy_init(&context);
    const bobe_policy_event_t boot = {
        .kind = BOBE_POLICY_EVENT_BOOT,
        .now_us = UINT64_C(1000),
        .data.boot.ota_pending_verification = true,
    };
    reduce_event(&context, &boot, &effects);
    CHECK(
        context.ota_confirm_deadline_us ==
        UINT64_C(1000) + BOBE_POLICY_OTA_CONFIRM_TIMEOUT_US
    );

    bobe_policy_event_t tick = {
        .kind = BOBE_POLICY_EVENT_TIME_TICK,
        .now_us = context.ota_confirm_deadline_us - 1U,
    };
    reduce_event(&context, &tick, &effects);
    CHECK(!has_effect(&effects, BOBE_POLICY_EFFECT_OTA_ROLLBACK));

    tick.now_us = context.ota_confirm_deadline_us;
    reduce_event(&context, &tick, &effects);
    CHECK(has_effect(&effects, BOBE_POLICY_EFFECT_OTA_ROLLBACK));
    CHECK(context.ota_confirm_deadline_us == 0);

    bobe_policy_init(&context);
    reduce_event(&context, &boot, &effects);
    bobe_policy_event_t ready = {
        .kind = BOBE_POLICY_EVENT_TRANSPORT,
        .now_us = UINT64_C(2000),
        .data.transport = {
            .kind = BOBE_POLICY_TRANSPORT_SESSION_READY,
            .generation = 7,
            .current_generation = 7,
            .ota_pending_verification = true,
        },
    };
    context.state.lifecycle = BOBE_LIFECYCLE_CONNECTING;
    reduce_event(&context, &ready, &effects);
    CHECK(has_effect(&effects, BOBE_POLICY_EFFECT_OTA_CONFIRM));
    CHECK(
        context.pending_operation == BOBE_POLICY_OPERATION_OTA_CONFIRM
    );
    check_result_effect_is_last(&effects);
    CHECK(context.ota_confirm_deadline_us != 0);

    bobe_policy_event_t hold = volume_down_long_press(30);
    reduce_event(&context, &hold, &effects);
    CHECK(context.pending_operation == BOBE_POLICY_OPERATION_OTA_CONFIRM);
    CHECK(!context.state.microphone_muted);

    complete_operation(
        &context,
        BOBE_POLICY_RESULT_FAILED,
        2100,
        0,
        &effects
    );
    CHECK(context.ota_confirm_deadline_us != 0);

    ready.now_us = 2200;
    reduce_event(&context, &ready, &effects);
    complete_operation(
        &context,
        BOBE_POLICY_RESULT_OK,
        2300,
        0,
        &effects
    );
    CHECK(context.ota_confirm_deadline_us == 0);

    tick.now_us = UINT64_MAX;
    reduce_event(&context, &tick, &effects);
    CHECK(!has_effect(&effects, BOBE_POLICY_EFFECT_OTA_ROLLBACK));
}

static bobe_control_message_t update_offer(void)
{
    bobe_control_message_t control = {
        .kind = BOBE_CONTROL_UPDATE_OFFER,
        .data.update = {
            .build_number = 2,
            .image_size = 1024,
            .url = "https://ota.example.com/bobe.bin",
        },
    };
    return control;
}

static void test_update_policy(void)
{
    bobe_policy_context_t context;
    bobe_policy_effect_batch_t effects;
    init_online(&context);

    context.state.turn = BOBE_TURN_LISTENING;
    bobe_policy_event_t offer = control_event(update_offer(), 1000);
    reduce_event(&context, &offer, &effects);
    const bobe_policy_effect_t *effect = find_effect(
        &effects,
        BOBE_POLICY_EFFECT_TRANSPORT_UPDATE_STATE
    );
    CHECK(effect != NULL);
    CHECK(strcmp(effect->data.update_state.phase, "failed") == 0);
    CHECK(strcmp(effect->data.update_state.reason, "busy") == 0);
    CHECK(!has_effect(&effects, BOBE_POLICY_EFFECT_UPDATE_INSTALL));

    init_online(&context);
    reduce_event(&context, &offer, &effects);
    effect = find_effect(
        &effects,
        BOBE_POLICY_EFFECT_TRANSPORT_UPDATE_STATE
    );
    CHECK(effect != NULL);
    CHECK(strcmp(effect->data.update_state.reason, "battery_unknown") == 0);
    CHECK(!has_effect(&effects, BOBE_POLICY_EFFECT_UPDATE_INSTALL));

    init_online(&context);
    context.battery_valid = true;
    context.battery.percent = 49;
    context.battery.charging = false;
    reduce_event(&context, &offer, &effects);
    effect = find_effect(
        &effects,
        BOBE_POLICY_EFFECT_TRANSPORT_UPDATE_STATE
    );
    CHECK(effect != NULL);
    CHECK(strcmp(effect->data.update_state.reason, "low_battery") == 0);
    CHECK(!has_effect(&effects, BOBE_POLICY_EFFECT_UPDATE_INSTALL));

    context.battery.charging = true;
    reduce_event(&context, &offer, &effects);
    CHECK(context.state.lifecycle == BOBE_LIFECYCLE_UPDATING);
    CHECK(context.pending_operation == BOBE_POLICY_OPERATION_UPDATE_INSTALL);
    effect = find_effect(&effects, BOBE_POLICY_EFFECT_UPDATE_INSTALL);
    CHECK(effect != NULL);
    CHECK(effect->data.update.build_number == 2);
    CHECK(strcmp(effect->data.update.url, "https://ota.example.com/bobe.bin") == 0);
    CHECK(!has_effect(&effects, BOBE_POLICY_EFFECT_WAKE_MONITOR_START));
    check_result_effect_is_last(&effects);

    bobe_policy_event_t hold = volume_down_long_press(31);
    reduce_event(&context, &hold, &effects);
    CHECK(context.pending_operation == BOBE_POLICY_OPERATION_UPDATE_INSTALL);
    CHECK(context.state.lifecycle == BOBE_LIFECYCLE_UPDATING);
    CHECK(!context.state.microphone_muted);

    complete_operation(
        &context,
        BOBE_POLICY_RESULT_FAILED,
        2000,
        0,
        &effects
    );
    CHECK(context.state.lifecycle == BOBE_LIFECYCLE_ONLINE);
    CHECK(has_effect(&effects, BOBE_POLICY_EFFECT_WAKE_MONITOR_START));
    effect = find_effect(
        &effects,
        BOBE_POLICY_EFFECT_TRANSPORT_UPDATE_STATE
    );
    CHECK(effect != NULL);
    CHECK(strcmp(effect->data.update_state.phase, "failed") == 0);

    context.battery.charging = true;
    reduce_event(&context, &offer, &effects);
    complete_operation(
        &context,
        BOBE_POLICY_RESULT_OK,
        3000,
        0,
        &effects
    );
    CHECK(context.state.lifecycle == BOBE_LIFECYCLE_REBOOTING);
    CHECK(has_effect(&effects, BOBE_POLICY_EFFECT_UPDATE_REBOOT));
    effect = find_effect(
        &effects,
        BOBE_POLICY_EFFECT_TRANSPORT_UPDATE_STATE
    );
    CHECK(effect != NULL);
    CHECK(strcmp(effect->data.update_state.phase, "rebooting") == 0);
    CHECK(effect->data.update_state.progress_percent == 100);
}

static void test_claim_revocation(void)
{
    bobe_policy_context_t context;
    bobe_policy_effect_batch_t effects;
    init_online(&context);
    const bobe_policy_event_t revoke = control_event(
        (bobe_control_message_t){.kind = BOBE_CONTROL_CLAIM_REVOKE},
        1000
    );
    reduce_event(&context, &revoke, &effects);
    CHECK(context.pending_operation == BOBE_POLICY_OPERATION_CLAIM_REVOKE);
    CHECK(has_effect(&effects, BOBE_POLICY_EFFECT_CLAIM_REVOKE));
    CHECK(has_effect(&effects, BOBE_POLICY_EFFECT_WAKE_MONITOR_STOP));
    check_result_effect_is_last(&effects);

    complete_operation(
        &context,
        BOBE_POLICY_RESULT_OK,
        1100,
        0,
        &effects
    );
    CHECK(context.state.lifecycle == BOBE_LIFECYCLE_UNPROVISIONED);
    CHECK(
        context.pending_operation ==
        BOBE_POLICY_OPERATION_PROVISIONING_OPEN
    );
    CHECK(has_effect(
        &effects,
        BOBE_POLICY_EFFECT_CONNECTIVITY_OPEN_PROVISIONING
    ));
}

static void test_boot_restores_user_settings(void)
{
    bobe_policy_context_t context;
    bobe_policy_effect_batch_t effects;
    bobe_policy_init(&context);
    const bobe_policy_event_t boot = {
        .kind = BOBE_POLICY_EVENT_BOOT,
        .data.boot = {
            .volume = 35,
            .theme = BOBE_FACE_THEME_SOLAR_CROWN,
        },
    };
    reduce_event(&context, &boot, &effects);
    const bobe_policy_effect_t *effect = find_effect(
        &effects,
        BOBE_POLICY_EFFECT_AUDIO_SET_VOLUME
    );
    CHECK(effect != NULL && effect->data.volume == 35);
    effect = find_effect(&effects, BOBE_POLICY_EFFECT_DISPLAY_THEME);
    CHECK(effect != NULL);
    CHECK(effect->data.theme.theme == BOBE_FACE_THEME_SOLAR_CROWN);
    CHECK(effect->data.theme.state_version == 0);
    CHECK(context.theme == BOBE_FACE_THEME_SOLAR_CROWN);
    CHECK(context.theme_version == 0);

    context.state.lifecycle = BOBE_LIFECYCLE_ONLINE;
    context.transport_generation = 7;
    bobe_policy_event_t theme = control_event(
        theme_control(BOBE_FACE_THEME_WARM_APERTURE, 1),
        1000
    );
    reduce_event(&context, &theme, &effects);
    effect = find_effect(&effects, BOBE_POLICY_EFFECT_DISPLAY_THEME);
    CHECK(effect != NULL);
    CHECK(effect->data.theme.theme == BOBE_FACE_THEME_WARM_APERTURE);

    bobe_policy_init(&context);
    const bobe_policy_event_t defaults = {
        .kind = BOBE_POLICY_EVENT_BOOT,
        .data.boot.volume = 70,
    };
    reduce_event(&context, &defaults, &effects);
    CHECK(!has_effect(&effects, BOBE_POLICY_EFFECT_DISPLAY_THEME));
}

static void test_theme_version_gating(void)
{
    bobe_policy_context_t context;
    bobe_policy_effect_batch_t effects;
    init_online(&context);
    CHECK(context.theme == BOBE_FACE_THEME_DEFAULT);
    CHECK(context.theme_version == 0);

    bobe_policy_event_t event =
        control_event(theme_control(BOBE_FACE_THEME_SOLAR_CROWN, 4), 1000);
    reduce_event(&context, &event, &effects);
    const bobe_policy_effect_t *effect =
        find_effect(&effects, BOBE_POLICY_EFFECT_DISPLAY_THEME);
    CHECK(effect != NULL);
    CHECK(effect->data.theme.theme == BOBE_FACE_THEME_SOLAR_CROWN);
    CHECK(effect->data.theme.state_version == 4);
    CHECK(context.theme == BOBE_FACE_THEME_SOLAR_CROWN);
    CHECK(context.theme_version == 4);

    event = control_event(theme_control(BOBE_FACE_THEME_SOLAR_CROWN, 4), 2000);
    reduce_event(&context, &event, &effects);
    CHECK(!has_effect(&effects, BOBE_POLICY_EFFECT_DISPLAY_THEME));
    CHECK(context.theme == BOBE_FACE_THEME_SOLAR_CROWN);
    CHECK(context.theme_version == 4);

    event = control_event(theme_control(BOBE_FACE_THEME_WARM_LEDGE, 4), 3000);
    reduce_event(&context, &event, &effects);
    CHECK(!has_effect(&effects, BOBE_POLICY_EFFECT_DISPLAY_THEME));
    CHECK(context.theme == BOBE_FACE_THEME_SOLAR_CROWN);

    event = control_event(theme_control(BOBE_FACE_THEME_WARM_LEDGE, 3), 4000);
    reduce_event(&context, &event, &effects);
    CHECK(!has_effect(&effects, BOBE_POLICY_EFFECT_DISPLAY_THEME));
    CHECK(context.theme == BOBE_FACE_THEME_SOLAR_CROWN);
    CHECK(context.theme_version == 4);

    event = control_event(
        theme_control(BOBE_FACE_THEME_KETTLE_TALL_BRIGHT, 5),
        5000
    );
    reduce_event(&context, &event, &effects);
    effect = find_effect(&effects, BOBE_POLICY_EFFECT_DISPLAY_THEME);
    CHECK(effect != NULL);
    CHECK(effect->data.theme.theme == BOBE_FACE_THEME_KETTLE_TALL_BRIGHT);
    CHECK(context.theme == BOBE_FACE_THEME_KETTLE_TALL_BRIGHT);
    CHECK(context.theme_version == 5);

    bobe_policy_event_t ready = {
        .kind = BOBE_POLICY_EVENT_TRANSPORT,
        .now_us = 6000,
        .data.transport = {
            .kind = BOBE_POLICY_TRANSPORT_SESSION_READY,
            .generation = 7,
            .current_generation = 7,
        },
    };
    reduce_event(&context, &ready, &effects);
    CHECK(context.theme_version == 0);
    CHECK(context.theme == BOBE_FACE_THEME_KETTLE_TALL_BRIGHT);

    event = control_event(theme_control(BOBE_FACE_THEME_WARM_APERTURE, 1), 7000);
    reduce_event(&context, &event, &effects);
    effect = find_effect(&effects, BOBE_POLICY_EFFECT_DISPLAY_THEME);
    CHECK(effect != NULL);
    CHECK(effect->data.theme.theme == BOBE_FACE_THEME_WARM_APERTURE);
    CHECK(context.theme == BOBE_FACE_THEME_WARM_APERTURE);
    CHECK(context.theme_version == 1);
}

static bobe_policy_event_t volume_button(
    bobe_button_id_t button,
    uint8_t current_volume
)
{
    bobe_policy_event_t event = {
        .kind = BOBE_POLICY_EVENT_BUTTON,
        .now_us = UINT64_C(1000),
        .data.button = {
            .event = {
                .button = button,
                .action = BOBE_BUTTON_PRESSED,
                .event_id = 5,
            },
            .transport_connected = true,
            .transport_configured = true,
            .current_volume = current_volume,
        },
    };
    return event;
}

static bobe_policy_event_t notice_tick(uint64_t now_us)
{
    bobe_policy_event_t event = {
        .kind = BOBE_POLICY_EVENT_TIME_TICK,
        .now_us = now_us,
        .data.tick = {
            .network_online = true,
        },
    };
    return event;
}

static void test_local_notices(void)
{
    bobe_policy_context_t context;
    bobe_policy_effect_batch_t effects;

    init_online(&context);
    bobe_policy_event_t event = volume_button(BOBE_BUTTON_VOLUME_UP, 50);
    reduce_event(&context, &event, &effects);
    const bobe_policy_effect_t *volume =
        find_effect(&effects, BOBE_POLICY_EFFECT_AUDIO_SET_VOLUME);
    const bobe_policy_effect_t *notice =
        find_effect(&effects, BOBE_POLICY_EFFECT_DISPLAY_NOTICE);
    CHECK(volume != NULL && volume->data.volume == 55);
    CHECK(notice != NULL);
    CHECK(notice->data.notice.kind == BOBE_NOTICE_VOLUME);
    CHECK(strcmp(notice->data.notice.text, "Volume 55%") == 0);

    init_online(&context);
    event = volume_button(BOBE_BUTTON_VOLUME_DOWN, 5);
    reduce_event(&context, &event, &effects);
    notice = find_effect(&effects, BOBE_POLICY_EFFECT_DISPLAY_NOTICE);
    CHECK(notice != NULL);
    CHECK(notice->data.notice.kind == BOBE_NOTICE_SILENT);
    CHECK(!context.state.microphone_muted);

    init_online(&context);
    event = volume_button(BOBE_BUTTON_VOLUME_UP, 100);
    reduce_event(&context, &event, &effects);
    CHECK(!has_effect(&effects, BOBE_POLICY_EFFECT_DISPLAY_NOTICE));

    init_online(&context);
    bobe_policy_event_t hold = volume_down_long_press(11);
    reduce_event(&context, &hold, &effects);
    CHECK(context.state.microphone_muted);
    CHECK(!has_effect(&effects, BOBE_POLICY_EFFECT_DISPLAY_NOTICE));
    CHECK(context.pending_notice.kind == BOBE_NOTICE_HINT);
    event = notice_tick(UINT64_C(9000));
    reduce_event(&context, &event, &effects);
    notice = find_effect(&effects, BOBE_POLICY_EFFECT_DISPLAY_NOTICE);
    CHECK(notice != NULL);
    CHECK(strcmp(notice->data.notice.text, "Hold -: mic") == 0);

    bobe_policy_init(&context);
    const bobe_policy_event_t boot = {.kind = BOBE_POLICY_EVENT_BOOT};
    reduce_event(&context, &boot, &effects);
    CHECK(!has_effect(&effects, BOBE_POLICY_EFFECT_DISPLAY_NOTICE));
    const bobe_policy_event_t unprovisioned = {
        .kind = BOBE_POLICY_EVENT_NETWORK,
        .now_us = UINT64_C(1500),
        .data.network = {
            .state = BOBE_NETWORK_UNPROVISIONED,
            .generation = 1,
            .current_generation = 1,
        },
    };
    reduce_event(&context, &unprovisioned, &effects);
    CHECK(context.state.lifecycle == BOBE_LIFECYCLE_UNPROVISIONED);
    CHECK(!has_effect(&effects, BOBE_POLICY_EFFECT_DISPLAY_NOTICE));
    CHECK(context.pending_notice.kind == BOBE_NOTICE_HINT);
    event = notice_tick(UINT64_C(2000));
    reduce_event(&context, &event, &effects);
    notice = find_effect(&effects, BOBE_POLICY_EFFECT_DISPLAY_NOTICE);
    CHECK(notice != NULL);
    CHECK(strcmp(notice->data.notice.text, "Hold +/-: setup") == 0);
    CHECK(context.pending_notice.kind == BOBE_NOTICE_NONE);
    event = notice_tick(UINT64_C(3000));
    reduce_event(&context, &event, &effects);
    CHECK(!has_effect(&effects, BOBE_POLICY_EFFECT_DISPLAY_NOTICE));

    bobe_policy_init(&context);
    context.state.lifecycle = BOBE_LIFECYCLE_CONNECTING;
    context.network_online = true;
    context.transport_configured = true;
    bobe_policy_event_t ready = {
        .kind = BOBE_POLICY_EVENT_TRANSPORT,
        .now_us = UINT64_C(4000),
        .data.transport = {
            .kind = BOBE_POLICY_TRANSPORT_SESSION_READY,
            .generation = 7,
            .current_generation = 7,
            .ota_pending_verification = true,
        },
    };
    reduce_event(&context, &ready, &effects);
    CHECK(has_effect(&effects, BOBE_POLICY_EFFECT_OTA_CONFIRM));
    CHECK(!has_effect(&effects, BOBE_POLICY_EFFECT_DISPLAY_NOTICE));
    CHECK(context.pending_notice.kind == BOBE_NOTICE_UPDATE);
    complete_operation(
        &context,
        BOBE_POLICY_RESULT_OK,
        UINT64_C(4500),
        0,
        &effects
    );
    event = notice_tick(UINT64_C(5000));
    reduce_event(&context, &event, &effects);
    notice = find_effect(&effects, BOBE_POLICY_EFFECT_DISPLAY_NOTICE);
    CHECK(notice != NULL);
    CHECK(notice->data.notice.kind == BOBE_NOTICE_UPDATE);

    CHECK(context.remote_face_version == 0);
}

int main(void)
{
    test_capture_grant_deny_release();
    test_playback_open_ack_close();
    test_timeouts_and_stale_generations();
    test_button_release_fencing();
    test_retry_jitter_boundaries();
    test_destructive_chord_and_critical_order();
    test_effect_failure_refreshes_transport_state();
    test_wake_vad_ordering();
    test_network_and_provisioning_cancellation();
    test_queue_and_effect_bounds();
    test_microphone_mute_toggle();
    test_persisted_microphone_mute_boot();
    test_ota_confirmation_deadline();
    test_update_policy();
    test_claim_revocation();
    test_boot_restores_user_settings();
    test_theme_version_gating();
    test_local_notices();
    puts("BoBe pure core policy tests passed.");
    return 0;
}
