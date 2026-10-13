#pragma once

#include "bobe_protocol.h"
#include "esp_err.h"

#ifdef __cplusplus
extern "C" {
#endif

#define ESP_BOBE_VERSION "0.2.0"

typedef enum {
    ESP_BOBE_CONNECTED,
    ESP_BOBE_DISCONNECTED,
    ESP_BOBE_CONTROL,
    ESP_BOBE_ERROR,
} esp_bobe_event_kind_t;

typedef struct {
    esp_bobe_event_kind_t kind;
    uint32_t generation;
    esp_err_t error;
    /* Borrowed for the duration of the callback; copy anything you retain. */
    const bobe_control_message_t *control;
} esp_bobe_event_t;

typedef void (*esp_bobe_event_handler_t)(const esp_bobe_event_t *event, void *context);

typedef struct {
    const char *endpoint;
    const char *server_ca;
    const char *client_certificate;
    const char *client_key;
    uint64_t controller_epoch;
} esp_bobe_credentials_t;

typedef struct {
    /* NULL uses the shared persistent device ID (NVS must be initialized). */
    const char *device_id;
    const char *hardware_model;
    bobe_capabilities_t capabilities;
    /* NULL loads an existing provisioned NVS claim. Non-NULL PEMs are copied. */
    const esp_bobe_credentials_t *credentials;
    esp_bobe_event_handler_t on_event;
    void *context;
} esp_bobe_client_config_t;

/** Initialize one lightweight client per MCU, without owning Wi-Fi or display.
 * Call after NVS/network setup. This mode accepts non-audio capabilities; use
 * esp_bobe_start() and a board port for the complete voice body.
 * Callback work must be bounded and nonblocking. Never call stop from a callback.
 * The client retains its bounded queues for the firmware lifetime; stop/start
 * can be repeated. Init may be retried after an initialization failure.
 */
esp_err_t esp_bobe_client_init(const esp_bobe_client_config_t *config);
/** Register before starting either the lightweight client or the voice runtime.
 * This also exposes interaction events on a voice body with enabled capabilities.
 */
esp_err_t esp_bobe_set_event_handler(esp_bobe_event_handler_t on_event, void *context);
esp_err_t esp_bobe_client_start(void);
esp_err_t esp_bobe_client_stop(void);
bool esp_bobe_client_is_connected(void);
bool esp_bobe_client_is_configured(void);

/** Caller-owned 37-byte buffer. Retain the ID until the request is resolved.
 * Never automatically retry a user action with a new request ID.
 */
void esp_bobe_request_id(char output[37]);
esp_err_t esp_bobe_client_device_id(char *output, size_t capacity);

/** Submit complete user input, not individual key presses. No offline replay. */
esp_err_t esp_bobe_send_text(const char *request_id, const char *text);
esp_err_t esp_bobe_send_action(const char *request_id, const char *action, const char *label);
esp_err_t esp_bobe_cancel_input(const char *request_id);
esp_err_t esp_bobe_report_number(const char *name, uint32_t sequence, double value, const char *unit);
esp_err_t esp_bobe_report_boolean(const char *name, uint32_t sequence, bool value);

#ifdef __cplusplus
}
#endif
