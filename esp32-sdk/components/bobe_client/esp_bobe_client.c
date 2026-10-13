#include "esp_bobe_client.h"

#include "bobe_transport.h"
#include "esp_app_desc.h"
#include "bobe_identity.h"

#include <stdatomic.h>
#include <stdlib.h>
#include <string.h>

static atomic_bool initialized;
static atomic_bool initializing;
static esp_bobe_event_handler_t handler;
static void *handler_context;

esp_err_t esp_bobe_client_device_id(char *output, size_t capacity)
{
    return bobe_identity_device_id(output, capacity);
}

void esp_bobe_request_id(char output[37])
{
    bobe_identity_request_id(output);
}

static void transport_event(const bobe_transport_event_t *input, void *unused)
{
    (void)unused;
    esp_bobe_event_t event = {.generation = input->generation};
    switch (input->kind) {
    case BOBE_TRANSPORT_LINK_CONNECTED:
        return;
    case BOBE_TRANSPORT_SESSION_READY:
        event.kind = ESP_BOBE_CONNECTED;
        break;
    case BOBE_TRANSPORT_DISCONNECTED:
        event.kind = ESP_BOBE_DISCONNECTED;
        break;
    case BOBE_TRANSPORT_ERROR:
        event.kind = ESP_BOBE_ERROR;
        event.error = input->data.error;
        break;
    case BOBE_TRANSPORT_AUDIO:
        return;
    case BOBE_TRANSPORT_CONTROL:
        event.kind = ESP_BOBE_CONTROL;
        event.control = &input->data.control;
        break;
    }
    if (handler != NULL) handler(&event, handler_context);
}

esp_err_t esp_bobe_client_init(const esp_bobe_client_config_t *config)
{
    if (config == NULL || config->hardware_model == NULL ||
        (config->capabilities.text_output && config->on_event == NULL) ||
        config->capabilities.microphone || config->capabilities.speaker ||
        config->capabilities.aec || config->capabilities.wake_word ||
        (!config->capabilities.text_input && !config->capabilities.text_output &&
         !config->capabilities.actions && !config->capabilities.sensors)) return ESP_ERR_INVALID_ARG;
    if (atomic_load(&initialized) || atomic_exchange(&initializing, true)) return ESP_ERR_INVALID_STATE;
    esp_err_t result = ESP_OK;
    char active_device_id[BOBE_DEVICE_ID_MAX_BYTES];
    if (config->device_id != NULL) {
        if (strlen(config->device_id) >= sizeof(active_device_id)) result = ESP_ERR_INVALID_SIZE;
        else strcpy(active_device_id, config->device_id);
    } else {
        result = esp_bobe_client_device_id(active_device_id, sizeof(active_device_id));
    }
    if (result != ESP_OK) {
        atomic_store(&initializing, false);
        return result;
    }
    char boot_id[33];
    bobe_identity_boot_id(boot_id);
    bobe_transport_credentials_t credentials = {0};
    if (config->credentials != NULL) {
        credentials = (bobe_transport_credentials_t){
            .endpoint = config->credentials->endpoint, .server_ca = config->credentials->server_ca,
            .client_certificate = config->credentials->client_certificate,
            .client_key = config->credentials->client_key,
            .controller_epoch = config->credentials->controller_epoch,
        };
    }
    const bobe_transport_config_t transport = {
        .hello = {
            .device_id = active_device_id, .boot_id = boot_id,
            .firmware_version = esp_app_get_description()->version,
            .hardware_revision = config->hardware_model,
            .capabilities = config->capabilities,
        },
        .credentials = config->credentials != NULL ? &credentials : NULL,
    };
    result = esp_bobe_set_event_handler(config->on_event, config->context);
    if (result == ESP_OK) result = bobe_transport_init(&transport, NULL, NULL);
    atomic_store(&initialized, result == ESP_OK);
    atomic_store(&initializing, false);
    return result;
}

esp_err_t esp_bobe_set_event_handler(esp_bobe_event_handler_t on_event, void *context)
{
    const esp_err_t result = bobe_transport_set_observer(transport_event, NULL);
    if (result != ESP_OK) return result;
    handler = on_event;
    handler_context = context;
    return ESP_OK;
}

esp_err_t esp_bobe_client_start(void)
{
    return atomic_load(&initialized) ? bobe_transport_start() : ESP_ERR_INVALID_STATE;
}

esp_err_t esp_bobe_client_stop(void)
{
    return atomic_load(&initialized) ? bobe_transport_stop() : ESP_ERR_INVALID_STATE;
}

bool esp_bobe_client_is_connected(void)
{
    return bobe_transport_is_connected();
}

bool esp_bobe_client_is_configured(void)
{
    return bobe_transport_is_configured();
}

static esp_err_t send_encoded(char *json, bobe_protocol_result_t result)
{
    esp_err_t error;
    if (result == BOBE_PROTOCOL_OK) {
        error = bobe_transport_try_send_control(json, strlen(json)) ? ESP_OK : ESP_ERR_TIMEOUT;
    } else {
        error = result == BOBE_PROTOCOL_TOO_LARGE ? ESP_ERR_INVALID_SIZE : ESP_ERR_INVALID_ARG;
    }
    free(json);
    return error;
}

#define PREPARE_SEND(capability) \
    if (!esp_bobe_client_is_connected()) return ESP_ERR_INVALID_STATE; \
    const bobe_capabilities_t *capabilities = bobe_transport_capabilities(); \
    if (capabilities == NULL) return ESP_ERR_INVALID_STATE; \
    if (!(capability)) return ESP_ERR_NOT_SUPPORTED; \
    char *json = malloc(BOBE_CONTROL_MAX_BYTES + 1U); \
    if (json == NULL) return ESP_ERR_NO_MEM

esp_err_t esp_bobe_send_text(const char *request_id, const char *text)
{
    PREPARE_SEND(capabilities->text_input);
    return send_encoded(json, bobe_protocol_build_text_input(json, BOBE_CONTROL_MAX_BYTES + 1U, request_id, text));
}

esp_err_t esp_bobe_send_action(const char *request_id, const char *action, const char *label)
{
    PREPARE_SEND(capabilities->actions);
    return send_encoded(json, bobe_protocol_build_action(json, BOBE_CONTROL_MAX_BYTES + 1U, request_id, action, label));
}

esp_err_t esp_bobe_cancel_input(const char *request_id)
{
    PREPARE_SEND(capabilities->text_input || capabilities->actions);
    return send_encoded(json, bobe_protocol_build_input_cancel(json, BOBE_CONTROL_MAX_BYTES + 1U, request_id));
}

esp_err_t esp_bobe_report_number(const char *name, uint32_t sequence, double value, const char *unit)
{
    PREPARE_SEND(capabilities->sensors);
    return send_encoded(json, bobe_protocol_build_sensor_number(json, BOBE_CONTROL_MAX_BYTES + 1U, name, sequence, value, unit));
}

esp_err_t esp_bobe_report_boolean(const char *name, uint32_t sequence, bool value)
{
    PREPARE_SEND(capabilities->sensors);
    return send_encoded(json, bobe_protocol_build_sensor_boolean(json, BOBE_CONTROL_MAX_BYTES + 1U, name, sequence, value));
}
