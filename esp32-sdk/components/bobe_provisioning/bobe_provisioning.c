#include "bobe_provisioning.h"
#include "bobe_identity.h"

#include "bobe_provisioning_credentials.h"
#include "bobe_claim.h"
#include "bobe_storage.h"
#include "bobe_task_config.h"
#include "cJSON.h"
#include "esp_check.h"
#include "esp_log.h"
#include "esp_mac.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "mbedtls/pk.h"
#include "mbedtls/x509_crt.h"
#include "network_provisioning/manager.h"
#if defined(CONFIG_BOBE_PROVISIONING_TRANSPORT_BLE)
#if !defined(CONFIG_BT_NIMBLE_ENABLED)
#error "BLE provisioning requires NimBLE; include sdkconfig.ble.defaults"
#endif
#include "esp_bt.h"
#include "host/ble_gap.h"
#include "network_provisioning/scheme_ble.h"
#elif defined(CONFIG_BOBE_PROVISIONING_TRANSPORT_SOFTAP)
#include "esp_http_server.h"
#include "network_provisioning/scheme_softap.h"
#endif
#include "nvs.h"

#include <ctype.h>
#include <limits.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#if defined(CONFIG_BOBE_ENABLE_SECURE_PROVISIONING)
static const char *TAG = "bobe_provisioning";
static atomic_bool running;
static atomic_bool claim_configured;
static atomic_bool claim_existed_at_start;
static atomic_bool wifi_configured;
static atomic_bool stop_requested;
static atomic_bool provisioning_complete;
static esp_timer_handle_t window_timer;
static atomic_uintptr_t control_task_handle;
static bobe_provisioning_credentials_t credentials;
static network_prov_security2_params_t security_params;
static bobe_provisioning_finished_callback_t finished_callback;
static void *finished_callback_context;
#if defined(CONFIG_BOBE_PROVISIONING_TRANSPORT_SOFTAP)
static httpd_handle_t provisioning_httpd;
#endif

static void maybe_stop_provisioning(void);

#if defined(CONFIG_BOBE_PROVISIONING_TRANSPORT_SOFTAP)
static esp_err_t start_provisioning_httpd(void)
{
    if (provisioning_httpd != NULL) {
        return ESP_ERR_INVALID_STATE;
    }
    httpd_config_t config = HTTPD_DEFAULT_CONFIG();
    config.stack_size = 12288;
    config.task_priority = tskIDLE_PRIORITY + 5;
    config.lru_purge_enable = true;
    config.max_open_sockets = 1;
    const esp_err_t result = httpd_start(&provisioning_httpd, &config);
    if (result == ESP_OK) {
        network_prov_scheme_softap_set_httpd_handle(&provisioning_httpd);
    }
    return result;
}

static void stop_provisioning_httpd(void)
{
    network_prov_scheme_softap_set_httpd_handle(NULL);
    if (provisioning_httpd != NULL) {
        ESP_ERROR_CHECK_WITHOUT_ABORT(httpd_stop(provisioning_httpd));
        provisioning_httpd = NULL;
    }
}
#endif

static bool contains_nul_escape(const uint8_t *input, size_t input_size)
{
    for (size_t index = 0; index + 5U < input_size; ++index) {
        if (input[index] == '\\') {
            if (input[index + 1U] == 'u' &&
                input[index + 2U] == '0' && input[index + 3U] == '0' &&
                input[index + 4U] == '0' && input[index + 5U] == '0') {
                return true;
            }
            ++index;
        }
    }
    return false;
}

static bool claim_schema_is_valid(const cJSON *root)
{
    static const char *const fields[] = {
        "endpoint_uri",
        "trust_kid",
        "device_id",
        "controller_epoch",
        "server_ca_pem",
        "client_cert_pem",
        "client_key_pem",
    };
    bool seen[sizeof(fields) / sizeof(fields[0])] = {false};
    size_t count = 0;
    const cJSON *item = NULL;
    cJSON_ArrayForEach(item, root) {
        bool matched = false;
        for (size_t index = 0;
             index < sizeof(fields) / sizeof(fields[0]);
             ++index) {
            if (item->string != NULL &&
                strcmp(item->string, fields[index]) == 0) {
                if (seen[index]) {
                    return false;
                }
                seen[index] = true;
                matched = true;
                ++count;
                break;
            }
        }
        if (!matched) {
            return false;
        }
    }
    return count == sizeof(fields) / sizeof(fields[0]);
}

static bool json_string(
    const cJSON *root,
    const char *name,
    char *output,
    size_t output_size
)
{
    const cJSON *item = cJSON_GetObjectItemCaseSensitive(root, name);
    if (!cJSON_IsString(item) || item->valuestring == NULL) {
        return false;
    }
    const size_t length = strlen(item->valuestring);
    if (length == 0 || length >= output_size) {
        return false;
    }
    memcpy(output, item->valuestring, length + 1);
    return true;
}

static bool key_id_is_valid(const char *key_id)
{
    if (key_id == NULL || key_id[0] == '\0') {
        return false;
    }
    for (const char *cursor = key_id; *cursor != '\0'; ++cursor) {
        const unsigned char value = (unsigned char)*cursor;
        if (!(isalnum(value) || value == '-' || value == '_' ||
              value == '.' || value == ':')) {
            return false;
        }
    }
    return true;
}

static bool endpoint_is_valid(const char *uri)
{
    if (uri == NULL || strncmp(uri, "wss://", 6) != 0) {
        return false;
    }
    const char *authority = uri + 6;
    const char *path = strchr(authority, '/');
    const char *authority_end =
        path != NULL ? path : authority + strlen(authority);
    if (authority == authority_end || strchr(authority, '@') != NULL ||
        strchr(authority, '#') != NULL || strchr(authority, '?') != NULL ||
        strchr(authority, '\\') != NULL || strstr(uri, "..") != NULL) {
        return false;
    }
    for (const char *cursor = authority; *cursor != '\0'; ++cursor) {
        const unsigned char value = (unsigned char)*cursor;
        if (value <= 0x20 || value >= 0x7F) {
            return false;
        }
    }
    if (path != NULL && path[0] == '/' && path[1] == '/') {
        return false;
    }
    const char *port = NULL;
    if (*authority == '[') {
        const char *closing = memchr(
            authority,
            ']',
            (size_t)(authority_end - authority)
        );
        if (closing == NULL || closing == authority + 1) {
            return false;
        }
        if (closing + 1 < authority_end) {
            if (closing[1] != ':') {
                return false;
            }
            port = closing + 2;
        }
    } else {
        const char *colon = memchr(
            authority,
            ':',
            (size_t)(authority_end - authority)
        );
        if (colon == authority) {
            return false;
        }
        port = colon != NULL ? colon + 1 : NULL;
    }
    if (port != NULL) {
        unsigned long value = 0;
        if (port == authority_end) {
            return false;
        }
        for (const char *cursor = port; cursor < authority_end; ++cursor) {
            if (!isdigit((unsigned char)*cursor)) {
                return false;
            }
            value = value * 10UL + (unsigned long)(*cursor - '0');
            if (value > 65535UL) {
                return false;
            }
        }
        if (value == 0) {
            return false;
        }
    }
    return true;
}

static esp_err_t load_claim_state(bobe_claim_state_t *state)
{
    if (state == NULL) {
        return ESP_ERR_INVALID_ARG;
    }
    memset(state, 0, sizeof(*state));
    nvs_handle_t nvs;
    const esp_err_t open_result =
        nvs_open(BOBE_CLAIM_NAMESPACE, NVS_READONLY, &nvs);
    if (open_result == ESP_ERR_NVS_NOT_FOUND) {
        return ESP_OK;
    }
    if (open_result != ESP_OK) {
        return open_result;
    }
    uint8_t ready = 0;
    const esp_err_t ready_result =
        nvs_get_u8(nvs, BOBE_CLAIM_READY_KEY, &ready);
    nvs_close(nvs);
    if (ready_result == ESP_ERR_NVS_NOT_FOUND || ready != 1) {
        return ready_result == ESP_ERR_NVS_NOT_FOUND ? ESP_OK : ESP_ERR_INVALID_STATE;
    }
    state->claim_ready = true;
    uint8_t slot = 0;
    const esp_err_t slot_result = bobe_storage_load_active_slot(&slot);
    if (slot_result != ESP_OK || bobe_storage_slot_keys(slot) == NULL) {
        return ESP_ERR_INVALID_STATE;
    }
    state->active_slot_valid = true;
    state->active_slot = slot;
    const bobe_storage_slot_keys_t *const keys = bobe_storage_slot_keys(slot);
    if (nvs_open(BOBE_CLAIM_NAMESPACE, NVS_READONLY, &nvs) != ESP_OK) {
        return ESP_ERR_INVALID_STATE;
    }
    const esp_err_t epoch_result = nvs_get_u64(
        nvs,
        keys->controller_epoch,
        &state->active_epoch
    );
    nvs_close(nvs);
    return epoch_result;
}

static esp_err_t erase_keys(
    const char *namespace_name,
    const char *const *keys,
    size_t count
)
{
    nvs_handle_t nvs;
    const esp_err_t open_result =
        nvs_open(namespace_name, NVS_READWRITE, &nvs);
    if (open_result == ESP_ERR_NVS_NOT_FOUND) {
        return ESP_OK;
    }
    if (open_result != ESP_OK) {
        return open_result;
    }
    esp_err_t result = ESP_OK;
    for (size_t index = 0; index < count && result == ESP_OK; ++index) {
        result = nvs_erase_key(nvs, keys[index]);
        if (result == ESP_ERR_NVS_NOT_FOUND) {
            result = ESP_OK;
        }
    }
    if (result == ESP_OK) {
        result = nvs_commit(nvs);
    }
    nvs_close(nvs);
    return result;
}

static esp_err_t retire_claim_material(const bobe_claim_plan_t *plan)
{
    if (plan == NULL) {
        return ESP_ERR_INVALID_ARG;
    }
    if (plan->retire_slot) {
        const bobe_storage_slot_keys_t *const retired =
            bobe_storage_slot_keys(plan->retired_slot);
        if (retired == NULL) {
            return ESP_ERR_INVALID_STATE;
        }
        const char *const claim_keys[] = {
            retired->endpoint,
            retired->trust_key_id,
            retired->controller_epoch,
        };
        ESP_RETURN_ON_ERROR(
            erase_keys(
                BOBE_CLAIM_NAMESPACE,
                claim_keys,
                sizeof(claim_keys) / sizeof(claim_keys[0])
            ),
            TAG,
            "retire previous claim slot"
        );
        const char *const tls_keys[] = {
            retired->server_ca,
            retired->client_cert,
            retired->client_key,
        };
        ESP_RETURN_ON_ERROR(
            erase_keys(
                BOBE_TLS_NAMESPACE,
                tls_keys,
                sizeof(tls_keys) / sizeof(tls_keys[0])
            ),
            TAG,
            "retire previous TLS slot"
        );
    }
    return ESP_OK;
}

static esp_err_t retire_inactive_claim_slot(void)
{
    bobe_claim_state_t state;
    ESP_RETURN_ON_ERROR(
        load_claim_state(&state),
        TAG,
        "load claim state for retirement"
    );
    if (!state.claim_ready) {
        return ESP_OK;
    }
    const bobe_claim_plan_t plan = {
        .retire_slot = true,
        .retired_slot = (uint8_t)(state.active_slot ^ 1U),
    };
    return retire_claim_material(&plan);
}

static esp_err_t save_claim_slot(
    const bobe_storage_slot_keys_t *keys,
    const char *endpoint,
    const char *key_id,
    uint64_t controller_epoch
)
{
    if (keys == NULL) {
        return ESP_ERR_INVALID_ARG;
    }
    nvs_handle_t nvs;
    ESP_RETURN_ON_ERROR(
        nvs_open(BOBE_CLAIM_NAMESPACE, NVS_READWRITE, &nvs),
        TAG,
        "open claim storage"
    );
    esp_err_t result = nvs_set_str(nvs, keys->endpoint, endpoint);
    if (result == ESP_OK) {
        result = nvs_set_str(nvs, keys->trust_key_id, key_id);
    }
    if (result == ESP_OK) {
        result = nvs_set_u64(
            nvs,
            keys->controller_epoch,
            controller_epoch
        );
    }
    if (result == ESP_OK) {
        result = nvs_commit(nvs);
    }
    nvs_close(nvs);
    return result;
}

static esp_err_t save_tls_credentials(
    const bobe_storage_slot_keys_t *keys,
    const char *server_ca,
    const char *client_cert,
    const char *client_key
)
{
    if (keys == NULL) {
        return ESP_ERR_INVALID_ARG;
    }
    nvs_handle_t nvs;
    ESP_RETURN_ON_ERROR(
        nvs_open(BOBE_TLS_NAMESPACE, NVS_READWRITE, &nvs),
        TAG,
        "open TLS storage"
    );
    esp_err_t result = nvs_set_str(nvs, keys->server_ca, server_ca);
    if (result == ESP_OK) {
        result = nvs_set_str(nvs, keys->client_cert, client_cert);
    }
    if (result == ESP_OK) {
        result = nvs_set_str(nvs, keys->client_key, client_key);
    }
    if (result == ESP_OK) {
        result = nvs_commit(nvs);
    }
    nvs_close(nvs);
    return result;
}

static esp_err_t activate_claim_slot(uint8_t slot)
{
    if (bobe_storage_slot_keys(slot) == NULL) {
        return ESP_ERR_INVALID_ARG;
    }
    nvs_handle_t nvs;
    ESP_RETURN_ON_ERROR(
        nvs_open(BOBE_CLAIM_NAMESPACE, NVS_READWRITE, &nvs),
        TAG,
        "open claim activation storage"
    );
    esp_err_t result =
        nvs_set_u8(nvs, BOBE_CLAIM_ACTIVE_SLOT_KEY, slot);
    if (result == ESP_OK) {
        result = nvs_set_u8(nvs, BOBE_CLAIM_READY_KEY, 1);
    }
    if (result == ESP_OK) {
        result = nvs_commit(nvs);
    }
    nvs_close(nvs);
    return result;
}

static const char *required_pem(
    const cJSON *root,
    const char *name,
    const char *header,
    const char *footer,
    size_t maximum_length
)
{
    const cJSON *item = cJSON_GetObjectItemCaseSensitive(root, name);
    if (!cJSON_IsString(item) || item->valuestring == NULL) {
        return NULL;
    }
    const size_t length = strlen(item->valuestring);
    size_t content_length = length;
    while (content_length > 0 &&
           (item->valuestring[content_length - 1] == '\r' ||
            item->valuestring[content_length - 1] == '\n')) {
        --content_length;
    }
    const size_t footer_length = strlen(footer);
    return content_length > strlen(header) + footer_length &&
                   length < maximum_length &&
                   strncmp(item->valuestring, header, strlen(header)) == 0 &&
                   content_length >= footer_length &&
                   memcmp(
                       item->valuestring + content_length - footer_length,
                       footer,
                       footer_length
                   ) == 0
               ? item->valuestring
               : NULL;
}

static const char *required_private_key(
    const cJSON *root,
    size_t maximum_length
)
{
    static const struct {
        const char *header;
        const char *footer;
    } formats[] = {
        {
            "-----BEGIN PRIVATE KEY-----",
            "-----END PRIVATE KEY-----",
        },
        {
            "-----BEGIN EC PRIVATE KEY-----",
            "-----END EC PRIVATE KEY-----",
        },
        {
            "-----BEGIN RSA PRIVATE KEY-----",
            "-----END RSA PRIVATE KEY-----",
        },
    };
    for (size_t index = 0;
         index < sizeof(formats) / sizeof(formats[0]);
         ++index) {
        const char *value = required_pem(
            root,
            "client_key_pem",
            formats[index].header,
            formats[index].footer,
            maximum_length
        );
        if (value != NULL) {
            return value;
        }
    }
    return NULL;
}

static bool tls_credentials_are_valid(
    const char *server_ca_pem,
    const char *client_cert_pem,
    const char *client_key_pem
)
{
    mbedtls_x509_crt server_ca;
    mbedtls_x509_crt client_cert;
    mbedtls_pk_context client_key;
    mbedtls_x509_crt_init(&server_ca);
    mbedtls_x509_crt_init(&client_cert);
    mbedtls_pk_init(&client_key);

    const int ca_result = mbedtls_x509_crt_parse(
        &server_ca,
        (const unsigned char *)server_ca_pem,
        strlen(server_ca_pem) + 1
    );
    const int cert_result = ca_result == 0
                                ? mbedtls_x509_crt_parse(
                                      &client_cert,
                                      (const unsigned char *)client_cert_pem,
                                      strlen(client_cert_pem) + 1
                                  )
                                : ca_result;
    const int key_result = cert_result == 0
                               ? mbedtls_pk_parse_key(
                                     &client_key,
                                     (const unsigned char *)client_key_pem,
                                     strlen(client_key_pem) + 1,
                                     NULL,
                                     0
                                 )
                               : cert_result;
    const int pair_result = key_result == 0
                                ? mbedtls_pk_check_pair(
                                      &client_cert.pk,
                                      &client_key
                                  )
                                : key_result;
    const bool valid =
        ca_result == 0 && cert_result == 0 && key_result == 0 &&
        pair_result == 0;
    if (!valid) {
        ESP_LOGW(
            TAG,
            "claim rejected invalid TLS credentials (%d/%d/%d/%d)",
            ca_result,
            cert_result,
            key_result,
            pair_result
        );
    }

    mbedtls_pk_free(&client_key);
    mbedtls_x509_crt_free(&client_cert);
    mbedtls_x509_crt_free(&server_ca);
    return valid;
}

static esp_err_t claim_handler(
    uint32_t session_id,
    const uint8_t *input,
    ssize_t input_length,
    uint8_t **output,
    ssize_t *output_length,
    void *context
)
{
    (void)session_id;
    (void)context;
    if (atomic_load(&claim_existed_at_start)) {
        atomic_store(&claim_configured, false);
    }
    if (input == NULL || input_length <= 0 || input_length > 16384 ||
        output == NULL || output_length == NULL ||
        memchr(input, '\0', (size_t)input_length) != NULL ||
        contains_nul_escape(input, (size_t)input_length)) {
        ESP_LOGW(
            TAG,
            "claim rejected before parsing: bytes=%d",
            (int)input_length
        );
        return ESP_ERR_INVALID_ARG;
    }
    ESP_LOGI(
        TAG,
        "claim request received: bytes=%d stack_free_words=%u",
        (int)input_length,
        (unsigned int)uxTaskGetStackHighWaterMark(NULL)
    );

    const char *parse_end = NULL;
    cJSON *root = cJSON_ParseWithLengthOpts(
        (const char *)input,
        (size_t)input_length,
        &parse_end,
        false
    );
    const char *input_end = (const char *)input + input_length;
    while (parse_end != NULL && parse_end < input_end &&
           isspace((unsigned char)*parse_end)) {
        ++parse_end;
    }
    char endpoint[256] = {0};
    char key_id[64] = {0};
    char claimed_device_id[64] = {0};
    char local_device_id[64] = {0};
    const cJSON *epoch =
        root != NULL
            ? cJSON_GetObjectItemCaseSensitive(root, "controller_epoch")
            : NULL;
    const char *server_ca = root != NULL
                                ? required_pem(
                                      root,
                                      "server_ca_pem",
                                      "-----BEGIN CERTIFICATE-----",
                                      "-----END CERTIFICATE-----",
                                      BOBE_PEM_STORAGE_MAX_BYTES
                                  )
                                : NULL;
    const char *client_cert = root != NULL
                                  ? required_pem(
                                        root,
                                        "client_cert_pem",
                                        "-----BEGIN CERTIFICATE-----",
                                        "-----END CERTIFICATE-----",
                                        BOBE_PEM_STORAGE_MAX_BYTES
                                    )
                                  : NULL;
    const char *client_key = root != NULL
                                 ? required_private_key(
                                       root,
                                       BOBE_PEM_STORAGE_MAX_BYTES
                                   )
                                 : NULL;
    const bool parse_valid =
        root != NULL && parse_end == input_end && cJSON_IsObject(root);
    const bool schema_valid = parse_valid && claim_schema_is_valid(root);
    const bool endpoint_present =
        schema_valid &&
        json_string(root, "endpoint_uri", endpoint, sizeof(endpoint));
    const bool key_id_valid =
        schema_valid &&
        json_string(root, "trust_kid", key_id, sizeof(key_id)) &&
        key_id_is_valid(key_id);
    const bool claimed_id_present =
        schema_valid &&
        json_string(
            root,
            "device_id",
            claimed_device_id,
            sizeof(claimed_device_id)
        );
    const bool local_id_present =
        bobe_identity_device_id(local_device_id, sizeof(local_device_id)) == ESP_OK;
    const bool identity_matches =
        claimed_id_present && local_id_present &&
        strcmp(claimed_device_id, local_device_id) == 0;
    const bool epoch_valid =
        cJSON_IsNumber(epoch) && epoch->valuedouble >= 1 &&
        epoch->valuedouble <= 9007199254740991.0 &&
        (double)(uint64_t)epoch->valuedouble == epoch->valuedouble;
    const bool endpoint_valid =
        endpoint_present && endpoint_is_valid(endpoint);
    const bool pem_valid =
        server_ca != NULL && client_cert != NULL && client_key != NULL;
    const bool tls_valid =
        pem_valid &&
        tls_credentials_are_valid(server_ca, client_cert, client_key);
    if (!schema_valid || !key_id_valid || !identity_matches ||
        !epoch_valid || !endpoint_valid || !tls_valid) {
        ESP_LOGW(
            TAG,
            "claim rejected: parse=%d schema=%d endpoint=%d key=%d "
            "identity=%d epoch=%d pem=%d tls=%d",
            parse_valid,
            schema_valid,
            endpoint_valid,
            key_id_valid,
            identity_matches,
            epoch_valid,
            pem_valid,
            tls_valid
        );
        cJSON_Delete(root);
        return ESP_ERR_INVALID_ARG;
    }
    const uint64_t controller_epoch = (uint64_t)epoch->valuedouble;
    bobe_claim_state_t claim_state;
    esp_err_t result = load_claim_state(&claim_state);
    bobe_claim_plan_t plan;
    if (result == ESP_OK) {
        const bobe_claim_plan_result_t plan_result =
            bobe_claim_plan_rotation(
                &claim_state,
                controller_epoch,
                &plan
            );
        if (plan_result != BOBE_CLAIM_PLAN_OK) {
            ESP_LOGW(
                TAG,
                "claim rotation rejected: %s",
                bobe_claim_plan_result_name(plan_result)
            );
            result = plan_result == BOBE_CLAIM_PLAN_STALE_EPOCH
                         ? ESP_ERR_INVALID_VERSION
                         : ESP_ERR_INVALID_STATE;
        }
    }
    if (result != ESP_OK) {
        cJSON_Delete(root);
        return result;
    }

    static const char response[] =
        "{\"status\":\"ok\",\"reboot_required\":false}";
    const size_t response_size = strlen(response) + 1;
    *output = malloc(response_size);
    if (*output == NULL) {
        cJSON_Delete(root);
        return ESP_ERR_NO_MEM;
    }
    *output_length = (ssize_t)(response_size - 1);

    const uint8_t target_slot = plan.target_slot;
    const bobe_storage_slot_keys_t *target_keys =
        bobe_storage_slot_keys(target_slot);
    if (result == ESP_OK) {
        result = save_tls_credentials(
            target_keys,
            server_ca,
            client_cert,
            client_key
        );
    }
    if (result == ESP_OK) {
        result = save_claim_slot(
            target_keys,
            endpoint,
            key_id,
            controller_epoch
        );
    }
    if (result == ESP_OK) {
        result = activate_claim_slot(target_slot);
    }
    if (result == ESP_OK) {
        result = retire_claim_material(&plan);
    }
    cJSON_Delete(root);
    if (result != ESP_OK) {
        free(*output);
        *output = NULL;
        *output_length = 0;
        return result;
    }
    atomic_store(&claim_configured, true);
    ESP_LOGI(TAG, "BodyLink claim stored successfully");
    memcpy(*output, response, response_size);
    maybe_stop_provisioning();
    return ESP_OK;
}

static void request_provisioning_stop(bool completed)
{
    if (completed) {
        atomic_store(&provisioning_complete, true);
    }

    const TaskHandle_t task = (TaskHandle_t)atomic_load(
        &control_task_handle
    );
    if (!atomic_exchange(&stop_requested, true) && task != NULL) {
        xTaskNotifyGive(task);
    }
}

static void maybe_stop_provisioning(void)
{
    if (atomic_load(&claim_configured) &&
        atomic_load(&wifi_configured)) {
        request_provisioning_stop(true);
    }
}

static void provisioning_control_task(void *unused)
{
    (void)unused;
    ulTaskNotifyTake(pdTRUE, portMAX_DELAY);
    if (atomic_load(&provisioning_complete)) {
        vTaskDelay(pdMS_TO_TICKS(1500));
    }
    if (atomic_load(&running)) {
        network_prov_mgr_stop_provisioning();
    }
    atomic_store(&control_task_handle, (uintptr_t)NULL);
    vTaskDelete(NULL);
}

static void provisioning_event(
    void *context,
    esp_event_base_t event_base,
    int32_t event_id,
    void *event_data
)
{
    (void)context;
    (void)event_data;
    if (event_base != NETWORK_PROV_EVENT) {
        return;
    }
    if (event_id == NETWORK_PROV_WIFI_CRED_SUCCESS) {
        atomic_store(&wifi_configured, true);
        maybe_stop_provisioning();
    } else if (event_id == NETWORK_PROV_END) {
        (void)network_prov_mgr_deinit();
        (void)esp_event_handler_unregister(
            NETWORK_PROV_EVENT,
            ESP_EVENT_ANY_ID,
            provisioning_event
        );
#if defined(CONFIG_BOBE_PROVISIONING_TRANSPORT_SOFTAP)
        stop_provisioning_httpd();
#endif
        atomic_store(&running, false);
        if (window_timer != NULL && esp_timer_is_active(window_timer)) {
            (void)esp_timer_stop(window_timer);
        }
        if (finished_callback != NULL) {
            finished_callback(
                atomic_load(&provisioning_complete)
                    ? BOBE_PROVISIONING_COMPLETED
                    : BOBE_PROVISIONING_ABORTED,
                finished_callback_context
            );
        }
    }
}

static void window_expired(void *context)
{
    (void)context;
    if (atomic_load(&running)) {
        ESP_LOGW(TAG, "provisioning window expired");
        request_provisioning_stop(false);
    }
}

#if defined(CONFIG_BOBE_PROVISIONING_TRANSPORT_BLE)
static int tx_power_dbm(esp_power_level_t level)
{
    static const int levels[] = {
        -24, -21, -18, -15, -12, -9, -6, -3,
        0, 3, 6, 9, 12, 15, 18, 20,
    };
    const unsigned int index = (unsigned int)level;
    return index < sizeof(levels) / sizeof(levels[0]) ? levels[index] :
                                                           INT_MIN;
}
#endif
#endif

esp_err_t bobe_provisioning_stop(void)
{
#if !defined(CONFIG_BOBE_ENABLE_SECURE_PROVISIONING)
    return ESP_ERR_NOT_SUPPORTED;
#else
    if (!atomic_load(&running)) {
        return ESP_OK;
    }
    request_provisioning_stop(false);
    return ESP_OK;
#endif
}

esp_err_t bobe_provisioning_start(
    bobe_provisioning_finished_callback_t callback,
    void *context
)
{
#if !defined(CONFIG_BOBE_ENABLE_SECURE_PROVISIONING)
    (void)callback;
    (void)context;
    return ESP_ERR_NOT_SUPPORTED;
#else
    if (atomic_exchange(&running, true)) {
        return ESP_ERR_INVALID_STATE;
    }
    finished_callback = callback;
    finished_callback_context = context;
    const bool existing_claim = bobe_provisioning_has_complete_claim();
    if (existing_claim) {
        const esp_err_t retirement_result = retire_inactive_claim_slot();
        if (retirement_result != ESP_OK) {
            atomic_store(&running, false);
            return retirement_result;
        }
    }
    atomic_store(&claim_configured, existing_claim);
    atomic_store(&claim_existed_at_start, existing_claim);
    atomic_store(&wifi_configured, false);
    atomic_store(&stop_requested, false);
    atomic_store(&provisioning_complete, false);

    esp_err_t ret =
        bobe_provisioning_credentials_get(&credentials);
    bool credentials_invalid =
        ret != ESP_OK || credentials.username == NULL ||
        credentials.salt == NULL || credentials.salt_length != 16 ||
        credentials.verifier == NULL ||
        credentials.verifier_length != 384;
#if defined(CONFIG_BOBE_PROVISIONING_TRANSPORT_SOFTAP)
    const size_t softap_password_length =
        credentials.softap_password != NULL
            ? strnlen(credentials.softap_password, 65)
            : 0;
    credentials_invalid =
        credentials_invalid || softap_password_length < 16 ||
        softap_password_length > 63;
#endif
    if (credentials_invalid) {
        atomic_store(&running, false);
        ESP_LOGE(TAG, "Security 2 credentials are unavailable");
        return ESP_ERR_NOT_FOUND;
    }

    const network_prov_mgr_config_t manager_config = {
#if defined(CONFIG_BOBE_PROVISIONING_TRANSPORT_BLE)
        .scheme = network_prov_scheme_ble,
        .scheme_event_handler =
            NETWORK_PROV_SCHEME_BLE_EVENT_HANDLER_FREE_BT,
#else
        .scheme = network_prov_scheme_softap,
        .scheme_event_handler = NETWORK_PROV_EVENT_HANDLER_NONE,
#endif
        .network_prov_wifi_conn_cfg = {
            .wifi_conn_attempts = 5,
        },
    };
    ESP_GOTO_ON_ERROR(
        network_prov_mgr_init(manager_config),
        fail,
        TAG,
        "initialize provisioning"
    );
    if (!atomic_load(&claim_configured)) {
        bool wifi_was_provisioned = false;
        ESP_GOTO_ON_ERROR(
            network_prov_mgr_is_wifi_provisioned(&wifi_was_provisioned),
            deinit,
            TAG,
            "inspect existing Wi-Fi credentials"
        );
        atomic_store(&wifi_configured, wifi_was_provisioned);
    }
    ESP_GOTO_ON_ERROR(
        esp_event_handler_register(
            NETWORK_PROV_EVENT,
            ESP_EVENT_ANY_ID,
            provisioning_event,
            NULL
        ),
        deinit,
        TAG,
        "register provisioning events"
    );

    const char *capabilities[] = {
        "wifi_scan",
        "bobe_claim_v1",
    };
    ESP_GOTO_ON_ERROR(
        network_prov_mgr_set_app_info(
            "bobe",
            "1",
            capabilities,
            sizeof(capabilities) / sizeof(capabilities[0])
        ),
        unregister,
        TAG,
        "set provisioning capabilities"
    );
    ESP_GOTO_ON_ERROR(
        network_prov_mgr_endpoint_create("bobe-claim"),
        unregister,
        TAG,
        "create claim endpoint"
    );
    ESP_GOTO_ON_ERROR(
        network_prov_mgr_disable_auto_stop(1500),
        unregister,
        TAG,
        "configure provisioning stop"
    );
#if defined(CONFIG_BOBE_PROVISIONING_TRANSPORT_SOFTAP)
    ESP_GOTO_ON_ERROR(
        start_provisioning_httpd(),
        unregister,
        TAG,
        "start provisioning HTTP server"
    );
#endif

    uint8_t mac[6];
    ESP_GOTO_ON_ERROR(
        esp_read_mac(mac, ESP_MAC_WIFI_STA),
        unregister,
        TAG,
        "read provisioning identity"
    );
    char service_name[16];
    snprintf(
        service_name,
        sizeof(service_name),
        "BOBE_%02X%02X%02X",
        mac[3],
        mac[4],
        mac[5]
    );
    security_params = (network_prov_security2_params_t) {
        .salt = (const char *)credentials.salt,
        .salt_len = (uint16_t)credentials.salt_length,
        .verifier = (const char *)credentials.verifier,
        .verifier_len = (uint16_t)credentials.verifier_length,
    };
#if defined(CONFIG_BOBE_PROVISIONING_TRANSPORT_SOFTAP)
    const char *service_key = credentials.softap_password;
#else
    const char *service_key = NULL;
#endif
    ESP_GOTO_ON_ERROR(
        network_prov_mgr_start_provisioning(
            NETWORK_PROV_SECURITY_2,
            &security_params,
            service_name,
            service_key
        ),
        unregister,
        TAG,
        "start Security 2 provisioning"
    );
    ESP_GOTO_ON_ERROR(
        network_prov_mgr_endpoint_register(
            "bobe-claim",
            claim_handler,
            NULL
        ),
        stop,
        TAG,
        "register claim endpoint"
    );
    TaskHandle_t control_task = NULL;
    if (xTaskCreatePinnedToCore(
            provisioning_control_task,
            "bobe_prov_ctl",
            3072,
            NULL,
            BOBE_TASK_PRIORITY_BACKGROUND,
            &control_task,
            BOBE_NETWORK_CORE
        ) != pdPASS) {
        ret = ESP_ERR_NO_MEM;
        goto stop;
    }
    atomic_store(&control_task_handle, (uintptr_t)control_task);
    if (atomic_load(&stop_requested)) {
        xTaskNotifyGive(control_task);
    }

    if (window_timer == NULL) {
        const esp_timer_create_args_t timer_config = {
            .callback = window_expired,
            .name = "bobe_pairing",
        };
        ESP_GOTO_ON_ERROR(
            esp_timer_create(&timer_config, &window_timer),
            stop,
            TAG,
            "create provisioning timer"
        );
    }
    ESP_GOTO_ON_ERROR(
        esp_timer_start_once(
            window_timer,
            (uint64_t)CONFIG_BOBE_PROVISIONING_WINDOW_SECONDS *
                UINT64_C(1000000)
        ),
        stop,
        TAG,
        "start provisioning timer"
    );
#if defined(CONFIG_BOBE_PROVISIONING_TRANSPORT_BLE)
    const esp_power_level_t advertising_power =
        esp_ble_tx_power_get_enhanced(ESP_BLE_ENHANCED_PWR_TYPE_ADV, 0);
    ESP_LOGI(
        TAG,
        "Security 2 BLE provisioning started as %s; controller=%d "
        "advertising=%d tx_power_index=%u tx_power_dbm=%d",
        service_name,
        (int)esp_bt_controller_get_status(),
        ble_gap_adv_active(),
        (unsigned int)advertising_power,
        tx_power_dbm(advertising_power)
    );
#else
    ESP_LOGI(
        TAG,
        "Security 2 SoftAP provisioning started as %s at 192.168.4.1; "
        "WPA2 protected",
        service_name
    );
#endif
    return ESP_OK;

stop:
    network_prov_mgr_stop_provisioning();
    control_task = (TaskHandle_t)atomic_exchange(
        &control_task_handle,
        (uintptr_t)NULL
    );
    if (control_task != NULL) {
        vTaskDelete(control_task);
    }
unregister:
    (void)esp_event_handler_unregister(
        NETWORK_PROV_EVENT,
        ESP_EVENT_ANY_ID,
        provisioning_event
    );
deinit:
    (void)network_prov_mgr_deinit();
fail:
#if defined(CONFIG_BOBE_PROVISIONING_TRANSPORT_SOFTAP)
    stop_provisioning_httpd();
#endif
    atomic_store(&running, false);
    return ret;
#endif
}

bool bobe_provisioning_is_running(void)
{
#if defined(CONFIG_BOBE_ENABLE_SECURE_PROVISIONING)
    return atomic_load(&running);
#else
    return false;
#endif
}

bool bobe_provisioning_has_complete_claim(void)
{
    nvs_handle_t nvs;
    if (nvs_open(BOBE_CLAIM_NAMESPACE, NVS_READONLY, &nvs) != ESP_OK) {
        return false;
    }
    uint8_t ready = 0;
    const esp_err_t result =
        nvs_get_u8(nvs, BOBE_CLAIM_READY_KEY, &ready);
    nvs_close(nvs);
    if (result != ESP_OK || ready != 1) {
        return false;
    }
    uint8_t active_slot = 0;
    const esp_err_t slot_result =
        bobe_storage_load_active_slot(&active_slot);
    return slot_result == ESP_OK &&
           bobe_storage_slot_keys(active_slot) != NULL;
}
