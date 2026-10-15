#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define BOBE_CONTROL_MAX_BYTES 4096U
#define BOBE_DEVICE_ID_MAX_BYTES 64U
#define BOBE_SESSION_ID_MAX_BYTES 64U
#define BOBE_REQUEST_ID_MAX_BYTES 64U
#define BOBE_LEASE_ID_MAX_BYTES 64U
#define BOBE_TURN_ID_MAX_BYTES 64U
#define BOBE_CAPTION_MAX_BYTES 160U
#define BOBE_NOTICE_MAX_BYTES 32U
#define BOBE_AUDIO_FRAME_DURATION_MS 20U
#define BOBE_MIC_SAMPLE_RATE_HZ 16000U
#define BOBE_SPEAKER_SAMPLE_RATE_HZ 24000U
#define BOBE_AUDIO_MAX_SAMPLES 480U
#define BOBE_MIC_FRAME_SAMPLES \
    (BOBE_MIC_SAMPLE_RATE_HZ * BOBE_AUDIO_FRAME_DURATION_MS / 1000U)
#define BOBE_SPEAKER_FRAME_SAMPLES \
    (BOBE_SPEAKER_SAMPLE_RATE_HZ * BOBE_AUDIO_FRAME_DURATION_MS / 1000U)
#define BOBE_FACE_DEFAULT_ACCENT_RGB UINT32_C(0x94472E)
#define BOBE_FACE_THEME_ACCENT_RGB UINT32_MAX

#ifdef __cplusplus
#define BOBE_STATIC_ASSERT static_assert
#else
#define BOBE_STATIC_ASSERT _Static_assert
#endif

BOBE_STATIC_ASSERT(
    BOBE_MIC_FRAME_SAMPLES <= BOBE_AUDIO_MAX_SAMPLES,
    "microphone frame exceeds shared audio capacity"
);
BOBE_STATIC_ASSERT(
    BOBE_SPEAKER_FRAME_SAMPLES <= BOBE_AUDIO_MAX_SAMPLES,
    "speaker frame exceeds shared audio capacity"
);
#undef BOBE_STATIC_ASSERT

typedef enum {
    BOBE_LIFECYCLE_BOOTING = 0,
    BOBE_LIFECYCLE_UNPROVISIONED,
    BOBE_LIFECYCLE_JOINING_WIFI,
    BOBE_LIFECYCLE_DISCOVERING_HUB,
    BOBE_LIFECYCLE_CONNECTING,
    BOBE_LIFECYCLE_ONLINE,
    BOBE_LIFECYCLE_RECONNECTING,
    BOBE_LIFECYCLE_UPDATING,
    BOBE_LIFECYCLE_REBOOTING,
    BOBE_LIFECYCLE_LOW_BATTERY,
    BOBE_LIFECYCLE_SHUTTING_DOWN,
    BOBE_LIFECYCLE_FATAL_ERROR,
} bobe_lifecycle_state_t;

typedef enum {
    BOBE_TURN_IDLE = 0,
    BOBE_TURN_REQUESTING_CAPTURE,
    BOBE_TURN_LISTENING,
    BOBE_TURN_WAITING_FOR_RUNTIME,
    BOBE_TURN_THINKING,
    BOBE_TURN_SPEAKING,
    BOBE_TURN_INTERRUPTED,
    BOBE_TURN_ABORTED,
} bobe_turn_state_t;

typedef enum {
    BOBE_FACE_BOOTING = 0,
    BOBE_FACE_UNPROVISIONED,
    BOBE_FACE_CONNECTING,
    BOBE_FACE_IDLE,
    BOBE_FACE_ATTENTIVE,
    BOBE_FACE_LISTENING,
    BOBE_FACE_THINKING,
    BOBE_FACE_SPEAKING,
    BOBE_FACE_HAPPY,
    BOBE_FACE_CONCERNED,
    BOBE_FACE_MUTED,
    BOBE_FACE_OFFLINE,
    BOBE_FACE_UPDATING,
    BOBE_FACE_ERROR,
    BOBE_FACE_SLEEPING,
} bobe_face_expression_t;

typedef struct {
    uint64_t state_version;
    bobe_face_expression_t expression;
    uint32_t accent_rgb;
    uint32_t ttl_ms;
    char lease_id[BOBE_LEASE_ID_MAX_BYTES];
    char turn_id[BOBE_TURN_ID_MAX_BYTES];
    char caption[BOBE_CAPTION_MAX_BYTES];
} bobe_face_state_t;

typedef enum {
    BOBE_FACE_THEME_DEFAULT = 0,
    BOBE_FACE_THEME_WARM_APERTURE,
    BOBE_FACE_THEME_WARM_LINTEL,
    BOBE_FACE_THEME_WARM_LEDGE,
    BOBE_FACE_THEME_KETTLE_TALL_BRIGHT,
    BOBE_FACE_THEME_KETTLE_SOFT_BOWL,
    BOBE_FACE_THEME_SOLAR_WIDE_AWAKE,
    BOBE_FACE_THEME_SOLAR_CROWN,
    BOBE_FACE_THEME_COUNT,
} bobe_face_theme_id_t;

#define BOBE_FACE_THEME_ID_MAX_BYTES 32U

typedef struct {
    uint64_t state_version;
    bobe_face_theme_id_t theme;
} bobe_theme_state_t;

typedef enum {
    BOBE_NOTICE_NONE = 0,
    BOBE_NOTICE_HINT,
    BOBE_NOTICE_VOLUME,
    BOBE_NOTICE_SILENT,
    BOBE_NOTICE_UPDATE,
} bobe_notice_kind_t;

#define BOBE_NOTICE_HINT_TTL_MS 4000U
#define BOBE_NOTICE_CONTROL_TTL_MS 1500U
#define BOBE_NOTICE_UPDATE_TTL_MS 5000U

typedef struct {
    bobe_notice_kind_t kind;
    uint32_t ttl_ms;
    char text[BOBE_NOTICE_MAX_BYTES];
} bobe_notice_t;

typedef enum {
    BOBE_BUTTON_ACTION = 0,
    BOBE_BUTTON_VOLUME_DOWN,
    BOBE_BUTTON_VOLUME_UP,
} bobe_button_id_t;

typedef enum {
    BOBE_BUTTON_PRESSED = 0,
    BOBE_BUTTON_RELEASED,
    BOBE_BUTTON_LONG_PRESS,
} bobe_button_action_t;

typedef struct {
    bobe_button_id_t button;
    bobe_button_action_t action;
    uint32_t event_id;
    uint64_t monotonic_us;
} bobe_button_event_t;

typedef struct {
    int raw_adc;
    uint8_t percent;
    bool charging;
} bobe_battery_state_t;

typedef struct {
    uint8_t battery_percent;
    int8_t wifi_rssi_dbm;
    bool battery_valid;
    bool charging;
    bool wifi_connected;
    bool microphone_muted;
} bobe_display_status_t;

typedef struct {
    uint32_t stream_id;
    uint32_t sequence;
    uint64_t monotonic_timestamp_us;
    size_t sample_count;
    int16_t samples[BOBE_AUDIO_MAX_SAMPLES];
} bobe_audio_frame_t;

typedef enum {
    BOBE_NETWORK_UNPROVISIONED = 0,
    BOBE_NETWORK_CONNECTING,
    BOBE_NETWORK_CONNECTED,
    BOBE_NETWORK_PROVISIONING_COMPLETE,
    BOBE_NETWORK_PROVISIONING_ABORTED,
    BOBE_NETWORK_DISCONNECTED,
    BOBE_NETWORK_AUTH_FAILED,
} bobe_network_state_t;
