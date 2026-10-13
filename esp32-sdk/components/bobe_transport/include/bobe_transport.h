#pragma once

#include "bobe_protocol.h"
#include "bobe_types.h"
#include "bobe_update_offer.h"
#include "esp_err.h"

#include <stdbool.h>
#include <stddef.h>

typedef enum {
    BOBE_TRANSPORT_LINK_CONNECTED = 0,
    BOBE_TRANSPORT_SESSION_READY,
    BOBE_TRANSPORT_DISCONNECTED,
    BOBE_TRANSPORT_CONTROL,
    BOBE_TRANSPORT_AUDIO,
    BOBE_TRANSPORT_ERROR,
} bobe_transport_event_kind_t;

typedef struct {
    bobe_transport_event_kind_t kind;
    uint32_t generation;
    union {
        bobe_control_message_t control;
        bobe_audio_frame_t audio;
        esp_err_t error;
    } data;
} bobe_transport_event_t;

typedef void (*bobe_transport_callback_t)(
    const bobe_transport_event_t *event,
    void *context
);

typedef struct {
    const char *endpoint;
    const char *server_ca;
    const char *client_certificate;
    const char *client_key;
    uint64_t controller_epoch;
} bobe_transport_credentials_t;

typedef bobe_update_offer_result_t (*bobe_transport_update_verifier_t)(
    const char *, size_t, const bobe_update_manifest_context_t *,
    bobe_update_manifest_t *, bobe_update_manifest_result_t *
);

typedef struct {
    bobe_hello_info_t hello;
    /* NULL uses the existing provisioned NVS claim. Strings are copied at init. */
    const bobe_transport_credentials_t *credentials;
    uint32_t build_number;
    uint32_t max_image_size;
    bobe_transport_update_verifier_t verify_update;
} bobe_transport_config_t;

esp_err_t bobe_transport_init(
    const bobe_transport_config_t *config,
    bobe_transport_callback_t callback,
    void *context
);
esp_err_t bobe_transport_start(void);
esp_err_t bobe_transport_stop(void);
esp_err_t bobe_transport_set_observer(bobe_transport_callback_t callback, void *context);
const bobe_capabilities_t *bobe_transport_capabilities(void);
bool bobe_transport_is_connected(void);
uint32_t bobe_transport_generation(void);
uint32_t bobe_transport_playback_sequence_gaps(void);
bool bobe_transport_is_configured(void);
esp_err_t bobe_transport_reload_configuration(void);
esp_err_t bobe_transport_configure_endpoint(const char *uri);
esp_err_t bobe_transport_configure_stored_endpoint(void);
bool bobe_transport_send_control(const char *json, size_t length);
bool bobe_transport_try_send_control(const char *json, size_t length);
bool bobe_transport_send_control_sync(
    const char *json,
    size_t length,
    uint32_t timeout_ms
);
bool bobe_transport_send_audio(const bobe_audio_frame_t *frame);
bool bobe_transport_flush_audio(uint32_t timeout_ms);
void bobe_transport_discard_audio(void);
void bobe_transport_set_playback_stream(uint32_t stream_id);
void bobe_transport_clear_playback_stream(void);
void bobe_transport_confirm_authenticated_time(void);
void bobe_transport_fail_session(esp_err_t error);
