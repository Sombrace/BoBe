#pragma once

#include "bobe_types.h"
#include "bobe_update_manifest.h"

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define BOBE_PROTOCOL_MAJOR 1U
#define BOBE_PROTOCOL_MINOR 2U
#define BOBE_PROTOCOL_MIN_MINOR 1U
#define BOBE_BODY_MDNS_AUTH "mtls-pinned"
#define BOBE_MEDIA_HEADER_BYTES 20U
#define BOBE_MEDIA_FLAG_DISCONTINUITY 0x0001U
#define BOBE_MAX_PLAYBACK_CREDIT_MS 300U
#define BOBE_MAX_LEASE_LIFETIME_MS 600000U
#define BOBE_TEXT_INPUT_MAX_BYTES 1024U
#define BOBE_TEXT_CHUNK_MAX_BYTES 512U
#define BOBE_ACTION_ID_MAX_BYTES 64U
#define BOBE_ACTION_LABEL_MAX_BYTES 128U
#define BOBE_SENSOR_NAME_MAX_BYTES 32U
#define BOBE_SENSOR_UNIT_MAX_BYTES 16U

#ifdef __cplusplus
extern "C" {
#endif

typedef enum {
    BOBE_MEDIA_MICROPHONE_PCM = 0x01,
    BOBE_MEDIA_SPEAKER_PCM = 0x02,
} bobe_media_kind_t;

typedef struct {
    uint8_t protocol_major;
    bobe_media_kind_t kind;
    uint16_t flags;
    uint32_t stream_id;
    uint32_t sequence;
    uint64_t monotonic_timestamp_us;
} bobe_media_header_t;

typedef enum {
    BOBE_PROTOCOL_OK = 0,
    BOBE_PROTOCOL_INVALID_ARGUMENT,
    BOBE_PROTOCOL_TOO_LARGE,
    BOBE_PROTOCOL_MALFORMED,
    BOBE_PROTOCOL_UNSUPPORTED,
    BOBE_PROTOCOL_VERSION_MISMATCH,
    BOBE_PROTOCOL_RANGE_ERROR,
} bobe_protocol_result_t;

typedef struct {
    bool microphone;
    bool speaker;
    bool battery;
    bool aec;
    bool wake_word;
    bool text_input;
    bool text_output;
    bool actions;
    bool sensors;
    uint16_t display_width;
    uint16_t display_height;
    uint8_t face_schema;
    uint16_t text_columns;
    uint16_t text_rows;
} bobe_capabilities_t;

typedef struct {
    const char *device_id;
    const char *boot_id;
    const char *firmware_version;
    const char *hardware_revision;
    bobe_capabilities_t capabilities;
    uint16_t max_playback_credit_ms;
} bobe_hello_info_t;

typedef struct {
    uint32_t uptime_s;
    int wifi_rssi_dbm;
    size_t free_internal_heap;
    size_t minimum_internal_heap;
    size_t free_psram;
    uint32_t audio_rx_overruns;
    uint32_t audio_tx_underruns;
    bobe_lifecycle_state_t lifecycle;
    bobe_turn_state_t turn;
    bool microphone_muted;
    const char *app_version;
    uint32_t build_number;
    uint8_t running_slot;
    bool ota_pending_verification;
    uint32_t reset_reason;
    uint32_t transport_generation;
    uint32_t core_queue_depth;
    uint32_t core_queue_capacity;
    uint32_t core_stack_free_words;
    uint32_t display_stack_free_words;
    uint32_t display_flush_submitted;
    uint32_t display_flush_errors;
    uint32_t display_flush_timeouts;
    uint32_t display_flush_last_us;
    uint32_t display_flush_max_us;
    uint32_t playback_sequence_gaps;
    int32_t temperature_millicelsius;
    uint32_t cpu_frequency_mhz;
    uint32_t display_frame_rate;
    uint8_t backlight_percent;
} bobe_health_info_t;

typedef enum {
    BOBE_CONTROL_UNKNOWN = 0,
    BOBE_CONTROL_WELCOME,
    BOBE_CONTROL_CAPTURE_GRANTED,
    BOBE_CONTROL_CAPTURE_DENIED,
    BOBE_CONTROL_FACE_SET,
    BOBE_CONTROL_THEME_SET,
    BOBE_CONTROL_PLAYBACK_OPEN,
    BOBE_CONTROL_PLAYBACK_CLOSE,
    BOBE_CONTROL_TURN_STATE,
    BOBE_CONTROL_UPDATE_OFFER,
    BOBE_CONTROL_ERROR,
    BOBE_CONTROL_INPUT_RESULT,
    BOBE_CONTROL_DISPLAY_TEXT,
    BOBE_CONTROL_SENSOR_ACK,
} bobe_control_kind_t;

typedef struct {
    bobe_control_kind_t kind;
    union {
        struct {
            char session_id[BOBE_SESSION_ID_MAX_BYTES];
            uint8_t protocol_minor;
            uint32_t heartbeat_interval_ms;
            uint64_t controller_epoch;
        } welcome;
        struct {
            uint32_t stream_id;
            char request_id[BOBE_REQUEST_ID_MAX_BYTES];
            char lease_id[BOBE_LEASE_ID_MAX_BYTES];
            char turn_id[BOBE_TURN_ID_MAX_BYTES];
            char reason[96];
        } capture;
        bobe_face_state_t face;
        bobe_theme_state_t theme;
        struct {
            uint32_t stream_id;
            uint32_t sample_rate_hz;
            uint16_t frame_duration_ms;
            uint8_t channels;
            char lease_id[BOBE_LEASE_ID_MAX_BYTES];
            char turn_id[BOBE_TURN_ID_MAX_BYTES];
        } playback_open;
        struct {
            uint32_t stream_id;
            char lease_id[BOBE_LEASE_ID_MAX_BYTES];
            char turn_id[BOBE_TURN_ID_MAX_BYTES];
        } playback_close;
        struct {
            bobe_turn_state_t state;
            char lease_id[BOBE_LEASE_ID_MAX_BYTES];
            char turn_id[BOBE_TURN_ID_MAX_BYTES];
        } turn;
        bobe_update_manifest_t update;
        struct {
            char code[48];
            char message[160];
        } error;
        struct {
            char request_id[37];
            char status[16];
            char code[48];
            bool replayed;
        } input_result;
        struct {
            char request_id[37];
            uint32_t sequence;
            bool clear;
            bool final;
            char text[BOBE_TEXT_CHUNK_MAX_BYTES + 1U];
        } display_text;
        struct {
            char name[BOBE_SENSOR_NAME_MAX_BYTES + 1U];
            uint32_t sequence;
        } sensor_ack;
    } data;
} bobe_control_message_t;

uint8_t bobe_capabilities_protocol_minor(const bobe_capabilities_t *capabilities);
bool bobe_utf8_valid(const char *input, size_t length);
bool bobe_json_escapes_valid(const char *input, size_t length);

bobe_protocol_result_t bobe_protocol_build_text_input(
    char *output, size_t capacity, const char *request_id, const char *text
);
bobe_protocol_result_t bobe_protocol_build_action(
    char *output, size_t capacity, const char *request_id,
    const char *action, const char *label
);
bobe_protocol_result_t bobe_protocol_build_input_cancel(
    char *output, size_t capacity, const char *request_id
);
bobe_protocol_result_t bobe_protocol_build_sensor_number(
    char *output, size_t capacity, const char *name,
    uint32_t sequence, double value, const char *unit
);
bobe_protocol_result_t bobe_protocol_build_sensor_boolean(
    char *output, size_t capacity, const char *name, uint32_t sequence, bool value
);

void bobe_media_header_encode(
    const bobe_media_header_t *header,
    uint8_t output[BOBE_MEDIA_HEADER_BYTES]
);

bobe_protocol_result_t bobe_media_header_decode(
    const uint8_t *input,
    size_t input_size,
    bobe_media_header_t *header
);

bobe_protocol_result_t bobe_media_validate_payload(
    const bobe_media_header_t *header,
    size_t payload_size
);

bobe_protocol_result_t bobe_protocol_build_hello(
    char *output,
    size_t output_size,
    const bobe_hello_info_t *info
);

bobe_protocol_result_t bobe_protocol_build_health(
    char *output,
    size_t output_size,
    const bobe_health_info_t *info
);

bobe_protocol_result_t bobe_protocol_build_button_event(
    char *output,
    size_t output_size,
    const char *boot_id,
    const bobe_button_event_t *event
);

bobe_protocol_result_t bobe_protocol_build_battery_state(
    char *output,
    size_t output_size,
    const bobe_battery_state_t *battery
);

bobe_protocol_result_t bobe_protocol_build_capture_state(
    char *output,
    size_t output_size,
    bool open,
    uint32_t stream_id,
    const char *lease_id,
    const char *turn_id,
    const char *reason
);

bobe_protocol_result_t bobe_protocol_build_capture_request(
    char *output,
    size_t output_size,
    const char *request_id,
    uint32_t stream_id,
    const char *trigger
);

bobe_protocol_result_t bobe_protocol_build_capture_cancel(
    char *output,
    size_t output_size,
    const char *request_id,
    uint32_t stream_id
);

bobe_protocol_result_t bobe_protocol_build_playback_ready(
    char *output,
    size_t output_size,
    uint32_t stream_id,
    const char *lease_id,
    const char *turn_id,
    uint32_t credit_ms
);

bobe_protocol_result_t bobe_protocol_build_playback_ack(
    char *output,
    size_t output_size,
    uint32_t stream_id,
    const char *lease_id,
    const char *turn_id,
    uint64_t played_samples,
    uint32_t credit_ms
);

bobe_protocol_result_t bobe_protocol_build_playback_drained(
    char *output,
    size_t output_size,
    uint32_t stream_id,
    const char *lease_id,
    const char *turn_id
);
bobe_protocol_result_t bobe_protocol_build_playback_failed(
    char *output,
    size_t output_size,
    uint32_t stream_id,
    const char *lease_id,
    const char *turn_id,
    const char *reason
);

bobe_protocol_result_t bobe_protocol_build_update_state(
    char *output,
    size_t output_size,
    const char *phase,
    uint8_t progress_percent,
    const char *reason
);

bobe_protocol_result_t bobe_protocol_parse_control(
    const char *input,
    size_t input_size,
    bobe_control_message_t *message
);

const char *bobe_lifecycle_state_name(bobe_lifecycle_state_t state);
const char *bobe_turn_state_name(bobe_turn_state_t state);
const char *bobe_face_expression_name(bobe_face_expression_t expression);
const char *bobe_face_theme_name(bobe_face_theme_id_t theme);
bool bobe_face_theme_parse(const char *name, bobe_face_theme_id_t *theme);

#ifdef __cplusplus
}
#endif
