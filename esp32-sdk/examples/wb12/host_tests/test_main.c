#include "bobe_battery.h"
#include "bobe_protocol.h"
#include "bobe_state.h"

#include <stdint.h>
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

static bobe_protocol_result_t parse(
    const char *json,
    bobe_control_message_t *message
)
{
    return bobe_protocol_parse_control(json, strlen(json), message);
}

static void test_media_framing(void)
{
    const bobe_media_header_t expected = {
        .protocol_major = BOBE_PROTOCOL_MAJOR,
        .kind = BOBE_MEDIA_MICROPHONE_PCM,
        .flags = BOBE_MEDIA_FLAG_DISCONTINUITY,
        .stream_id = UINT32_C(0x12345678),
        .sequence = UINT32_C(0x90ABCDEF),
        .monotonic_timestamp_us = UINT64_C(0x0123456789ABCDEF),
    };
    const uint8_t golden[BOBE_MEDIA_HEADER_BYTES] = {
        0x01, 0x01, 0x00, 0x01,
        0x12, 0x34, 0x56, 0x78,
        0x90, 0xAB, 0xCD, 0xEF,
        0x01, 0x23, 0x45, 0x67,
        0x89, 0xAB, 0xCD, 0xEF,
    };
    uint8_t encoded[BOBE_MEDIA_HEADER_BYTES];
    bobe_media_header_encode(&expected, encoded);
    CHECK(memcmp(encoded, golden, sizeof(golden)) == 0);

    bobe_media_header_t decoded;
    CHECK(
        bobe_media_header_decode(encoded, sizeof(encoded), &decoded) ==
        BOBE_PROTOCOL_OK
    );
    CHECK(decoded.protocol_major == expected.protocol_major);
    CHECK(decoded.kind == expected.kind);
    CHECK(decoded.flags == expected.flags);
    CHECK(decoded.stream_id == expected.stream_id);
    CHECK(decoded.sequence == expected.sequence);
    CHECK(
        decoded.monotonic_timestamp_us ==
        expected.monotonic_timestamp_us
    );

    CHECK(
        bobe_media_header_decode(NULL, sizeof(encoded), &decoded) ==
        BOBE_PROTOCOL_INVALID_ARGUMENT
    );
    CHECK(
        bobe_media_header_decode(encoded, sizeof(encoded) - 1, &decoded) ==
        BOBE_PROTOCOL_MALFORMED
    );
    encoded[0] = 2;
    CHECK(
        bobe_media_header_decode(encoded, sizeof(encoded), &decoded) ==
        BOBE_PROTOCOL_UNSUPPORTED
    );
    memcpy(encoded, golden, sizeof(encoded));
    encoded[1] = 0x03;
    CHECK(
        bobe_media_header_decode(encoded, sizeof(encoded), &decoded) ==
        BOBE_PROTOCOL_UNSUPPORTED
    );
    memcpy(encoded, golden, sizeof(encoded));
    encoded[3] = 0x02;
    CHECK(
        bobe_media_header_decode(encoded, sizeof(encoded), &decoded) ==
        BOBE_PROTOCOL_MALFORMED
    );
    memcpy(encoded, golden, sizeof(encoded));
    encoded[2] = 0x80;
    CHECK(
        bobe_media_header_decode(encoded, sizeof(encoded), &decoded) ==
        BOBE_PROTOCOL_MALFORMED
    );
    memcpy(encoded, golden, sizeof(encoded));
    memset(&encoded[4], 0, 4);
    CHECK(
        bobe_media_header_decode(encoded, sizeof(encoded), &decoded) ==
        BOBE_PROTOCOL_MALFORMED
    );
}

static void test_media_payloads(void)
{
    bobe_media_header_t header = {
        .kind = BOBE_MEDIA_MICROPHONE_PCM,
    };
    CHECK(
        bobe_media_validate_payload(
            &header,
            BOBE_MIC_FRAME_SAMPLES * sizeof(int16_t)
        ) == BOBE_PROTOCOL_OK
    );
    CHECK(
        bobe_media_validate_payload(
            &header,
            BOBE_MIC_FRAME_SAMPLES * sizeof(int16_t) - 1
        ) == BOBE_PROTOCOL_RANGE_ERROR
    );
    header.kind = BOBE_MEDIA_SPEAKER_PCM;
    CHECK(
        bobe_media_validate_payload(
            &header,
            BOBE_SPEAKER_FRAME_SAMPLES * sizeof(int16_t)
        ) == BOBE_PROTOCOL_OK
    );
    header.kind = (bobe_media_kind_t)99;
    CHECK(
        bobe_media_validate_payload(&header, 1) ==
        BOBE_PROTOCOL_UNSUPPORTED
    );
    CHECK(
        bobe_media_validate_payload(NULL, 0) ==
        BOBE_PROTOCOL_INVALID_ARGUMENT
    );
}

static void test_control_parser(void)
{
    bobe_control_message_t message;
    const char welcome[] =
        "{\"type\":\"welcome\",\"protocol_major\":1,\"protocol_minor\":1,"
        "\"session_id\":\"session-1\",\"heartbeat_interval_ms\":5000,"
        "\"controller_epoch\":42}";
    CHECK(parse(welcome, &message) == BOBE_PROTOCOL_OK);
    CHECK(message.kind == BOBE_CONTROL_WELCOME);
    CHECK(strcmp(message.data.welcome.session_id, "session-1") == 0);
    CHECK(message.data.welcome.heartbeat_interval_ms == 5000);
    CHECK(message.data.welcome.controller_epoch == 42);

    const char welcome_whitespace[] =
        "{\"type\":\"welcome\",\"protocol_major\":1,\"protocol_minor\":1,"
        "\"session_id\":\"s\",\"heartbeat_interval_ms\":1000,"
        "\"controller_epoch\":1} \r\n\t";
    CHECK(parse(welcome_whitespace, &message) == BOBE_PROTOCOL_OK);

    const char duplicate[] =
        "{\"type\":\"error\",\"type\":\"sensor.ack\","
        "\"protocol_major\":1,\"code\":\"x\",\"message\":\"y\"}";
    CHECK(parse(duplicate, &message) == BOBE_PROTOCOL_MALFORMED);
    const char trailing[] =
        "{\"type\":\"error\",\"protocol_major\":1,"
        "\"code\":\"x\",\"message\":\"y\"}{}";
    CHECK(parse(trailing, &message) == BOBE_PROTOCOL_MALFORMED);
    const char nul_escape[] =
        "{\"type\":\"welcome\",\"protocol_major\":1,\"protocol_minor\":1,"
        "\"session_id\":\"x\\u0000y\",\"heartbeat_interval_ms\":1000,"
        "\"controller_epoch\":1}";
    CHECK(parse(nul_escape, &message) == BOBE_PROTOCOL_MALFORMED);
    const char *const invalid_escapes[] = {"x\\uZZZZy", "x\\u12", "x\\qy"};
    for (size_t index = 0;
         index < sizeof(invalid_escapes) / sizeof(invalid_escapes[0]);
         ++index) {
        char invalid_escape[256];
        snprintf(
            invalid_escape,
            sizeof(invalid_escape),
            "{\"type\":\"welcome\",\"protocol_major\":1,\"protocol_minor\":1,"
            "\"session_id\":\"%s\",\"heartbeat_interval_ms\":1000,"
            "\"controller_epoch\":1}",
            invalid_escapes[index]
        );
        CHECK(parse(invalid_escape, &message) == BOBE_PROTOCOL_MALFORMED);
    }
    const char valid_escapes[] =
        "{\"type\":\"error\",\"protocol_major\":1,"
        "\"code\":\"x\",\"message\":\"\\\"\\\\\\/\\b\\f\\n\\r\\t\\u00e9\"}";
    CHECK(parse(valid_escapes, &message) == BOBE_PROTOCOL_OK);
    const char raw_nul[] =
        "{\"type\":\"error\",\"protocol_major\":1,"
        "\"code\":\"x\",\"message\":\"y\"}\0junk";
    CHECK(
        bobe_protocol_parse_control(
            raw_nul,
            sizeof(raw_nul) - 1,
            &message
        ) == BOBE_PROTOCOL_MALFORMED
    );
    CHECK(
        bobe_protocol_parse_control(NULL, 0, &message) ==
        BOBE_PROTOCOL_INVALID_ARGUMENT
    );
    CHECK(
        bobe_protocol_parse_control("{}", 2, NULL) ==
        BOBE_PROTOCOL_INVALID_ARGUMENT
    );

    const char wrong_major[] =
        "{\"type\":\"error\",\"protocol_major\":2,"
        "\"code\":\"x\",\"message\":\"y\"}";
    CHECK(
        parse(wrong_major, &message) ==
        BOBE_PROTOCOL_VERSION_MISMATCH
    );
    const char unknown[] =
        "{\"type\":\"future.message\",\"protocol_major\":1}";
    CHECK(parse(unknown, &message) == BOBE_PROTOCOL_UNSUPPORTED);
    CHECK(message.kind == BOBE_CONTROL_UNKNOWN);
    const char fractional[] =
        "{\"type\":\"theme.set\",\"protocol_major\":1,"
        "\"theme_id\":\"solar-crown\",\"state_version\":1.5}";
    CHECK(parse(fractional, &message) == BOBE_PROTOCOL_MALFORMED);
    const char imprecise[] =
        "{\"type\":\"theme.set\",\"protocol_major\":1,"
        "\"theme_id\":\"solar-crown\",\"state_version\":9007199254740992}";
    CHECK(parse(imprecise, &message) == BOBE_PROTOCOL_MALFORMED);
    const char maximum_exact[] =
        "{\"type\":\"theme.set\",\"protocol_major\":1,"
        "\"theme_id\":\"solar-crown\",\"state_version\":9007199254740991}";
    CHECK(parse(maximum_exact, &message) == BOBE_PROTOCOL_OK);
    CHECK(message.data.theme.state_version == UINT64_C(9007199254740991));

    const char short_heartbeat[] =
        "{\"type\":\"welcome\",\"protocol_major\":1,\"protocol_minor\":1,"
        "\"session_id\":\"s\",\"heartbeat_interval_ms\":999,"
        "\"controller_epoch\":1}";
    CHECK(parse(short_heartbeat, &message) == BOBE_PROTOCOL_RANGE_ERROR);
    const char zero_epoch[] =
        "{\"type\":\"welcome\",\"protocol_major\":1,\"protocol_minor\":1,"
        "\"session_id\":\"s\",\"heartbeat_interval_ms\":1000,"
        "\"controller_epoch\":0}";
    CHECK(parse(zero_epoch, &message) == BOBE_PROTOCOL_RANGE_ERROR);
    const char empty_session[] =
        "{\"type\":\"welcome\",\"protocol_major\":1,\"protocol_minor\":1,"
        "\"session_id\":\"\",\"heartbeat_interval_ms\":1000,"
        "\"controller_epoch\":1}";
    CHECK(parse(empty_session, &message) == BOBE_PROTOCOL_MALFORMED);

    const char granted[] =
        "{\"type\":\"capture.granted\",\"protocol_major\":1,"
        "\"stream_id\":7,\"lease_ttl_ms\":30000,"
        "\"request_id\":\"request\",\"lease_id\":\"lease\","
        "\"turn_id\":\"turn\"}";
    CHECK(parse(granted, &message) == BOBE_PROTOCOL_OK);
    CHECK(message.kind == BOBE_CONTROL_CAPTURE_GRANTED);
    CHECK(message.data.capture.stream_id == 7);
    const char bad_grant[] =
        "{\"type\":\"capture.granted\",\"protocol_major\":1,"
        "\"stream_id\":0,\"lease_ttl_ms\":999,\"request_id\":\"r\","
        "\"lease_id\":\"l\",\"turn_id\":\"t\"}";
    CHECK(parse(bad_grant, &message) == BOBE_PROTOCOL_RANGE_ERROR);

    const char face[] =
        "{\"type\":\"face.set\",\"protocol_major\":1,"
        "\"expression\":\"happy\",\"state_version\":7,"
        "\"lease_id\":\"lease\",\"turn_id\":\"turn\","
        "\"caption\":\"hello\",\"accent_rgb\":16711935,\"ttl_ms\":600000}";
    CHECK(parse(face, &message) == BOBE_PROTOCOL_OK);
    CHECK(message.kind == BOBE_CONTROL_FACE_SET);
    CHECK(message.data.face.expression == BOBE_FACE_HAPPY);
    CHECK(message.data.face.state_version == 7);
    CHECK(strcmp(message.data.face.lease_id, "lease") == 0);
    CHECK(strcmp(message.data.face.turn_id, "turn") == 0);
    CHECK(strcmp(message.data.face.caption, "hello") == 0);
    CHECK(message.data.face.accent_rgb == UINT32_C(0x00FF00FF));
    const char face_without_accent[] =
        "{\"type\":\"face.set\",\"protocol_major\":1,"
        "\"expression\":\"happy\",\"state_version\":8}";
    CHECK(parse(face_without_accent, &message) == BOBE_PROTOCOL_OK);
    CHECK(message.data.face.accent_rgb == BOBE_FACE_THEME_ACCENT_RGB);
    CHECK(message.data.face.lease_id[0] == '\0');
    const char half_correlated_face[] =
        "{\"type\":\"face.set\",\"protocol_major\":1,"
        "\"expression\":\"happy\",\"state_version\":8,\"lease_id\":\"lease\"}";
    CHECK(parse(half_correlated_face, &message) == BOBE_PROTOCOL_MALFORMED);
    const char invalid_face[] =
        "{\"type\":\"face.set\",\"protocol_major\":1,"
        "\"expression\":\"not-a-face\",\"state_version\":1}";
    CHECK(parse(invalid_face, &message) == BOBE_PROTOCOL_MALFORMED);
    const char invalid_accent[] =
        "{\"type\":\"face.set\",\"protocol_major\":1,"
        "\"expression\":\"idle\",\"state_version\":1,"
        "\"accent_rgb\":16777216}";
    CHECK(parse(invalid_accent, &message) == BOBE_PROTOCOL_RANGE_ERROR);
    const char invalid_ttl[] =
        "{\"type\":\"face.set\",\"protocol_major\":1,"
        "\"expression\":\"idle\",\"state_version\":1,\"ttl_ms\":600001}";
    CHECK(parse(invalid_ttl, &message) == BOBE_PROTOCOL_RANGE_ERROR);

    const char theme[] =
        "{\"type\":\"theme.set\",\"protocol_major\":1,"
        "\"theme_id\":\"kettle-soft-bowl\",\"state_version\":12}";
    CHECK(parse(theme, &message) == BOBE_PROTOCOL_OK);
    CHECK(message.kind == BOBE_CONTROL_THEME_SET);
    CHECK(message.data.theme.theme == BOBE_FACE_THEME_KETTLE_SOFT_BOWL);
    CHECK(message.data.theme.state_version == 12);
    const char theme_unknown[] =
        "{\"type\":\"theme.set\",\"protocol_major\":1,"
        "\"theme_id\":\"warm-window\",\"state_version\":1}";
    CHECK(parse(theme_unknown, &message) == BOBE_PROTOCOL_MALFORMED);
    const char theme_default[] =
        "{\"type\":\"theme.set\",\"protocol_major\":1,"
        "\"theme_id\":\"default\",\"state_version\":1}";
    CHECK(parse(theme_default, &message) == BOBE_PROTOCOL_MALFORMED);
    const char theme_missing_id[] =
        "{\"type\":\"theme.set\",\"protocol_major\":1,"
        "\"state_version\":1}";
    CHECK(parse(theme_missing_id, &message) == BOBE_PROTOCOL_MALFORMED);
    const char theme_long_id[] =
        "{\"type\":\"theme.set\",\"protocol_major\":1,"
        "\"theme_id\":\"solar-crown-solar-crown-solar-crown-solar\","
        "\"state_version\":1}";
    CHECK(parse(theme_long_id, &message) == BOBE_PROTOCOL_MALFORMED);
    const char theme_missing_version[] =
        "{\"type\":\"theme.set\",\"protocol_major\":1,"
        "\"theme_id\":\"solar-crown\"}";
    CHECK(parse(theme_missing_version, &message) == BOBE_PROTOCOL_MALFORMED);
    const char theme_zero_version[] =
        "{\"type\":\"theme.set\",\"protocol_major\":1,"
        "\"theme_id\":\"solar-crown\",\"state_version\":0}";
    CHECK(parse(theme_zero_version, &message) == BOBE_PROTOCOL_RANGE_ERROR);
    const char theme_wrong_major[] =
        "{\"type\":\"theme.set\",\"protocol_major\":2,"
        "\"theme_id\":\"solar-crown\",\"state_version\":1}";
    CHECK(
        parse(theme_wrong_major, &message) == BOBE_PROTOCOL_VERSION_MISMATCH
    );

    const char playback[] =
        "{\"type\":\"audio.playback.open\",\"protocol_major\":1,"
        "\"stream_id\":9,\"codec\":\"pcm_s16le\","
        "\"sample_rate_hz\":24000,\"channels\":1,"
        "\"frame_duration_ms\":20,\"lease_id\":\"lease\","
        "\"turn_id\":\"turn\"}";
    CHECK(parse(playback, &message) == BOBE_PROTOCOL_OK);
    CHECK(message.kind == BOBE_CONTROL_PLAYBACK_OPEN);
    const char wrong_rate[] =
        "{\"type\":\"audio.playback.open\",\"protocol_major\":1,"
        "\"stream_id\":9,\"codec\":\"pcm_s16le\","
        "\"sample_rate_hz\":16000,\"channels\":1,"
        "\"frame_duration_ms\":20,\"lease_id\":\"lease\","
        "\"turn_id\":\"turn\"}";
    CHECK(parse(wrong_rate, &message) == BOBE_PROTOCOL_RANGE_ERROR);

    const char turn[] =
        "{\"type\":\"turn.state\",\"protocol_major\":1,"
        "\"state\":\"thinking\",\"lease_id\":\"lease\","
        "\"turn_id\":\"turn\"}";
    CHECK(parse(turn, &message) == BOBE_PROTOCOL_OK);
    CHECK(message.data.turn.state == BOBE_TURN_THINKING);
}

static void test_control_builders(void)
{
    char json[1024];
    bobe_hello_info_t hello = {
        .device_id = "device",
        .boot_id = "0123456789abcdef0123456789abcdef",
        .firmware_version = "1.2.3",
        .hardware_revision = "wb12",
        .capabilities = {
            .microphone = true, .speaker = true, .battery = true, .wake_word = true,
            .display_width = 280, .display_height = 240, .face_schema = 1,
        },
        .max_playback_credit_ms = 300,
    };
    CHECK(
        bobe_protocol_build_hello(json, sizeof(json), &hello) ==
        BOBE_PROTOCOL_OK
    );
    CHECK(strstr(json, "\"type\":\"hello\"") != NULL);
    CHECK(strstr(json, "\"width\":280") != NULL);
    CHECK(strstr(json, "\"height\":240") != NULL);
    CHECK(strstr(json, "\"wake_word\":true") != NULL);
    CHECK(
        bobe_protocol_build_hello(json, 8, &hello) ==
        BOBE_PROTOCOL_TOO_LARGE
    );
    hello.device_id = NULL;
    CHECK(
        bobe_protocol_build_hello(json, sizeof(json), &hello) ==
        BOBE_PROTOCOL_INVALID_ARGUMENT
    );

    bobe_health_info_t health = {
        .uptime_s = 42,
        .wifi_rssi_dbm = -54,
        .free_internal_heap = 131072,
        .minimum_internal_heap = 104448,
        .free_psram = 7340032,
        .audio_rx_overruns = 1,
        .audio_tx_underruns = 2,
        .lifecycle = BOBE_LIFECYCLE_ONLINE,
        .turn = BOBE_TURN_IDLE,
        .microphone_muted = true,
        .app_version = "0.1.0",
        .build_number = 42,
        .running_slot = 1,
        .ota_pending_verification = true,
        .reset_reason = 3,
        .transport_generation = 7,
        .core_queue_depth = 2,
        .core_queue_capacity = 16,
        .core_stack_free_words = 900,
        .display_stack_free_words = 800,
        .display_flush_submitted = 100,
        .display_flush_errors = 1,
        .display_flush_timeouts = 2,
        .display_flush_last_us = 3000,
        .display_flush_max_us = 4000,
        .playback_sequence_gaps = 5,
        .temperature_millicelsius = 41250,
        .cpu_frequency_mhz = 80,
        .display_frame_rate = 5,
        .backlight_percent = 35,
    };
    CHECK(
        bobe_protocol_build_health(json, sizeof(json), &health) ==
        BOBE_PROTOCOL_OK
    );
    CHECK(strstr(json, "\"microphone_muted\":true") != NULL);
    CHECK(strstr(json, "\"app_version\":\"0.1.0\"") != NULL);
    CHECK(strstr(json, "\"build_number\":42") != NULL);
    CHECK(strstr(json, "\"running_slot\":1") != NULL);
    CHECK(strstr(json, "\"ota_pending_verification\":true") != NULL);
    CHECK(strstr(json, "\"reset_reason\":3") != NULL);
    CHECK(strstr(json, "\"transport_generation\":7") != NULL);
    CHECK(strstr(json, "\"core_queue_depth\":2") != NULL);
    CHECK(strstr(json, "\"core_queue_capacity\":16") != NULL);
    CHECK(strstr(json, "\"core_stack_free_words\":900") != NULL);
    CHECK(strstr(json, "\"display_stack_free_words\":800") != NULL);
    CHECK(strstr(json, "\"display_flush_submitted\":100") != NULL);
    CHECK(strstr(json, "\"display_flush_errors\":1") != NULL);
    CHECK(strstr(json, "\"display_flush_timeouts\":2") != NULL);
    CHECK(strstr(json, "\"display_flush_last_us\":3000") != NULL);
    CHECK(strstr(json, "\"display_flush_max_us\":4000") != NULL);
    CHECK(strstr(json, "\"playback_sequence_gaps\":5") != NULL);
    CHECK(strstr(json, "\"temperature_millicelsius\":41250") != NULL);
    CHECK(strstr(json, "\"cpu_frequency_mhz\":80") != NULL);
    CHECK(strstr(json, "\"display_frame_rate\":5") != NULL);
    CHECK(strstr(json, "\"backlight_percent\":35") != NULL);
    health.microphone_muted = false;
    CHECK(
        bobe_protocol_build_health(json, sizeof(json), &health) ==
        BOBE_PROTOCOL_OK
    );
    CHECK(strstr(json, "\"microphone_muted\":false") != NULL);
    CHECK(
        bobe_protocol_build_health(json, 8, &health) ==
        BOBE_PROTOCOL_TOO_LARGE
    );
    CHECK(
        bobe_protocol_build_health(json, sizeof(json), NULL) ==
        BOBE_PROTOCOL_INVALID_ARGUMENT
    );

    const bobe_button_event_t button = {
        .button = BOBE_BUTTON_VOLUME_UP,
        .action = BOBE_BUTTON_LONG_PRESS,
        .event_id = 7,
        .monotonic_us = 9,
    };
    CHECK(
        bobe_protocol_build_button_event(
            json,
            sizeof(json),
            "boot",
            &button
        ) == BOBE_PROTOCOL_OK
    );
    CHECK(strstr(json, "\"button\":\"volume_up\"") != NULL);
    bobe_button_event_t bad_button = button;
    bad_button.button = (bobe_button_id_t)99;
    CHECK(
        bobe_protocol_build_button_event(
            json,
            sizeof(json),
            "boot",
            &bad_button
        ) == BOBE_PROTOCOL_INVALID_ARGUMENT
    );

    const bobe_battery_state_t battery = {
        .raw_adc = 2200,
        .percent = 50,
        .charging = true,
    };
    CHECK(
        bobe_protocol_build_battery_state(
            json,
            sizeof(json),
            &battery
        ) == BOBE_PROTOCOL_OK
    );
    bobe_battery_state_t invalid_battery = battery;
    invalid_battery.percent = 101;
    CHECK(
        bobe_protocol_build_battery_state(
            json,
            sizeof(json),
            &invalid_battery
        ) == BOBE_PROTOCOL_INVALID_ARGUMENT
    );

    CHECK(
        bobe_protocol_build_update_state(
            json,
            sizeof(json),
            "failed",
            0,
            "low_battery"
        ) == BOBE_PROTOCOL_OK
    );
    CHECK(strstr(json, "\"type\":\"update.state\"") != NULL);
    CHECK(strstr(json, "\"phase\":\"failed\"") != NULL);
    CHECK(strstr(json, "\"reason\":\"low_battery\"") != NULL);
    CHECK(
        bobe_protocol_build_update_state(
            json,
            sizeof(json),
            "downloading",
            101,
            NULL
        ) == BOBE_PROTOCOL_INVALID_ARGUMENT
    );

    CHECK(
        bobe_protocol_build_playback_ready(
            json,
            sizeof(json),
            1,
            "lease",
            "turn",
            CONFIG_BOBE_AUDIO_PLAYBACK_CREDIT_MS
        ) == BOBE_PROTOCOL_OK
    );
    CHECK(
        bobe_protocol_build_playback_ready(
            json,
            sizeof(json),
            1,
            "lease",
            "turn",
            BOBE_AUDIO_FRAME_DURATION_MS - 1
        ) == BOBE_PROTOCOL_INVALID_ARGUMENT
    );
    CHECK(
        bobe_protocol_build_playback_ack(
            json,
            sizeof(json),
            1,
            "lease",
            "turn",
            UINT64_C(9007199254740991),
            0
        ) == BOBE_PROTOCOL_OK
    );
    CHECK(
        bobe_protocol_build_playback_ack(
            json,
            sizeof(json),
            1,
            "lease",
            "turn",
            UINT64_C(9007199254740992),
            0
        ) == BOBE_PROTOCOL_INVALID_ARGUMENT
    );
    CHECK(
        bobe_protocol_build_playback_drained(
            json,
            sizeof(json),
            4,
            "lease",
            "turn"
        ) == BOBE_PROTOCOL_OK
    );
    CHECK(strstr(json, "\"type\":\"audio.playback.drained\"") != NULL);
    CHECK(strstr(json, "\"stream_id\":4") != NULL);
    CHECK(
        bobe_protocol_build_playback_failed(
            json,
            sizeof(json),
            4,
            "lease",
            "turn",
            "physical_playback_timeout"
        ) == BOBE_PROTOCOL_OK
    );
    CHECK(strstr(json, "\"type\":\"audio.playback.failed\"") != NULL);
    CHECK(strstr(json, "\"reason\":\"physical_playback_timeout\"") != NULL);
}

static void test_state_machine(void)
{
    bobe_state_machine_t state;
    bobe_state_init(&state);
    CHECK(state.lifecycle == BOBE_LIFECYCLE_BOOTING);
    CHECK(state.turn == BOBE_TURN_IDLE);
    CHECK(
        !bobe_state_set_lifecycle(&state, BOBE_LIFECYCLE_ONLINE)
    );
    CHECK(
        !bobe_state_set_turn(&state, BOBE_TURN_REQUESTING_CAPTURE)
    );
    CHECK(
        bobe_state_set_lifecycle(
            &state,
            BOBE_LIFECYCLE_JOINING_WIFI
        )
    );
    CHECK(
        bobe_state_set_lifecycle(
            &state,
            BOBE_LIFECYCLE_DISCOVERING_HUB
        )
    );
    CHECK(
        bobe_state_set_lifecycle(&state, BOBE_LIFECYCLE_CONNECTING)
    );
    CHECK(bobe_state_set_lifecycle(&state, BOBE_LIFECYCLE_ONLINE));

    CHECK(bobe_state_set_turn(&state, BOBE_TURN_REQUESTING_CAPTURE));
    CHECK(bobe_state_set_turn(&state, BOBE_TURN_LISTENING));
    CHECK(bobe_state_set_turn(&state, BOBE_TURN_WAITING_FOR_RUNTIME));
    CHECK(bobe_state_set_turn(&state, BOBE_TURN_THINKING));
    CHECK(bobe_state_set_turn(&state, BOBE_TURN_SPEAKING));
    CHECK(bobe_state_set_turn(&state, BOBE_TURN_INTERRUPTED));
    CHECK(bobe_state_set_turn(&state, BOBE_TURN_IDLE));

    CHECK(bobe_state_set_turn(&state, BOBE_TURN_REQUESTING_CAPTURE));
    CHECK(bobe_state_set_turn(&state, BOBE_TURN_ABORTED));
    CHECK(bobe_state_set_turn(&state, BOBE_TURN_IDLE));
    CHECK(bobe_state_set_turn(&state, BOBE_TURN_REQUESTING_CAPTURE));
    CHECK(bobe_state_set_turn(&state, BOBE_TURN_LISTENING));
    CHECK(
        bobe_state_set_lifecycle(
            &state,
            BOBE_LIFECYCLE_RECONNECTING
        )
    );
    CHECK(state.turn == BOBE_TURN_ABORTED);
    CHECK(bobe_state_set_turn(&state, BOBE_TURN_IDLE));
    CHECK(!bobe_state_set_turn(&state, BOBE_TURN_REQUESTING_CAPTURE));

    bobe_state_init(&state);
    CHECK(
        bobe_state_set_lifecycle(
            &state,
            BOBE_LIFECYCLE_JOINING_WIFI
        )
    );
    CHECK(
        bobe_state_set_lifecycle(
            &state,
            BOBE_LIFECYCLE_DISCOVERING_HUB
        )
    );
    CHECK(bobe_state_set_lifecycle(&state, BOBE_LIFECYCLE_CONNECTING));
    CHECK(bobe_state_set_lifecycle(&state, BOBE_LIFECYCLE_ONLINE));
    CHECK(
        bobe_state_set_lifecycle(
            &state,
            BOBE_LIFECYCLE_UNPROVISIONED
        )
    );
    CHECK(state.turn == BOBE_TURN_IDLE);

    const struct {
        bobe_lifecycle_state_t lifecycle;
        bobe_face_expression_t face;
    } lifecycle_faces[] = {
        {BOBE_LIFECYCLE_BOOTING, BOBE_FACE_BOOTING},
        {BOBE_LIFECYCLE_UNPROVISIONED, BOBE_FACE_UNPROVISIONED},
        {BOBE_LIFECYCLE_JOINING_WIFI, BOBE_FACE_CONNECTING},
        {BOBE_LIFECYCLE_DISCOVERING_HUB, BOBE_FACE_CONNECTING},
        {BOBE_LIFECYCLE_CONNECTING, BOBE_FACE_CONNECTING},
        {BOBE_LIFECYCLE_RECONNECTING, BOBE_FACE_CONNECTING},
        {BOBE_LIFECYCLE_UPDATING, BOBE_FACE_UPDATING},
        {BOBE_LIFECYCLE_REBOOTING, BOBE_FACE_UPDATING},
        {BOBE_LIFECYCLE_LOW_BATTERY, BOBE_FACE_CONCERNED},
        {BOBE_LIFECYCLE_SHUTTING_DOWN, BOBE_FACE_SLEEPING},
        {BOBE_LIFECYCLE_FATAL_ERROR, BOBE_FACE_ERROR},
    };
    for (size_t index = 0;
         index < sizeof(lifecycle_faces) / sizeof(lifecycle_faces[0]);
         ++index) {
        state.lifecycle = lifecycle_faces[index].lifecycle;
        state.turn = BOBE_TURN_IDLE;
        CHECK(
            bobe_state_default_face(&state) ==
            lifecycle_faces[index].face
        );
    }
    state.lifecycle = BOBE_LIFECYCLE_JOINING_WIFI;
    CHECK(strcmp(bobe_state_default_caption(&state), "Joining Wi-Fi") == 0);
    state.lifecycle = BOBE_LIFECYCLE_DISCOVERING_HUB;
    CHECK(strcmp(bobe_state_default_caption(&state), "Finding BoBe") == 0);
    state.lifecycle = BOBE_LIFECYCLE_CONNECTING;
    CHECK(
        strcmp(bobe_state_default_caption(&state), "Connecting to BoBe") == 0
    );
    state.lifecycle = BOBE_LIFECYCLE_ONLINE;
    state.turn = BOBE_TURN_LISTENING;
    CHECK(bobe_state_default_face(&state) == BOBE_FACE_LISTENING);
    state.turn = BOBE_TURN_WAITING_FOR_RUNTIME;
    CHECK(bobe_state_default_face(&state) == BOBE_FACE_THINKING);
    state.turn = BOBE_TURN_SPEAKING;
    CHECK(bobe_state_default_face(&state) == BOBE_FACE_SPEAKING);
    state.microphone_muted = true;
    CHECK(bobe_state_default_face(&state) == BOBE_FACE_MUTED);
    CHECK(
        strcmp(bobe_state_default_caption(&state), "Microphone muted") == 0
    );
    state.lifecycle = BOBE_LIFECYCLE_FATAL_ERROR;
    CHECK(bobe_state_default_face(&state) == BOBE_FACE_ERROR);
    CHECK(strcmp(bobe_state_default_caption(&state), "Needs help") == 0);
    state.microphone_muted = false;
    CHECK(bobe_state_default_face(NULL) == BOBE_FACE_ERROR);

    bobe_face_state_t face = {0};
    CHECK(bobe_state_face_targets_turn(&face, "lease", "turn"));
    strcpy(face.lease_id, "lease");
    strcpy(face.turn_id, "turn");
    CHECK(bobe_state_face_targets_turn(&face, "lease", "turn"));
    CHECK(!bobe_state_face_targets_turn(&face, "stale", "turn"));
    CHECK(!bobe_state_face_targets_turn(&face, "lease", "stale"));
    CHECK(!bobe_state_face_targets_turn(NULL, "lease", "turn"));
}

static void test_battery_curve(void)
{
    CHECK(bobe_battery_percent_from_raw(-1) == 0);
    CHECK(bobe_battery_percent_from_raw(1970) == 0);
    CHECK(bobe_battery_percent_from_raw(2016) == 10);
    CHECK(bobe_battery_percent_from_raw(2062) == 20);
    CHECK(bobe_battery_percent_from_raw(2154) == 40);
    CHECK(bobe_battery_percent_from_raw(2246) == 60);
    CHECK(bobe_battery_percent_from_raw(2338) == 80);
    CHECK(bobe_battery_percent_from_raw(2430) == 100);
    CHECK(bobe_battery_percent_from_raw(4095) == 100);
}

int main(void)
{
    test_media_framing();
    test_media_payloads();
    test_control_parser();
    test_control_builders();
    test_state_machine();
    test_battery_curve();
    puts("BoBe host protocol, state, and battery tests passed.");
    return 0;
}
