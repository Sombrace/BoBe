#include "bobe_device_config.h"
#include "bobe_connectivity.h"

#include "bobe_protocol.h"
#include "bobe_provisioning.h"
#include "bobe_storage.h"
#include "bobe_task_config.h"
#include "bobe_wifi_recovery.h"
#include "esp_check.h"
#include "esp_event.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "esp_mac.h"
#include "esp_netif_sntp.h"
#include "esp_random.h"
#include "esp_timer.h"
#include "esp_wifi.h"
#include "freertos/FreeRTOS.h"
#include "freertos/event_groups.h"
#include "freertos/semphr.h"
#include "freertos/task.h"
#include "mdns.h"
#include "nvs.h"

#include <ctype.h>
#include <stdatomic.h>
#include <stdio.h>
#include <string.h>
#include <strings.h>
#include <sys/time.h>
#include <time.h>

#define BOBE_RECONNECT_MIN_MS 500U
#define BOBE_RECONNECT_MAX_MS 30000U
#define BOBE_PROVISIONING_MIN_DMA_BLOCK_BYTES 40960U
#define BOBE_PROVISIONING_START_TIMEOUT_MS 5000U
#define BOBE_TIME_VALID_AFTER INT64_C(1704067200)
#define BOBE_TIME_VALID_BEFORE INT64_C(2082758400)
#define BOBE_CLOCK_NAMESPACE "bobe_clock"
#define BOBE_CLOCK_FLOOR_KEY "unix_floor"
#define BOBE_MDNS_SERVICE "_bobe-body-hub"
#define BOBE_MDNS_PROTO "_tcp"
#define BOBE_WIFI_CONFIG_EVENT_CONNECTED BIT0
#define BOBE_WIFI_CONFIG_EVENT_AUTH_FAILED BIT1
#define BOBE_WIFI_CONFIG_EVENT_AP_NOT_FOUND BIT2
#define BOBE_WIFI_CONFIG_EVENT_QUIESCED BIT3

static const char *TAG = "bobe_connectivity";

static bobe_network_callback_t network_callback;
static void *network_callback_context;
static esp_timer_handle_t reconnect_timer;
static atomic_bool online;
static atomic_bool ip_up;
static atomic_bool time_sync_running;
static atomic_bool provisioning_start_pending;
static atomic_int provisioning_start_result;
static atomic_uint connection_generation;
static atomic_uint reconnect_attempt;
static bobe_wifi_recovery_t wifi_recovery;
static atomic_bool wifi_configuration_active;
static atomic_bool wifi_configuration_trial_started;
static EventGroupHandle_t wifi_configuration_events;
static SemaphoreHandle_t wifi_configuration_mutex;

static void start_time_sync_task(void);

static bool station_has_credentials(void)
{
    wifi_config_t config;
    memset(&config, 0, sizeof(config));
    return esp_wifi_get_config(WIFI_IF_STA, &config) == ESP_OK &&
           config.sta.ssid[0] != '\0';
}
static esp_err_t start_provisioning_async(void);
static void schedule_reconnect(void);

static void publish_state(bobe_network_state_t state)
{
    if (network_callback != NULL) {
        const bobe_network_event_t event = {
            .state = state,
            .generation = atomic_load(&connection_generation),
        };
        network_callback(&event, network_callback_context);
    }
}

static void restore_clock_floor(void)
{
    nvs_handle_t nvs;
    if (nvs_open(BOBE_CLOCK_NAMESPACE, NVS_READONLY, &nvs) != ESP_OK) {
        return;
    }
    uint64_t floor = 0;
    const esp_err_t result =
        nvs_get_u64(nvs, BOBE_CLOCK_FLOOR_KEY, &floor);
    nvs_close(nvs);
    if (result != ESP_OK || floor < (uint64_t)BOBE_TIME_VALID_AFTER ||
        floor >= (uint64_t)BOBE_TIME_VALID_BEFORE ||
        floor > (uint64_t)INT64_MAX || time(NULL) >= (time_t)floor) {
        return;
    }
    const struct timeval value = {
        .tv_sec = (time_t)floor,
        .tv_usec = 0,
    };
    if (settimeofday(&value, NULL) != 0) {
        ESP_LOGW(TAG, "could not restore the saved clock floor");
    }
}

static void save_authenticated_clock_floor(void)
{
    const time_t now = time(NULL);
    if (now < (time_t)BOBE_TIME_VALID_AFTER ||
        now >= (time_t)BOBE_TIME_VALID_BEFORE) {
        return;
    }
    nvs_handle_t nvs;
    if (nvs_open(BOBE_CLOCK_NAMESPACE, NVS_READWRITE, &nvs) != ESP_OK) {
        return;
    }
    uint64_t stored = 0;
    const esp_err_t get_result =
        nvs_get_u64(nvs, BOBE_CLOCK_FLOOR_KEY, &stored);
    if (get_result == ESP_OK &&
        (stored < (uint64_t)BOBE_TIME_VALID_AFTER ||
         stored >= (uint64_t)BOBE_TIME_VALID_BEFORE)) {
        stored = 0;
    }
    if ((get_result == ESP_ERR_NVS_NOT_FOUND ||
         (get_result == ESP_OK && (uint64_t)now > stored)) &&
        nvs_set_u64(nvs, BOBE_CLOCK_FLOOR_KEY, (uint64_t)now) == ESP_OK) {
        (void)nvs_commit(nvs);
    }
    nvs_close(nvs);
}

static void publish_local_connectivity(void)
{
    if (!atomic_load(&ip_up)) {
        return;
    }
    atomic_store(&online, true);
    atomic_store(&reconnect_attempt, 0);
    bobe_wifi_recovery_reset(&wifi_recovery);
    publish_state(
        bobe_provisioning_has_complete_claim()
            ? BOBE_NETWORK_CONNECTED
            : BOBE_NETWORK_UNPROVISIONED
    );
}

static uint32_t reconnect_delay_ms(void)
{
    const uint32_t attempt = atomic_load(&reconnect_attempt);
    const uint32_t shift = attempt < 6U ? attempt : 6U;
    uint32_t ceiling = BOBE_RECONNECT_MIN_MS << shift;
    if (ceiling > BOBE_RECONNECT_MAX_MS) {
        ceiling = BOBE_RECONNECT_MAX_MS;
    }
    const uint32_t span = ceiling - BOBE_RECONNECT_MIN_MS + 1U;
    return BOBE_RECONNECT_MIN_MS + esp_random() % span;
}

static void reconnect_timer_callback(void *unused)
{
    (void)unused;
    if (atomic_load(&wifi_configuration_active) ||
        bobe_provisioning_is_running() ||
        atomic_load(&provisioning_start_pending)) {
        return;
    }
    if (!station_has_credentials()) {
        ESP_LOGI(TAG, "reconnect deferred until station credentials exist");
        publish_state(BOBE_NETWORK_UNPROVISIONED);
        (void)start_provisioning_async();
        return;
    }
    publish_state(BOBE_NETWORK_CONNECTING);
    const esp_err_t result = esp_wifi_connect();
    if (result != ESP_OK) {
        ESP_LOGW(
            TAG,
            "Wi-Fi reconnect start failed: %s",
            esp_err_to_name(result)
        );
        schedule_reconnect();
    }
}

static void schedule_reconnect(void)
{
    if (atomic_load(&wifi_configuration_active) ||
        bobe_provisioning_is_running() ||
        atomic_load(&provisioning_start_pending)) {
        return;
    }
    const uint64_t delay_us =
        (uint64_t)reconnect_delay_ms() * UINT64_C(1000);
    atomic_fetch_add(&reconnect_attempt, 1U);
    if (esp_timer_is_active(reconnect_timer)) {
        ESP_ERROR_CHECK_WITHOUT_ABORT(esp_timer_stop(reconnect_timer));
    }
    ESP_ERROR_CHECK_WITHOUT_ABORT(
        esp_timer_start_once(reconnect_timer, delay_us)
    );
}

static void time_sync_task(void *context)
{
    const unsigned int generation = (unsigned int)(uintptr_t)context;
    esp_sntp_config_t config = ESP_NETIF_SNTP_DEFAULT_CONFIG_MULTIPLE(
        2,
        ESP_SNTP_SERVER_LIST("pool.ntp.org", "time.cloudflare.com")
    );
#ifdef CONFIG_LWIP_DHCP_GET_NTP_SRV
    config.server_from_dhcp = true;
    config.renew_servers_after_new_IP = true;
    config.index_of_first_server = 2;
#endif

    while (atomic_load(&ip_up) &&
           atomic_load(&connection_generation) == generation) {
        esp_err_t result = esp_netif_sntp_init(&config);
        if (result == ESP_OK) {
            result = esp_netif_sntp_sync_wait(pdMS_TO_TICKS(10000));
            esp_netif_sntp_deinit();
        }
        if (result == ESP_OK) {
            break;
        }
        ESP_LOGW(TAG, "SNTP synchronization timed out");
        vTaskDelay(pdMS_TO_TICKS(5000));
    }

    atomic_store(&time_sync_running, false);
    if (atomic_load(&ip_up) &&
        atomic_load(&connection_generation) != generation) {
        start_time_sync_task();
    }
    vTaskDelete(NULL);
}

static void start_time_sync_task(void)
{
    bool expected = false;
    if (!atomic_compare_exchange_strong(
            &time_sync_running,
            &expected,
            true
        )) {
        return;
    }
    const void *context = (void *)(uintptr_t)atomic_load(
        &connection_generation
    );
    if (xTaskCreatePinnedToCore(
            time_sync_task,
            "bobe_time_sync",
            4096,
            (void *)context,
            BOBE_TASK_PRIORITY_BACKGROUND,
            NULL,
            BOBE_NETWORK_CORE
        ) != pdPASS) {
        atomic_store(&time_sync_running, false);
        ESP_LOGE(TAG, "unable to create time synchronization task");
    }
}

static void provisioning_finished(
    bobe_provisioning_outcome_t outcome,
    void *unused
)
{
    (void)unused;
    publish_state(
        outcome == BOBE_PROVISIONING_COMPLETED
            ? BOBE_NETWORK_PROVISIONING_COMPLETE
            : BOBE_NETWORK_PROVISIONING_ABORTED
    );
    if (outcome == BOBE_PROVISIONING_ABORTED &&
        !atomic_load(&ip_up) &&
        station_has_credentials()) {
        schedule_reconnect();
    }
}

static void provisioning_start_task(void *unused)
{
    (void)unused;
    wifi_config_t connected_station_config;
    memset(
        &connected_station_config,
        0,
        sizeof(connected_station_config)
    );
    const bool preserve_connected_station =
        atomic_load(&ip_up) &&
        esp_wifi_get_config(
            WIFI_IF_STA,
            &connected_station_config
        ) == ESP_OK &&
        connected_station_config.sta.ssid[0] != '\0';
    ESP_LOGI(
        TAG,
        "starting provisioning; internal free=%u largest=%u dma_largest=%u",
        (unsigned int)heap_caps_get_free_size(MALLOC_CAP_INTERNAL),
        (unsigned int)heap_caps_get_largest_free_block(MALLOC_CAP_INTERNAL),
        (unsigned int)heap_caps_get_largest_free_block(
            MALLOC_CAP_INTERNAL | MALLOC_CAP_DMA
        )
    );
    const size_t largest_dma_block = heap_caps_get_largest_free_block(
        MALLOC_CAP_INTERNAL | MALLOC_CAP_DMA
    );
    const esp_err_t result =
        largest_dma_block < BOBE_PROVISIONING_MIN_DMA_BLOCK_BYTES
            ? ESP_ERR_NO_MEM
            : bobe_provisioning_start(provisioning_finished, NULL);
    atomic_store(&provisioning_start_result, result);
    atomic_store(&provisioning_start_pending, false);
    if (result == ESP_OK && preserve_connected_station) {
        const esp_err_t restore_result = esp_wifi_set_config(
            WIFI_IF_STA,
            &connected_station_config
        );
        if (restore_result != ESP_OK) {
            ESP_LOGW(
                TAG,
                "station restore after provisioning startup failed: %s",
                esp_err_to_name(restore_result)
            );
        }
        vTaskDelay(pdMS_TO_TICKS(100));
        const esp_err_t reconnect_result =
            restore_result == ESP_OK ? esp_wifi_connect() : restore_result;
        if (reconnect_result != ESP_OK) {
            ESP_LOGW(
                TAG,
                "station reconnect after provisioning startup failed: %s",
                esp_err_to_name(reconnect_result)
            );
        }
    }
    if (result == ESP_ERR_NOT_SUPPORTED || result == ESP_ERR_NOT_FOUND) {
        publish_state(BOBE_NETWORK_PROVISIONING_ABORTED);
        if (!atomic_load(&ip_up) && station_has_credentials()) {
            schedule_reconnect();
        }
        vTaskDelete(NULL);
        return;
    }
    if (result == ESP_ERR_INVALID_STATE &&
        bobe_provisioning_is_running()) {
        vTaskDelete(NULL);
        return;
    }
    if (result != ESP_OK) {
        ESP_LOGE(
            TAG,
            "provisioning start failed: %s",
            esp_err_to_name(result)
        );
        publish_state(BOBE_NETWORK_PROVISIONING_ABORTED);
        if (!atomic_load(&ip_up) && station_has_credentials()) {
            schedule_reconnect();
        }
    }
    vTaskDelete(NULL);
}

static esp_err_t start_provisioning_async(void)
{
    bool expected = false;
    if (bobe_provisioning_is_running() ||
        !atomic_compare_exchange_strong(
            &provisioning_start_pending,
            &expected,
            true
        )) {
        return ESP_ERR_INVALID_STATE;
    }
    publish_state(BOBE_NETWORK_UNPROVISIONED);
    bobe_wifi_recovery_reset(&wifi_recovery);
    atomic_store(&provisioning_start_result, ESP_ERR_NOT_FINISHED);
    if (xTaskCreatePinnedToCore(
            provisioning_start_task,
            "bobe_provision",
            6144,
            NULL,
            BOBE_TASK_PRIORITY_PROVISIONING,
            NULL,
            BOBE_NETWORK_CORE
        ) != pdPASS) {
        atomic_store(&provisioning_start_pending, false);
        publish_state(BOBE_NETWORK_DISCONNECTED);
        ESP_LOGE(TAG, "unable to create provisioning task");
        return ESP_ERR_NO_MEM;
    }
    return ESP_OK;
}

static bobe_wifi_failure_class_t classify_disconnect(uint8_t reason)
{
    if (reason == WIFI_REASON_NO_AP_FOUND) {
        return BOBE_WIFI_FAILURE_AP_UNAVAILABLE;
    }
    if (reason == WIFI_REASON_AUTH_EXPIRE ||
        reason == WIFI_REASON_AUTH_FAIL ||
        reason == WIFI_REASON_4WAY_HANDSHAKE_TIMEOUT ||
        reason == WIFI_REASON_HANDSHAKE_TIMEOUT ||
        reason == WIFI_REASON_NO_AP_FOUND_W_COMPATIBLE_SECURITY ||
        reason == WIFI_REASON_NO_AP_FOUND_IN_AUTHMODE_THRESHOLD) {
        return BOBE_WIFI_FAILURE_CREDENTIAL;
    }
    return BOBE_WIFI_FAILURE_OTHER;
}

static void wifi_event_handler(
    void *unused,
    esp_event_base_t event_base,
    int32_t event_id,
    void *event_data
)
{
    (void)unused;
    if (event_base == WIFI_EVENT && event_id == WIFI_EVENT_STA_START) {
        if (!station_has_credentials()) {
            ESP_LOGI(TAG, "station started without credentials; awaiting provisioning");
            return;
        }
        publish_state(BOBE_NETWORK_CONNECTING);
        ESP_ERROR_CHECK_WITHOUT_ABORT(esp_wifi_connect());
        return;
    }

    if (event_base == WIFI_EVENT && event_id == WIFI_EVENT_STA_CONNECTED) {
        bobe_wifi_recovery_reset(&wifi_recovery);
        return;
    }

    if (event_base == WIFI_EVENT && event_id == WIFI_EVENT_STA_DISCONNECTED) {
        atomic_store(&online, false);
        atomic_store(&ip_up, false);
        atomic_fetch_add(&connection_generation, 1U);
        const wifi_event_sta_disconnected_t *event = event_data;
        const bobe_wifi_failure_class_t failure =
            event != NULL ? classify_disconnect(event->reason)
                          : BOBE_WIFI_FAILURE_OTHER;
        const bool auth_failed =
            failure == BOBE_WIFI_FAILURE_CREDENTIAL;
        ESP_LOGW(
            TAG,
            "station disconnected reason=%u auth_failure=%d attempt=%u",
            event != NULL ? (unsigned int)event->reason : 0U,
            auth_failed,
            atomic_load(&reconnect_attempt)
        );
        publish_state(
            auth_failed ? BOBE_NETWORK_AUTH_FAILED
                        : BOBE_NETWORK_DISCONNECTED
        );
        const bool provisioning_pending =
            atomic_load(&provisioning_start_pending);
        if (atomic_load(&wifi_configuration_active)) {
            if (!atomic_load(&wifi_configuration_trial_started)) {
                xEventGroupSetBits(
                    wifi_configuration_events,
                    BOBE_WIFI_CONFIG_EVENT_QUIESCED
                );
                return;
            }
            if (failure == BOBE_WIFI_FAILURE_CREDENTIAL) {
                xEventGroupSetBits(
                    wifi_configuration_events,
                    BOBE_WIFI_CONFIG_EVENT_AUTH_FAILED
                );
            } else if (failure == BOBE_WIFI_FAILURE_AP_UNAVAILABLE) {
                xEventGroupSetBits(
                    wifi_configuration_events,
                    BOBE_WIFI_CONFIG_EVENT_AP_NOT_FOUND
                );
            }
            return;
        }
        if (bobe_provisioning_is_running() || provisioning_pending) {
            return;
        }
        if (bobe_wifi_recovery_observe(
                &wifi_recovery,
                failure,
                (uint64_t)esp_timer_get_time()
            )) {
            if (failure == BOBE_WIFI_FAILURE_AP_UNAVAILABLE) {
                ESP_LOGW(
                    TAG,
                    "saved Wi-Fi unavailable; reopening provisioning"
                );
            } else {
                ESP_LOGW(
                    TAG,
                    "Wi-Fi credentials rejected; reopening provisioning"
                );
            }
            const esp_err_t result = start_provisioning_async();
            if (result == ESP_OK || result == ESP_ERR_INVALID_STATE) {
                return;
            }
            ESP_LOGW(
                TAG,
                "provisioning recovery could not start: %s",
                esp_err_to_name(result)
            );
        }
        schedule_reconnect();
        return;
    }

    if (event_base == IP_EVENT && event_id == IP_EVENT_STA_GOT_IP) {
        atomic_store(&ip_up, true);
        atomic_fetch_add(&connection_generation, 1U);
        publish_local_connectivity();
        if (atomic_load(&wifi_configuration_active) &&
            atomic_load(&wifi_configuration_trial_started)) {
            xEventGroupSetBits(
                wifi_configuration_events,
                BOBE_WIFI_CONFIG_EVENT_CONNECTED
            );
        }
        start_time_sync_task();
        if (!bobe_provisioning_has_complete_claim()) {
            (void)start_provisioning_async();
        }
        return;
    }

    if (event_base == IP_EVENT && event_id == IP_EVENT_STA_LOST_IP) {
        atomic_store(&online, false);
        atomic_store(&ip_up, false);
        atomic_fetch_add(&connection_generation, 1U);
        publish_state(BOBE_NETWORK_DISCONNECTED);
        if (atomic_load(&wifi_configuration_active)) {
            return;
        }
        if (!bobe_provisioning_is_running() &&
            !atomic_load(&provisioning_start_pending) &&
            esp_wifi_disconnect() != ESP_OK) {
            schedule_reconnect();
        }
    }
}

static esp_err_t initialize_mdns(void)
{
    uint8_t mac[6];
    ESP_RETURN_ON_ERROR(
        esp_read_mac(mac, ESP_MAC_WIFI_STA),
        TAG,
        "read mDNS identity"
    );
    char hostname[32];
    snprintf(
        hostname,
        sizeof(hostname),
        "%s-%02x%02x%02x",
        BOBE_HOSTNAME_PREFIX,
        mac[3],
        mac[4],
        mac[5]
    );
    ESP_RETURN_ON_ERROR(mdns_init(), TAG, "initialize mDNS");
    ESP_RETURN_ON_ERROR(
        mdns_hostname_set(hostname),
        TAG,
        "set mDNS hostname"
    );
    return mdns_instance_name_set(BOBE_MDNS_INSTANCE_NAME);
}

static size_t txt_value_length(const mdns_result_t *result, size_t index)
{
    return result->txt_value_len != NULL
               ? result->txt_value_len[index]
               : strlen(result->txt[index].value);
}

static bool txt_value_equals(
    const mdns_result_t *result,
    size_t index,
    const char *expected
)
{
    if (result->txt[index].value == NULL || expected == NULL) {
        return false;
    }
    const size_t expected_length = strlen(expected);
    const size_t value_length = txt_value_length(result, index);
    return value_length == expected_length &&
           memcmp(result->txt[index].value, expected, expected_length) == 0;
}

static bool copy_txt_value(
    const mdns_result_t *result,
    const char *key,
    char *output,
    size_t output_size
)
{
    for (size_t index = 0; index < result->txt_count; ++index) {
        if (result->txt[index].key == NULL ||
            strcmp(result->txt[index].key, key) != 0 ||
            result->txt[index].value == NULL) {
            continue;
        }
        const size_t value_length = txt_value_length(result, index);
        if (value_length == 0 || value_length >= output_size) {
            return false;
        }
        memcpy(output, result->txt[index].value, value_length);
        output[value_length] = '\0';
        return true;
    }
    return false;
}

static bool txt_matches(
    const mdns_result_t *result,
    const char *key,
    const char *expected
)
{
    for (size_t index = 0; index < result->txt_count; ++index) {
        if (result->txt[index].key != NULL &&
            strcmp(result->txt[index].key, key) == 0) {
            return txt_value_equals(result, index, expected);
        }
    }
    return false;
}

static bool copy_local_hostname(
    const char *hostname,
    char *output,
    size_t output_size
)
{
    if (hostname == NULL || output_size == 0) {
        return false;
    }
    size_t length = strlen(hostname);
    while (length > 0 && hostname[length - 1] == '.') {
        --length;
    }
    static const char suffix[] = ".local";
    const size_t suffix_length = sizeof(suffix) - 1;
    if (length >= suffix_length &&
        strcasecmp(hostname + length - suffix_length, suffix) == 0) {
        length -= suffix_length;
    }
    if (length == 0 || length + sizeof(suffix) > output_size) {
        return false;
    }
    for (size_t index = 0; index < length; ++index) {
        const unsigned char value = (unsigned char)hostname[index];
        if (!(isalnum(value) || value == '-' || value == '.')) {
            return false;
        }
        output[index] = hostname[index];
    }
    memcpy(output + length, suffix, sizeof(suffix));
    return true;
}

static bool path_is_safe(const char *path)
{
    if (path == NULL || path[0] != '/' || strstr(path, "..") != NULL ||
        strchr(path, '?') != NULL || strchr(path, '#') != NULL ||
        strchr(path, '\\') != NULL) {
        return false;
    }
    for (const char *cursor = path; *cursor != '\0'; ++cursor) {
        const unsigned char value = (unsigned char)*cursor;
        if (value <= 0x20 || value >= 0x7F) {
            return false;
        }
    }
    return true;
}

static bool load_trusted_key_id(char *output, size_t output_size)
{
    uint8_t active_slot = 0;
    const esp_err_t slot_result =
        bobe_storage_load_active_slot(&active_slot);
    if (slot_result != ESP_OK) {
        return false;
    }
    const bobe_storage_slot_keys_t *keys =
        bobe_storage_slot_keys(active_slot);
    if (keys == NULL) {
        return false;
    }

    nvs_handle_t nvs;
    if (nvs_open(BOBE_CLAIM_NAMESPACE, NVS_READONLY, &nvs) != ESP_OK) {
        return false;
    }
    uint8_t ready = 0;
    size_t length = output_size;
    esp_err_t result = nvs_get_u8(nvs, BOBE_CLAIM_READY_KEY, &ready);
    if (result == ESP_OK && ready == 1) {
        result = nvs_get_str(
            nvs,
            keys->trust_key_id,
            output,
            &length
        );
    }
    nvs_close(nvs);
    return result == ESP_OK && ready == 1 && output[0] != '\0';
}

esp_err_t bobe_connectivity_init(
    bobe_network_callback_t callback,
    void *context
)
{
    network_callback = callback;
    network_callback_context = context;
    wifi_configuration_events = xEventGroupCreate();
    wifi_configuration_mutex = xSemaphoreCreateMutex();
    if (wifi_configuration_events == NULL ||
        wifi_configuration_mutex == NULL) {
        return ESP_ERR_NO_MEM;
    }

    restore_clock_floor();
    ESP_RETURN_ON_ERROR(
        esp_netif_init(),
        TAG,
        "initialize network interfaces"
    );

    esp_err_t result = esp_event_loop_create_default();
    if (result != ESP_OK && result != ESP_ERR_INVALID_STATE) {
        return result;
    }
    if (esp_netif_create_default_wifi_sta() == NULL) {
        return ESP_ERR_NO_MEM;
    }

    const wifi_init_config_t wifi_init = WIFI_INIT_CONFIG_DEFAULT();
    ESP_RETURN_ON_ERROR(esp_wifi_init(&wifi_init), TAG, "initialize Wi-Fi");
    ESP_RETURN_ON_ERROR(
        esp_wifi_set_storage(WIFI_STORAGE_FLASH),
        TAG,
        "select persistent Wi-Fi storage"
    );
    ESP_RETURN_ON_ERROR(
        esp_event_handler_register(
            WIFI_EVENT,
            ESP_EVENT_ANY_ID,
            wifi_event_handler,
            NULL
        ),
        TAG,
        "register Wi-Fi events"
    );
    ESP_RETURN_ON_ERROR(
        esp_event_handler_register(
            IP_EVENT,
            IP_EVENT_STA_GOT_IP,
            wifi_event_handler,
            NULL
        ),
        TAG,
        "register IP events"
    );
    ESP_RETURN_ON_ERROR(
        esp_event_handler_register(
            IP_EVENT,
            IP_EVENT_STA_LOST_IP,
            wifi_event_handler,
            NULL
        ),
        TAG,
        "register lost-IP events"
    );

    const esp_timer_create_args_t reconnect_timer_config = {
        .callback = reconnect_timer_callback,
        .name = "bobe_wifi_retry",
    };
    ESP_RETURN_ON_ERROR(
        esp_timer_create(&reconnect_timer_config, &reconnect_timer),
        TAG,
        "create reconnect timer"
    );
    ESP_RETURN_ON_ERROR(initialize_mdns(), TAG, "initialize discovery");
    ESP_RETURN_ON_ERROR(
        esp_wifi_set_mode(WIFI_MODE_STA),
        TAG,
        "set station mode"
    );

    wifi_config_t wifi_config;
    memset(&wifi_config, 0, sizeof(wifi_config));
#if defined(CONFIG_BOBE_ENABLE_DEV_WIFI_BOOTSTRAP)
    {
        const size_t ssid_length = strlen(CONFIG_BOBE_WIFI_SSID);
        const size_t password_length = strlen(CONFIG_BOBE_WIFI_PASSWORD);
        if (ssid_length >= sizeof(wifi_config.sta.ssid) ||
            password_length >= sizeof(wifi_config.sta.password)) {
            return ESP_ERR_INVALID_SIZE;
        }
        memcpy(
            wifi_config.sta.ssid,
            CONFIG_BOBE_WIFI_SSID,
            ssid_length + 1
        );
        memcpy(
            wifi_config.sta.password,
            CONFIG_BOBE_WIFI_PASSWORD,
            password_length + 1
        );
        wifi_config.sta.threshold.authmode =
            password_length == 0 ? WIFI_AUTH_OPEN : WIFI_AUTH_WPA2_PSK;
        wifi_config.sta.pmf_cfg.capable = true;
        wifi_config.sta.pmf_cfg.required = false;
        ESP_RETURN_ON_ERROR(
            esp_wifi_set_config(WIFI_IF_STA, &wifi_config),
            TAG,
            "set development station configuration"
        );
    }
#else
    ESP_RETURN_ON_ERROR(
        esp_wifi_get_config(WIFI_IF_STA, &wifi_config),
        TAG,
        "load stored station configuration"
    );
#endif

    if (wifi_config.sta.ssid[0] == '\0') {
        ESP_LOGW(TAG, "Wi-Fi is not provisioned");
        ESP_RETURN_ON_ERROR(
            start_provisioning_async(),
            TAG,
            "queue provisioning startup"
        );
        const TickType_t deadline =
            xTaskGetTickCount() +
            pdMS_TO_TICKS(BOBE_PROVISIONING_START_TIMEOUT_MS);
        while (atomic_load(&provisioning_start_pending)) {
            if ((int32_t)(xTaskGetTickCount() - deadline) >= 0) {
                return ESP_ERR_TIMEOUT;
            }
            vTaskDelay(pdMS_TO_TICKS(10));
        }
        const esp_err_t provisioning_result =
            (esp_err_t)atomic_load(&provisioning_start_result);
        return provisioning_result == ESP_OK ||
                       provisioning_result == ESP_ERR_INVALID_STATE ||
                       provisioning_result == ESP_ERR_NOT_SUPPORTED ||
                       provisioning_result == ESP_ERR_NOT_FOUND
                   ? ESP_OK
                   : provisioning_result;
    }
    ESP_RETURN_ON_ERROR(
        esp_wifi_start(),
        TAG,
        "start Wi-Fi station"
    );
    return esp_wifi_set_ps(WIFI_PS_NONE);
}

bool bobe_connectivity_is_online(void)
{
    return atomic_load(&online);
}

bobe_wifi_config_result_t bobe_connectivity_configure_wifi(
    const uint8_t *ssid,
    size_t ssid_length,
    const uint8_t *passphrase,
    size_t passphrase_length,
    TickType_t timeout
)
{
    if (wifi_configuration_mutex == NULL ||
        wifi_configuration_events == NULL) {
        return BOBE_WIFI_CONFIG_NOT_READY;
    }
    if (ssid == NULL || ssid_length == 0 || ssid_length > 32 ||
        passphrase == NULL || passphrase_length > 63 ||
        (passphrase_length > 0 && passphrase_length < 8)) {
        return BOBE_WIFI_CONFIG_FAILED;
    }
    if (xSemaphoreTake(wifi_configuration_mutex, 0) != pdTRUE) {
        return BOBE_WIFI_CONFIG_BUSY;
    }

    bobe_wifi_config_result_t result = BOBE_WIFI_CONFIG_FAILED;
    bool stopped_provisioning = false;
    atomic_store(&wifi_configuration_active, true);
    atomic_store(&wifi_configuration_trial_started, false);
    if (atomic_load(&provisioning_start_pending)) {
        result = BOBE_WIFI_CONFIG_BUSY;
        goto done_active;
    }
    if (bobe_provisioning_is_running()) {
        stopped_provisioning = true;
        (void)bobe_provisioning_stop();
        const TickType_t deadline =
            xTaskGetTickCount() + pdMS_TO_TICKS(3000);
        while (bobe_provisioning_is_running() &&
               (int32_t)(xTaskGetTickCount() - deadline) < 0) {
            vTaskDelay(pdMS_TO_TICKS(20));
        }
        if (bobe_provisioning_is_running()) {
            result = BOBE_WIFI_CONFIG_BUSY;
            goto done_active;
        }
    }
    if (esp_timer_is_active(reconnect_timer)) {
        (void)esp_timer_stop(reconnect_timer);
    }

    wifi_config_t previous;
    memset(&previous, 0, sizeof(previous));
    if (esp_wifi_set_storage(WIFI_STORAGE_FLASH) != ESP_OK) {
        goto done_active;
    }
    if (esp_wifi_get_config(WIFI_IF_STA, &previous) != ESP_OK) {
        goto done_active;
    }

    wifi_config_t proposed;
    memset(&proposed, 0, sizeof(proposed));
    memcpy(proposed.sta.ssid, ssid, ssid_length);
    memcpy(proposed.sta.password, passphrase, passphrase_length);
    proposed.sta.threshold.authmode =
        passphrase_length == 0 ? WIFI_AUTH_OPEN : WIFI_AUTH_WPA2_PSK;
    proposed.sta.pmf_cfg.capable = true;
    proposed.sta.pmf_cfg.required = false;

    xEventGroupClearBits(
        wifi_configuration_events,
        BOBE_WIFI_CONFIG_EVENT_CONNECTED |
            BOBE_WIFI_CONFIG_EVENT_AUTH_FAILED |
            BOBE_WIFI_CONFIG_EVENT_AP_NOT_FOUND |
            BOBE_WIFI_CONFIG_EVENT_QUIESCED
    );
    const esp_err_t disconnect_result = esp_wifi_disconnect();
    if (disconnect_result == ESP_OK) {
        (void)xEventGroupWaitBits(
            wifi_configuration_events,
            BOBE_WIFI_CONFIG_EVENT_QUIESCED,
            pdTRUE,
            pdFALSE,
            pdMS_TO_TICKS(2000)
        );
    }
    if (esp_wifi_set_storage(WIFI_STORAGE_RAM) != ESP_OK ||
        esp_wifi_set_config(WIFI_IF_STA, &proposed) != ESP_OK) {
        goto rollback;
    }
    xEventGroupClearBits(
        wifi_configuration_events,
        BOBE_WIFI_CONFIG_EVENT_CONNECTED |
            BOBE_WIFI_CONFIG_EVENT_AUTH_FAILED |
            BOBE_WIFI_CONFIG_EVENT_AP_NOT_FOUND
    );
    atomic_store(&wifi_configuration_trial_started, true);
    if (
        esp_wifi_connect() != ESP_OK) {
        goto rollback;
    }

    const TickType_t deadline = xTaskGetTickCount() + timeout;
    for (;;) {
        const TickType_t now = xTaskGetTickCount();
        if ((int32_t)(now - deadline) >= 0) {
            result = BOBE_WIFI_CONFIG_TIMEOUT;
            break;
        }
        const EventBits_t events = xEventGroupWaitBits(
            wifi_configuration_events,
            BOBE_WIFI_CONFIG_EVENT_CONNECTED |
                BOBE_WIFI_CONFIG_EVENT_AUTH_FAILED |
                BOBE_WIFI_CONFIG_EVENT_AP_NOT_FOUND,
            pdTRUE,
            pdFALSE,
            deadline - now
        );
        if ((events & BOBE_WIFI_CONFIG_EVENT_CONNECTED) != 0) {
            wifi_ap_record_t access_point = {0};
            if (esp_wifi_sta_get_ap_info(&access_point) == ESP_OK &&
                memcmp(access_point.ssid, proposed.sta.ssid, 32) == 0) {
                if (esp_wifi_set_storage(WIFI_STORAGE_FLASH) == ESP_OK &&
                    esp_wifi_set_config(WIFI_IF_STA, &proposed) == ESP_OK) {
                    result = BOBE_WIFI_CONFIGURED;
                    goto done_active;
                }
                result = BOBE_WIFI_CONFIG_FAILED;
                break;
            }
            continue;
        }
        if ((events & BOBE_WIFI_CONFIG_EVENT_AUTH_FAILED) != 0) {
            result = BOBE_WIFI_CONFIG_AUTH_FAILED;
            break;
        }
        if ((events & BOBE_WIFI_CONFIG_EVENT_AP_NOT_FOUND) != 0) {
            result = BOBE_WIFI_CONFIG_AP_NOT_FOUND;
            break;
        }
    }

rollback:
    (void)esp_wifi_disconnect();
    (void)esp_wifi_set_storage(WIFI_STORAGE_RAM);
    if (esp_wifi_set_config(WIFI_IF_STA, &previous) == ESP_OK &&
        previous.sta.ssid[0] != '\0') {
        atomic_store(&wifi_configuration_trial_started, false);
        atomic_store(&wifi_configuration_active, false);
        (void)esp_wifi_set_storage(WIFI_STORAGE_FLASH);
        if (esp_wifi_connect() != ESP_OK) {
            schedule_reconnect();
        }
    } else {
        atomic_store(&wifi_configuration_trial_started, false);
        atomic_store(&wifi_configuration_active, false);
        (void)esp_wifi_set_storage(WIFI_STORAGE_FLASH);
        const esp_err_t provisioning_result = start_provisioning_async();
        if (provisioning_result != ESP_OK &&
            provisioning_result != ESP_ERR_INVALID_STATE) {
            schedule_reconnect();
        }
    }
    if (stopped_provisioning && previous.sta.ssid[0] == '\0' &&
        !bobe_provisioning_is_running()) {
        (void)start_provisioning_async();
    }
    goto done;

done_active:
    if (esp_timer_is_active(reconnect_timer)) {
        (void)esp_timer_stop(reconnect_timer);
    }
    atomic_store(&wifi_configuration_trial_started, false);
    atomic_store(&wifi_configuration_active, false);
done:
    xSemaphoreGive(wifi_configuration_mutex);
    return result;
}

bool bobe_connectivity_is_provisioning(void)
{
    return bobe_provisioning_is_running() ||
           atomic_load(&provisioning_start_pending);
}

void bobe_connectivity_confirm_authenticated_time(void)
{
    save_authenticated_clock_floor();
}

uint32_t bobe_connectivity_generation(void)
{
    return atomic_load(&connection_generation);
}

int bobe_connectivity_rssi_dbm(void)
{
    if (!atomic_load(&online)) {
        return INT32_MIN;
    }
    wifi_ap_record_t access_point = {0};
    return esp_wifi_sta_get_ap_info(&access_point) == ESP_OK
               ? access_point.rssi
               : INT32_MIN;
}

esp_err_t bobe_connectivity_open_provisioning_window(void)
{
    return start_provisioning_async();
}

esp_err_t bobe_connectivity_discover_hub(
    bobe_hub_candidate_t *candidate,
    uint32_t timeout_ms
)
{
    if (candidate == NULL || timeout_ms == 0 ||
        !atomic_load(&online)) {
        return ESP_ERR_INVALID_ARG;
    }
    memset(candidate, 0, sizeof(*candidate));

    char trusted_key_id[sizeof(candidate->key_id)];
    if (!load_trusted_key_id(trusted_key_id, sizeof(trusted_key_id))) {
        ESP_LOGW(TAG, "mDNS discovery requires a complete claim");
        return ESP_ERR_INVALID_STATE;
    }

    mdns_result_t *results = NULL;
    const esp_err_t query_result = mdns_query_ptr(
        BOBE_MDNS_SERVICE,
        BOBE_MDNS_PROTO,
        timeout_ms,
        8,
        &results
    );
    if (query_result != ESP_OK) {
        return query_result;
    }

    esp_err_t result = ESP_ERR_NOT_FOUND;
    for (const mdns_result_t *item = results;
         item != NULL;
         item = item->next) {
        char key_id[sizeof(candidate->key_id)] = {0};
        char path[64] = "/body/v1";
        char local_hostname[96];
        if (item->ttl == 0 || item->port == 0 ||
            !copy_local_hostname(
                item->hostname,
                local_hostname,
                sizeof(local_hostname)
            ) ||
            !txt_matches(item, "pv", "1") ||
            !txt_matches(item, "tls", "1") ||
            !txt_matches(item, "auth", BOBE_BODY_MDNS_AUTH) ||
            !copy_txt_value(item, "kid", key_id, sizeof(key_id)) ||
            strcmp(key_id, trusted_key_id) != 0) {
            continue;
        }
        (void)copy_txt_value(item, "path", path, sizeof(path));
        if (!path_is_safe(path)) {
            continue;
        }

        const int written = snprintf(
            candidate->uri,
            sizeof(candidate->uri),
            "wss://%s:%u%s",
            local_hostname,
            item->port,
            path
        );
        if (written <= 0 ||
            (size_t)written >= sizeof(candidate->uri)) {
            continue;
        }
        memcpy(candidate->key_id, key_id, strlen(key_id) + 1);
        result = ESP_OK;
        break;
    }
    mdns_query_results_free(results);
    return result;
}
