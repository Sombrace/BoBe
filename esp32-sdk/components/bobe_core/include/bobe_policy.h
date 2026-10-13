#pragma once

#include "bobe_notice.h"
#include "bobe_protocol.h"
#include "bobe_state.h"

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define BOBE_POLICY_EVENT_QUEUE_CAPACITY 16U
#define BOBE_POLICY_MAX_EFFECTS 16U
#define BOBE_POLICY_URI_MAX_BYTES 256U
#define BOBE_POLICY_CONTEXT_MAX_BYTES 1024U
#define BOBE_POLICY_EVENT_MAX_BYTES 1024U
#define BOBE_POLICY_EFFECT_MAX_BYTES 512U
#define BOBE_POLICY_EFFECT_BATCH_MAX_BYTES 8192U

#define BOBE_POLICY_CAPTURE_REQUEST_TIMEOUT_US UINT64_C(10000000)
#define BOBE_POLICY_OTA_CONFIRM_TIMEOUT_US UINT64_C(600000000)
#define BOBE_POLICY_TRANSPORT_RETRY_BASE_MS 500U
#define BOBE_POLICY_TRANSPORT_RETRY_MAX_MS 30000U

#define BOBE_POLICY_CRITICAL_ACTION_RELEASE (UINT32_C(1) << 0)
#define BOBE_POLICY_CRITICAL_NETWORK_LOST (UINT32_C(1) << 1)
#define BOBE_POLICY_CRITICAL_TRANSPORT_LOST (UINT32_C(1) << 2)
#define BOBE_POLICY_CRITICAL_CAPTURE_FAILED (UINT32_C(1) << 3)
#define BOBE_POLICY_CRITICAL_WAKE_DETECTED (UINT32_C(1) << 4)
#define BOBE_POLICY_CRITICAL_EVENT_OVERFLOW (UINT32_C(1) << 5)
#define BOBE_POLICY_CRITICAL_SPEECH_ENDED (UINT32_C(1) << 6)
#define BOBE_POLICY_CRITICAL_SHUTDOWN (UINT32_C(1) << 7)
#define BOBE_POLICY_CRITICAL_PROVISIONING_RECOVERY (UINT32_C(1) << 8)
#define BOBE_POLICY_CRITICAL_PROVISIONING_COMPLETE (UINT32_C(1) << 9)
#define BOBE_POLICY_CRITICAL_PROVISIONING_ABORTED (UINT32_C(1) << 10)
#define BOBE_POLICY_CRITICAL_PLAYBACK_FAILED (UINT32_C(1) << 11)

#define BOBE_POLICY_TICK_BATTERY (UINT32_C(1) << 0)
#define BOBE_POLICY_TICK_HEALTH (UINT32_C(1) << 1)

typedef enum {
    BOBE_POLICY_TRIGGER_ACTION_BUTTON = 0,
    BOBE_POLICY_TRIGGER_WAKE_WORD,
} bobe_policy_capture_trigger_t;

typedef enum {
    BOBE_POLICY_REASON_NONE = 0,
    BOBE_POLICY_REASON_BUTTON_RELEASED,
    BOBE_POLICY_REASON_VAD_END,
    BOBE_POLICY_REASON_AUDIO_START_FAILED,
    BOBE_POLICY_REASON_CAPTURE_STOP_FAILED,
    BOBE_POLICY_REASON_CAPTURE_FRONTEND_DRAIN_FAILED,
    BOBE_POLICY_REASON_UPLINK_BACKPRESSURE,
    BOBE_POLICY_REASON_CAPTURE_CLOSE_FAILED,
    BOBE_POLICY_REASON_REQUEST_TIMEOUT,
    BOBE_POLICY_REASON_NETWORK_LOST,
    BOBE_POLICY_REASON_BODYLINK_DISCONNECTED,
    BOBE_POLICY_REASON_MICROPHONE_MUTED,
    BOBE_POLICY_REASON_PROVISIONING_RECOVERY,
    BOBE_POLICY_REASON_SHUTDOWN,
    BOBE_POLICY_REASON_LOCAL_LEASE_FAILSAFE_EXPIRED,
    BOBE_POLICY_REASON_PHYSICAL_PLAYBACK_TIMEOUT,
    BOBE_POLICY_REASON_PHYSICAL_PLAYBACK_FAILED,
} bobe_policy_reason_t;

typedef struct {
    bool valid;
    uint32_t stream_id;
    char request_id[BOBE_REQUEST_ID_MAX_BYTES];
} bobe_policy_capture_seed_t;

typedef enum {
    BOBE_POLICY_TRANSPORT_LINK_CONNECTED = 0,
    BOBE_POLICY_TRANSPORT_SESSION_READY,
    BOBE_POLICY_TRANSPORT_DISCONNECTED,
    BOBE_POLICY_TRANSPORT_CONTROL,
    BOBE_POLICY_TRANSPORT_ERROR,
} bobe_policy_transport_kind_t;

typedef enum {
    BOBE_POLICY_OPERATION_NONE = 0,
    BOBE_POLICY_OPERATION_CAPTURE_REQUEST,
    BOBE_POLICY_OPERATION_CAPTURE_OPEN,
    BOBE_POLICY_OPERATION_VOICE_BEGIN,
    BOBE_POLICY_OPERATION_CAPTURE_START,
    BOBE_POLICY_OPERATION_CAPTURE_STOP,
    BOBE_POLICY_OPERATION_VOICE_END,
    BOBE_POLICY_OPERATION_CAPTURE_FLUSH,
    BOBE_POLICY_OPERATION_CAPTURE_CLOSE,
    BOBE_POLICY_OPERATION_PLAYBACK_OPEN,
    BOBE_POLICY_OPERATION_PLAYBACK_READY,
    BOBE_POLICY_OPERATION_PLAYBACK_CLOSE,
    BOBE_POLICY_OPERATION_PLAYBACK_DRAINED,
    BOBE_POLICY_OPERATION_PLAYBACK_FAILED,
    BOBE_POLICY_OPERATION_TRANSPORT_START,
    BOBE_POLICY_OPERATION_TRANSPORT_STOP,
    BOBE_POLICY_OPERATION_TRANSPORT_RELOAD,
    BOBE_POLICY_OPERATION_CONFIGURE_STORED_ENDPOINT,
    BOBE_POLICY_OPERATION_CONFIGURE_DISCOVERED_ENDPOINT,
    BOBE_POLICY_OPERATION_DISCOVERY_START,
    BOBE_POLICY_OPERATION_PROVISIONING_OPEN,
    BOBE_POLICY_OPERATION_OTA_CONFIRM,
    BOBE_POLICY_OPERATION_CLAIM_REVOKE,
    BOBE_POLICY_OPERATION_UPDATE_INSTALL,
} bobe_policy_operation_t;

typedef enum {
    BOBE_POLICY_RESULT_OK = 0,
    BOBE_POLICY_RESULT_FAILED,
    BOBE_POLICY_RESULT_TIMEOUT,
} bobe_policy_result_t;

typedef enum {
    BOBE_POLICY_EVENT_BOOT = 0,
    BOBE_POLICY_EVENT_VOICE_READY,
    BOBE_POLICY_EVENT_BUTTON,
    BOBE_POLICY_EVENT_NETWORK,
    BOBE_POLICY_EVENT_TRANSPORT,
    BOBE_POLICY_EVENT_PLAYBACK_PROGRESS,
    BOBE_POLICY_EVENT_CRITICAL,
    BOBE_POLICY_EVENT_TIME_TICK,
    BOBE_POLICY_EVENT_HUB_DISCOVERY,
    BOBE_POLICY_EVENT_BATTERY,
    BOBE_POLICY_EVENT_EFFECT_RESULT,
} bobe_policy_event_kind_t;

typedef struct {
    uint32_t flags;
    uint32_t latest_action_release_event_id;
    bool physical_action_pressed;
    bool physical_volume_down_pressed;
    bool physical_volume_up_pressed;
    bool network_online;
    bool provisioning;
    bool transport_connected;
    bool transport_configured;
    bobe_policy_capture_seed_t capture_seed;
} bobe_policy_critical_event_t;

typedef struct {
    bobe_policy_event_kind_t kind;
    uint64_t now_us;
    union {
        struct {
            bool microphone_muted;
            bool ota_pending_verification;
            uint8_t volume;
            bobe_face_theme_id_t theme;
        } boot;
        struct {
            bobe_button_event_t event;
            bool physical_action_pressed;
            bool physical_volume_down_pressed;
            bool physical_volume_up_pressed;
            bool transport_connected;
            bool transport_configured;
            bool provisioning;
            uint8_t current_volume;
            uint32_t latest_action_release_event_id;
            bobe_policy_capture_seed_t capture_seed;
        } button;
        struct {
            bobe_network_state_t state;
            uint32_t generation;
            uint32_t current_generation;
            bool provisioning;
            bool network_online;
            bool transport_configured;
        } network;
        struct {
            bobe_policy_transport_kind_t kind;
            uint32_t generation;
            uint32_t current_generation;
            bool audio_capturing;
            bool ota_pending_verification;
            int error;
            bobe_control_message_t control;
        } transport;
        struct {
            uint32_t stream_id;
            uint32_t route_generation;
            uint64_t played_samples;
            uint32_t credit_ms;
        } playback_progress;
        bobe_policy_critical_event_t critical;
        struct {
            uint32_t periodic_flags;
            bool network_online;
            bool provisioning;
        } tick;
        struct {
            bool success;
            uint32_t network_generation;
            uint32_t current_network_generation;
            bool network_online;
            bool transport_configured;
            char uri[BOBE_POLICY_URI_MAX_BYTES];
        } discovery;
        struct {
            bool valid;
            bobe_battery_state_t state;
            bool transport_connected;
        } battery;
        struct {
            bobe_policy_operation_t operation;
            bobe_policy_result_t result;
            uint32_t retry_random;
            uint32_t transport_generation;
            bool transport_connected;
            bool transport_configured;
            bool network_online;
        } effect_result;
    } data;
} bobe_policy_event_t;

typedef enum {
    BOBE_POLICY_EFFECT_DISPLAY_FACE = 0,
    BOBE_POLICY_EFFECT_DISPLAY_THEME,
    BOBE_POLICY_EFFECT_DISPLAY_MICROPHONE,
    BOBE_POLICY_EFFECT_DISPLAY_NOTICE,
    BOBE_POLICY_EFFECT_WAKE_MONITOR_START,
    BOBE_POLICY_EFFECT_WAKE_MONITOR_STOP,
    BOBE_POLICY_EFFECT_AUDIO_CAPTURE_START,
    BOBE_POLICY_EFFECT_AUDIO_CAPTURE_STOP,
    BOBE_POLICY_EFFECT_VOICE_STREAM_BEGIN,
    BOBE_POLICY_EFFECT_VOICE_STREAM_END,
    BOBE_POLICY_EFFECT_AUDIO_PLAYBACK_OPEN,
    BOBE_POLICY_EFFECT_AUDIO_PLAYBACK_CLOSE,
    BOBE_POLICY_EFFECT_AUDIO_PLAYBACK_ABORT,
    BOBE_POLICY_EFFECT_AUDIO_SET_MUTE,
    BOBE_POLICY_EFFECT_AUDIO_SET_VOLUME,
    BOBE_POLICY_EFFECT_TRANSPORT_CAPTURE_REQUEST,
    BOBE_POLICY_EFFECT_TRANSPORT_CAPTURE_STATE,
    BOBE_POLICY_EFFECT_TRANSPORT_CAPTURE_CANCEL,
    BOBE_POLICY_EFFECT_TRANSPORT_PLAYBACK_READY,
    BOBE_POLICY_EFFECT_TRANSPORT_PLAYBACK_ACK,
    BOBE_POLICY_EFFECT_TRANSPORT_PLAYBACK_DRAINED,
    BOBE_POLICY_EFFECT_TRANSPORT_PLAYBACK_FAILED,
    BOBE_POLICY_EFFECT_TRANSPORT_BUTTON,
    BOBE_POLICY_EFFECT_TRANSPORT_START,
    BOBE_POLICY_EFFECT_TRANSPORT_STOP,
    BOBE_POLICY_EFFECT_TRANSPORT_RELOAD,
    BOBE_POLICY_EFFECT_TRANSPORT_CONFIGURE_STORED,
    BOBE_POLICY_EFFECT_TRANSPORT_CONFIGURE_ENDPOINT,
    BOBE_POLICY_EFFECT_TRANSPORT_DISCARD_AUDIO,
    BOBE_POLICY_EFFECT_TRANSPORT_FLUSH_AUDIO,
    BOBE_POLICY_EFFECT_TRANSPORT_SET_PLAYBACK_ROUTE,
    BOBE_POLICY_EFFECT_TRANSPORT_CLEAR_PLAYBACK_ROUTE,
    BOBE_POLICY_EFFECT_TRANSPORT_FAIL_SESSION,
    BOBE_POLICY_EFFECT_CONNECTIVITY_CONFIRM_TIME,
    BOBE_POLICY_EFFECT_CONNECTIVITY_OPEN_PROVISIONING,
    BOBE_POLICY_EFFECT_CONNECTIVITY_DISCOVER_HUB,
    BOBE_POLICY_EFFECT_OTA_CONFIRM,
    BOBE_POLICY_EFFECT_OTA_ROLLBACK,
    BOBE_POLICY_EFFECT_CLAIM_REVOKE,
    BOBE_POLICY_EFFECT_UPDATE_INSTALL,
    BOBE_POLICY_EFFECT_TRANSPORT_UPDATE_STATE,
    BOBE_POLICY_EFFECT_UPDATE_REBOOT,
    BOBE_POLICY_EFFECT_BOARD_SHUTDOWN,
    BOBE_POLICY_EFFECT_READ_BATTERY,
    BOBE_POLICY_EFFECT_SEND_BATTERY,
    BOBE_POLICY_EFFECT_REPORT_HEALTH,
} bobe_policy_effect_kind_t;

typedef enum {
    BOBE_POLICY_FAILURE_GENERIC = 0,
    BOBE_POLICY_FAILURE_TIMEOUT,
    BOBE_POLICY_FAILURE_NO_MEMORY,
} bobe_policy_failure_t;

typedef struct {
    bobe_policy_effect_kind_t kind;
    bobe_policy_operation_t operation;
    union {
        bobe_face_state_t face;
        bobe_theme_state_t theme;
        bobe_notice_t notice;
        bool microphone_active;
        struct {
            bool muted;
            bool persist;
        } mute;
        uint8_t volume;
        uint32_t stream_id;
        bobe_button_event_t button;
        bobe_policy_failure_t failure;
        struct {
            uint32_t stream_id;
            bobe_policy_capture_trigger_t trigger;
            char request_id[BOBE_REQUEST_ID_MAX_BYTES];
        } capture_request;
        struct {
            bool open;
            uint32_t stream_id;
            bobe_policy_reason_t reason;
            char lease_id[BOBE_LEASE_ID_MAX_BYTES];
            char turn_id[BOBE_TURN_ID_MAX_BYTES];
        } capture_state;
        struct {
            uint32_t stream_id;
            char request_id[BOBE_REQUEST_ID_MAX_BYTES];
        } capture_cancel;
        struct {
            uint32_t stream_id;
            uint64_t played_samples;
            uint32_t credit_ms;
            bobe_policy_reason_t reason;
            char lease_id[BOBE_LEASE_ID_MAX_BYTES];
            char turn_id[BOBE_TURN_ID_MAX_BYTES];
        } playback;
        struct {
            uint32_t network_generation;
        } discovery;
        struct {
            char uri[BOBE_POLICY_URI_MAX_BYTES];
        } endpoint;
        bobe_update_manifest_t update;
        struct {
            uint8_t progress_percent;
            char phase[16];
            char reason[48];
        } update_state;
        bobe_battery_state_t battery;
    } data;
} bobe_policy_effect_t;

typedef struct {
    size_t count;
    bool overflowed;
    bobe_policy_effect_t items[BOBE_POLICY_MAX_EFFECTS];
} bobe_policy_effect_batch_t;

typedef enum {
    BOBE_POLICY_WORKFLOW_NONE = 0,
    BOBE_POLICY_WORKFLOW_NETWORK_CONNECTED,
    BOBE_POLICY_WORKFLOW_DISCOVERY_RESULT,
    BOBE_POLICY_WORKFLOW_PROVISIONING_COMPLETE,
    BOBE_POLICY_WORKFLOW_PROVISIONING_RECOVERY,
} bobe_policy_workflow_t;

typedef struct {
    bobe_state_machine_t state;
    bobe_face_theme_id_t theme;
    uint64_t local_face_version;
    uint64_t remote_face_version;
    uint64_t theme_version;
    uint32_t capture_stream_id;
    uint32_t playback_stream_id;
    uint32_t playback_route_generation;
    char capture_request_id[BOBE_REQUEST_ID_MAX_BYTES];
    char lease_id[BOBE_LEASE_ID_MAX_BYTES];
    char turn_id[BOBE_TURN_ID_MAX_BYTES];
    uint64_t capture_request_deadline_us;
    uint64_t lease_deadline_us;
    uint64_t face_expiry_us;
    uint64_t ota_confirm_deadline_us;
    uint64_t transport_retry_deadline_us;
    uint32_t action_capture_press_event_id;
    uint32_t latest_action_release_event_id;
    uint32_t network_generation;
    uint32_t transport_generation;
    uint8_t transport_retry_attempt;
    uint8_t low_battery_samples;
    bool wake_speech_ended_pending;
    bool network_online;
    bool transport_connected;
    bool transport_configured;
    bool provisioning;
    bool pending_button_report;
    bobe_button_event_t pending_button;
    bobe_policy_operation_t pending_operation;
    bobe_policy_workflow_t pending_workflow;
    bobe_policy_reason_t pending_capture_finish_reason;
    bobe_turn_state_t capture_finish_origin;
    bool capture_stop_succeeded;
    uint32_t pending_playback_stream_id;
    bool pending_transport_start_is_retry;
    bool provisioning_restart_transport;
    bool battery_valid;
    bobe_battery_state_t battery;
    /* Deferred local notice; the next time tick flushes it. */
    bobe_notice_t pending_notice;
    bobe_policy_critical_event_t deferred_critical;
} bobe_policy_context_t;

void bobe_policy_init(bobe_policy_context_t *context);

bool bobe_policy_reduce(
    bobe_policy_context_t *context,
    const bobe_policy_event_t *event,
    bobe_policy_effect_batch_t *effects
);

const char *bobe_policy_reason_name(bobe_policy_reason_t reason);
