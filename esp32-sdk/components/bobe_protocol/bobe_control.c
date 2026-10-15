#include "bobe_protocol.h"
#include "bobe_json.h"

#include "cJSON.h"

#include <ctype.h>
#include <string.h>

static bool add_number(cJSON *root, const char *name, double value)
{
    return cJSON_AddNumberToObject(root, name, value) != NULL;
}

static bool add_string(cJSON *root, const char *name, const char *value)
{
    return cJSON_AddStringToObject(root, name, value != NULL ? value : "") != NULL;
}

static bool copy_string_field(
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
    if (length >= output_size) {
        return false;
    }
    memcpy(output, item->valuestring, length + 1);
    return true;
}

static bool copy_nonempty_string_field(
    const cJSON *root,
    const char *name,
    char *output,
    size_t output_size
)
{
    return copy_string_field(root, name, output, output_size) &&
           output[0] != '\0';
}

static bool copy_optional_nonempty_string_field(
    const cJSON *root,
    const char *name,
    char *output,
    size_t output_size
)
{
    if (cJSON_GetObjectItemCaseSensitive(root, name) == NULL) {
        output[0] = '\0';
        return true;
    }
    return copy_nonempty_string_field(root, name, output, output_size);
}

static bool has_duplicate_keys(const cJSON *root)
{
    for (const cJSON *item = root->child; item != NULL; item = item->next) {
        if (item->string == NULL) {
            continue;
        }
        for (const cJSON *later = item->next;
             later != NULL;
             later = later->next) {
            if (later->string != NULL &&
                strcmp(item->string, later->string) == 0) {
                return true;
            }
        }
    }
    return false;
}

static bool contains_nul_escape(const char *input, size_t input_size)
{
    for (size_t index = 0; index + 5U < input_size; ++index) {
        if (input[index] != '\\') {
            continue;
        }
        if (input[index + 1U] == 'u' &&
            input[index + 2U] == '0' &&
            input[index + 3U] == '0' &&
            input[index + 4U] == '0' &&
            input[index + 5U] == '0') {
            return true;
        }
        ++index;
    }
    return false;
}

static bool read_u32_field(const cJSON *root, const char *name, uint32_t *value)
{
    const cJSON *item = cJSON_GetObjectItemCaseSensitive(root, name);
    if (!cJSON_IsNumber(item) || item->valuedouble < 0 ||
        item->valuedouble > UINT32_MAX) {
        return false;
    }
    const uint32_t converted = (uint32_t)item->valuedouble;
    if ((double)converted != item->valuedouble) {
        return false;
    }
    *value = converted;
    return true;
}

static bool read_bool_field(const cJSON *root, const char *name, bool *value)
{
    const cJSON *item = cJSON_GetObjectItemCaseSensitive(root, name);
    if (!cJSON_IsBool(item)) return false;
    *value = cJSON_IsTrue(item);
    return true;
}

static bool read_optional_u32_field(
    const cJSON *root,
    const char *name,
    uint32_t *value
)
{
    return cJSON_GetObjectItemCaseSensitive(root, name) == NULL ||
           read_u32_field(root, name, value);
}

static bool read_u64_field(const cJSON *root, const char *name, uint64_t *value)
{
    const cJSON *item = cJSON_GetObjectItemCaseSensitive(root, name);
    if (!cJSON_IsNumber(item) || item->valuedouble < 0 ||
        item->valuedouble > 9007199254740991.0) {
        return false;
    }
    const uint64_t converted = (uint64_t)item->valuedouble;
    if ((double)converted != item->valuedouble) {
        return false;
    }
    *value = converted;
    return true;
}

const char *bobe_lifecycle_state_name(bobe_lifecycle_state_t state)
{
    static const char *const names[] = {
        "booting",
        "unprovisioned",
        "joining_wifi",
        "discovering_hub",
        "connecting",
        "online",
        "reconnecting",
        "updating",
        "rebooting",
        "low_battery",
        "shutting_down",
        "fatal_error",
    };
    return state >= BOBE_LIFECYCLE_BOOTING &&
                   state <= BOBE_LIFECYCLE_FATAL_ERROR
               ? names[state]
               : "unknown";
}

const char *bobe_turn_state_name(bobe_turn_state_t state)
{
    static const char *const names[] = {
        "idle",
        "requesting_capture",
        "listening",
        "waiting_for_runtime",
        "thinking",
        "speaking",
        "interrupted",
        "aborted",
    };
    return state >= BOBE_TURN_IDLE && state <= BOBE_TURN_ABORTED
               ? names[state]
               : "unknown";
}

const char *bobe_face_expression_name(bobe_face_expression_t expression)
{
    static const char *const names[] = {
        "booting",
        "unprovisioned",
        "connecting",
        "idle",
        "attentive",
        "listening",
        "thinking",
        "speaking",
        "happy",
        "concerned",
        "muted",
        "offline",
        "updating",
        "error",
        "sleeping",
    };
    return expression >= BOBE_FACE_BOOTING && expression <= BOBE_FACE_SLEEPING
               ? names[expression]
               : "error";
}

static bool parse_face_expression(const char *name, bobe_face_expression_t *expression)
{
    for (int value = BOBE_FACE_BOOTING; value <= BOBE_FACE_SLEEPING; ++value) {
        if (strcmp(name, bobe_face_expression_name((bobe_face_expression_t)value)) == 0) {
            *expression = (bobe_face_expression_t)value;
            return true;
        }
    }
    return false;
}

const char *bobe_face_theme_name(bobe_face_theme_id_t theme)
{
    static const char *const names[] = {
        "warm-aperture",
        "warm-lintel",
        "warm-ledge",
        "kettle-tall-bright",
        "kettle-soft-bowl",
        "solar-wide-awake",
        "solar-crown",
    };
    if (theme <= BOBE_FACE_THEME_DEFAULT || theme >= BOBE_FACE_THEME_COUNT) {
        return NULL;
    }
    return names[theme - BOBE_FACE_THEME_WARM_APERTURE];
}

bool bobe_face_theme_parse(const char *name, bobe_face_theme_id_t *theme)
{
    if (name == NULL || theme == NULL) {
        return false;
    }
    for (int value = BOBE_FACE_THEME_WARM_APERTURE;
         value < BOBE_FACE_THEME_COUNT;
         ++value) {
        const char *const candidate =
            bobe_face_theme_name((bobe_face_theme_id_t)value);
        if (candidate != NULL && strcmp(name, candidate) == 0) {
            *theme = (bobe_face_theme_id_t)value;
            return true;
        }
    }
    return false;
}

static bool parse_turn_state(const char *name, bobe_turn_state_t *state)
{
    for (int value = BOBE_TURN_IDLE; value <= BOBE_TURN_ABORTED; ++value) {
        if (strcmp(name, bobe_turn_state_name((bobe_turn_state_t)value)) == 0) {
            *state = (bobe_turn_state_t)value;
            return true;
        }
    }
    return false;
}

bobe_protocol_result_t bobe_protocol_build_hello(
    char *output,
    size_t output_size,
    const bobe_hello_info_t *info
)
{
    if (info == NULL || info->device_id == NULL || info->device_id[0] == '\0' ||
        strlen(info->device_id) >= BOBE_DEVICE_ID_MAX_BYTES || info->boot_id == NULL ||
        strlen(info->boot_id) != 32U || info->firmware_version == NULL ||
        strlen(info->firmware_version) > 64U || info->hardware_revision == NULL ||
        strlen(info->hardware_revision) > 64U ||
        info->max_playback_credit_ms > BOBE_MAX_PLAYBACK_CREDIT_MS) {
        return BOBE_PROTOCOL_INVALID_ARGUMENT;
    }
    const bobe_capabilities_t *caps = &info->capabilities;
    const uint8_t minor = bobe_capabilities_protocol_minor(caps);
    if ((caps->display_width == 0) != (caps->display_height == 0) ||
        (caps->text_columns == 0) != (caps->text_rows == 0) ||
        (caps->text_columns != 0 && !caps->text_output) ||
        ((caps->aec || caps->wake_word) && !caps->microphone) ||
        (caps->speaker && info->max_playback_credit_ms < BOBE_AUDIO_FRAME_DURATION_MS)) {
        return BOBE_PROTOCOL_INVALID_ARGUMENT;
    }

    cJSON *root = cJSON_CreateObject();
    cJSON *capabilities = cJSON_CreateObject();
    cJSON *display = cJSON_CreateObject();
    cJSON *text_display = cJSON_CreateObject();
    cJSON *limits = cJSON_CreateObject();
    if (root == NULL || capabilities == NULL || display == NULL ||
        text_display == NULL || limits == NULL) {
        cJSON_Delete(root);
        cJSON_Delete(capabilities);
        cJSON_Delete(display);
        cJSON_Delete(text_display);
        cJSON_Delete(limits);
        return BOBE_PROTOCOL_MALFORMED;
    }

    bool ok = add_string(root, "type", "hello") &&
              add_number(root, "protocol_major", BOBE_PROTOCOL_MAJOR) &&
              add_number(root, "protocol_minor", minor) &&
              add_string(root, "device_id", info->device_id) &&
              add_string(root, "boot_id", info->boot_id) &&
              add_string(root, "firmware", info->firmware_version) &&
              add_string(root, "hardware", info->hardware_revision) &&
              add_number(display, "width", caps->display_width) &&
              add_number(display, "height", caps->display_height) &&
              add_number(display, "face_schema", caps->face_schema);
    if (caps->display_width != 0) {
        if (ok && cJSON_AddItemToObject(capabilities, "display", display)) display = NULL;
        else ok = false;
    }
    ok = ok &&
         cJSON_AddBoolToObject(capabilities, "microphone", caps->microphone) &&
         cJSON_AddBoolToObject(capabilities, "speaker", caps->speaker) &&
         cJSON_AddBoolToObject(capabilities, "battery", caps->battery) &&
         cJSON_AddBoolToObject(capabilities, "aec", caps->aec) &&
         cJSON_AddBoolToObject(capabilities, "wake_word", caps->wake_word);
    if (minor >= 2U) {
        ok = ok && cJSON_AddBoolToObject(capabilities, "text_input", caps->text_input) &&
             cJSON_AddBoolToObject(capabilities, "text_output", caps->text_output) &&
             cJSON_AddBoolToObject(capabilities, "actions", caps->actions) &&
             cJSON_AddBoolToObject(capabilities, "sensors", caps->sensors);
        if (caps->text_columns != 0) {
            ok = ok && add_number(text_display, "columns", caps->text_columns) &&
                 add_number(text_display, "rows", caps->text_rows);
            if (ok && cJSON_AddItemToObject(capabilities, "text_display", text_display)) text_display = NULL;
            else ok = false;
        }
    }
    if (ok && cJSON_AddItemToObject(root, "capabilities", capabilities)) {
        capabilities = NULL;
    } else {
        ok = false;
    }
    ok = ok &&
         add_number(limits, "max_control_bytes", BOBE_CONTROL_MAX_BYTES) &&
         add_number(
             limits,
             "max_playback_credit_ms",
             info->max_playback_credit_ms
         );
    if (ok && cJSON_AddItemToObject(root, "limits", limits)) {
        limits = NULL;
    } else {
        ok = false;
    }

    cJSON_Delete(display);
    cJSON_Delete(text_display);
    if (!ok) {
        cJSON_Delete(root);
        cJSON_Delete(capabilities);
        cJSON_Delete(limits);
        return BOBE_PROTOCOL_MALFORMED;
    }
    return bobe_json_print(root, output, output_size);
}

bobe_protocol_result_t bobe_protocol_build_health(
    char *output,
    size_t output_size,
    const bobe_health_info_t *info
)
{
    if (info == NULL) {
        return BOBE_PROTOCOL_INVALID_ARGUMENT;
    }

    cJSON *root = cJSON_CreateObject();
    if (root == NULL) {
        return BOBE_PROTOCOL_MALFORMED;
    }
    const bool ok = add_string(root, "type", "health.report") &&
                    add_number(root, "protocol_major", BOBE_PROTOCOL_MAJOR) &&
                    add_number(root, "uptime_s", info->uptime_s) &&
                    add_number(root, "wifi_rssi_dbm", info->wifi_rssi_dbm) &&
                    add_number(
                        root,
                        "free_internal_heap",
                        (double)info->free_internal_heap
                    ) &&
                    add_number(
                        root,
                        "minimum_internal_heap",
                        (double)info->minimum_internal_heap
                    ) &&
                    add_number(root, "free_psram", (double)info->free_psram) &&
                    add_number(root, "audio_rx_overruns", info->audio_rx_overruns) &&
                    add_number(root, "audio_tx_underruns", info->audio_tx_underruns) &&
                    add_string(
                        root,
                        "lifecycle",
                        bobe_lifecycle_state_name(info->lifecycle)
                    ) &&
                    add_string(root, "turn", bobe_turn_state_name(info->turn)) &&
                    cJSON_AddBoolToObject(
                        root,
                        "microphone_muted",
                        info->microphone_muted
                    ) != NULL &&
                    add_string(root, "app_version", info->app_version) &&
                    add_number(root, "build_number", info->build_number) &&
                    add_number(root, "running_slot", info->running_slot) &&
                    cJSON_AddBoolToObject(
                        root,
                        "ota_pending_verification",
                        info->ota_pending_verification
                    ) != NULL &&
                    add_number(root, "reset_reason", info->reset_reason) &&
                    add_number(
                        root,
                        "transport_generation",
                        info->transport_generation
                    ) &&
                    add_number(root, "core_queue_depth", info->core_queue_depth) &&
                    add_number(
                        root,
                        "core_queue_capacity",
                        info->core_queue_capacity
                    ) &&
                    add_number(
                        root,
                        "core_stack_free_words",
                        info->core_stack_free_words
                    ) &&
                    add_number(
                        root,
                        "display_stack_free_words",
                        info->display_stack_free_words
                    ) &&
                    add_number(
                        root,
                        "display_flush_submitted",
                        info->display_flush_submitted
                    ) &&
                    add_number(
                        root,
                        "display_flush_errors",
                        info->display_flush_errors
                    ) &&
                    add_number(
                        root,
                        "display_flush_timeouts",
                        info->display_flush_timeouts
                    ) &&
                    add_number(
                        root,
                        "display_flush_last_us",
                        info->display_flush_last_us
                    ) &&
                    add_number(
                        root,
                        "display_flush_max_us",
                        info->display_flush_max_us
                    ) &&
                    add_number(
                        root,
                        "playback_sequence_gaps",
                        info->playback_sequence_gaps
                    ) &&
                    add_number(
                        root,
                        "temperature_millicelsius",
                        info->temperature_millicelsius
                    ) &&
                    add_number(root, "cpu_frequency_mhz", info->cpu_frequency_mhz) &&
                    add_number(root, "display_frame_rate", info->display_frame_rate) &&
                    add_number(root, "backlight_percent", info->backlight_percent);
    if (!ok) {
        cJSON_Delete(root);
        return BOBE_PROTOCOL_MALFORMED;
    }
    return bobe_json_print(root, output, output_size);
}

bobe_protocol_result_t bobe_protocol_build_button_event(
    char *output,
    size_t output_size,
    const char *boot_id,
    const bobe_button_event_t *event
)
{
    static const char *const button_names[] = {
        "action",
        "volume_down",
        "volume_up",
    };
    static const char *const action_names[] = {
        "pressed",
        "released",
        "long_press",
    };
    if (boot_id == NULL || event == NULL ||
        event->button < BOBE_BUTTON_ACTION ||
        event->button > BOBE_BUTTON_VOLUME_UP ||
        event->action < BOBE_BUTTON_PRESSED ||
        event->action > BOBE_BUTTON_LONG_PRESS) {
        return BOBE_PROTOCOL_INVALID_ARGUMENT;
    }

    cJSON *root = cJSON_CreateObject();
    if (root == NULL) {
        return BOBE_PROTOCOL_MALFORMED;
    }
    const bool ok = add_string(root, "type", "button.event") &&
                    add_number(root, "protocol_major", BOBE_PROTOCOL_MAJOR) &&
                    add_string(root, "boot_id", boot_id) &&
                    add_number(root, "event_id", event->event_id) &&
                    add_string(root, "button", button_names[event->button]) &&
                    add_string(root, "action", action_names[event->action]) &&
                    add_number(root, "monotonic_us", (double)event->monotonic_us);
    if (!ok) {
        cJSON_Delete(root);
        return BOBE_PROTOCOL_MALFORMED;
    }
    return bobe_json_print(root, output, output_size);
}

bobe_protocol_result_t bobe_protocol_build_battery_state(
    char *output,
    size_t output_size,
    const bobe_battery_state_t *battery
)
{
    if (battery == NULL || battery->percent > 100) {
        return BOBE_PROTOCOL_INVALID_ARGUMENT;
    }
    cJSON *root = cJSON_CreateObject();
    if (root == NULL) {
        return BOBE_PROTOCOL_MALFORMED;
    }
    const bool ok = add_string(root, "type", "battery.state") &&
                    add_number(root, "protocol_major", BOBE_PROTOCOL_MAJOR) &&
                    add_number(root, "raw_adc", battery->raw_adc) &&
                    add_number(root, "percent", battery->percent) &&
                    cJSON_AddBoolToObject(root, "charging", battery->charging);
    if (!ok) {
        cJSON_Delete(root);
        return BOBE_PROTOCOL_MALFORMED;
    }
    return bobe_json_print(root, output, output_size);
}

bobe_protocol_result_t bobe_protocol_build_capture_state(
    char *output,
    size_t output_size,
    bool open,
    uint32_t stream_id,
    const char *lease_id,
    const char *turn_id,
    const char *reason
)
{
    if (stream_id == 0 || lease_id == NULL || lease_id[0] == '\0' ||
        turn_id == NULL || turn_id[0] == '\0') {
        return BOBE_PROTOCOL_INVALID_ARGUMENT;
    }
    cJSON *root = cJSON_CreateObject();
    if (root == NULL) {
        return BOBE_PROTOCOL_MALFORMED;
    }
    bool ok = add_string(
                  root,
                  "type",
                  open ? "audio.capture.open" : "audio.capture.close"
              ) &&
              add_number(root, "protocol_major", BOBE_PROTOCOL_MAJOR) &&
              add_number(root, "stream_id", stream_id) &&
              add_string(root, "lease_id", lease_id) &&
              add_string(root, "turn_id", turn_id);
    if (open) {
        ok = ok && add_string(root, "codec", "pcm_s16le") &&
             add_number(
                 root,
                 "sample_rate_hz",
                 BOBE_MIC_SAMPLE_RATE_HZ
             ) &&
             add_number(root, "channels", 1) &&
             add_number(
                 root,
                 "frame_duration_ms",
                 BOBE_AUDIO_FRAME_DURATION_MS
             );
    } else if (reason != NULL) {
        ok = ok && add_string(root, "reason", reason);
    }
    if (!ok) {
        cJSON_Delete(root);
        return BOBE_PROTOCOL_MALFORMED;
    }
    return bobe_json_print(root, output, output_size);
}

bobe_protocol_result_t bobe_protocol_build_capture_request(
    char *output,
    size_t output_size,
    const char *request_id,
    uint32_t stream_id,
    const char *trigger
)
{
    if (request_id == NULL || request_id[0] == '\0' ||
        stream_id == 0 || trigger == NULL || trigger[0] == '\0') {
        return BOBE_PROTOCOL_INVALID_ARGUMENT;
    }
    cJSON *root = cJSON_CreateObject();
    if (root == NULL) {
        return BOBE_PROTOCOL_MALFORMED;
    }
    const bool ok = add_string(root, "type", "capture.request") &&
                    add_number(root, "protocol_major", BOBE_PROTOCOL_MAJOR) &&
                    add_string(root, "request_id", request_id) &&
                    add_number(root, "stream_id", stream_id) &&
                    add_string(root, "trigger", trigger) &&
                    add_string(root, "codec", "pcm_s16le") &&
                    add_number(
                        root,
                        "sample_rate_hz",
                        BOBE_MIC_SAMPLE_RATE_HZ
                    ) &&
                    add_number(root, "channels", 1) &&
                    add_number(
                        root,
                        "frame_duration_ms",
                        BOBE_AUDIO_FRAME_DURATION_MS
                    );
    if (!ok) {
        cJSON_Delete(root);
        return BOBE_PROTOCOL_MALFORMED;
    }
    return bobe_json_print(root, output, output_size);
}

bobe_protocol_result_t bobe_protocol_build_capture_cancel(
    char *output,
    size_t output_size,
    const char *request_id,
    uint32_t stream_id
)
{
    if (request_id == NULL || request_id[0] == '\0' || stream_id == 0) {
        return BOBE_PROTOCOL_INVALID_ARGUMENT;
    }
    cJSON *root = cJSON_CreateObject();
    if (root == NULL) {
        return BOBE_PROTOCOL_MALFORMED;
    }
    const bool ok = add_string(root, "type", "capture.cancel") &&
                    add_number(root, "protocol_major", BOBE_PROTOCOL_MAJOR) &&
                    add_string(root, "request_id", request_id) &&
                    add_number(root, "stream_id", stream_id);
    if (!ok) {
        cJSON_Delete(root);
        return BOBE_PROTOCOL_MALFORMED;
    }
    return bobe_json_print(root, output, output_size);
}

bobe_protocol_result_t bobe_protocol_build_playback_ready(
    char *output,
    size_t output_size,
    uint32_t stream_id,
    const char *lease_id,
    const char *turn_id,
    uint32_t credit_ms
)
{
    if (stream_id == 0 || lease_id == NULL || lease_id[0] == '\0' ||
        turn_id == NULL || turn_id[0] == '\0' ||
        credit_ms < BOBE_AUDIO_FRAME_DURATION_MS ||
        credit_ms > BOBE_MAX_PLAYBACK_CREDIT_MS) {
        return BOBE_PROTOCOL_INVALID_ARGUMENT;
    }
    cJSON *root = cJSON_CreateObject();
    if (root == NULL) {
        return BOBE_PROTOCOL_MALFORMED;
    }
    const bool ok = add_string(root, "type", "audio.playback.ready") &&
                    add_number(root, "protocol_major", BOBE_PROTOCOL_MAJOR) &&
                    add_number(root, "stream_id", stream_id) &&
                    add_string(root, "lease_id", lease_id) &&
                    add_string(root, "turn_id", turn_id) &&
                    add_number(root, "credit_ms", credit_ms);
    if (!ok) {
        cJSON_Delete(root);
        return BOBE_PROTOCOL_MALFORMED;
    }
    return bobe_json_print(root, output, output_size);
}

bobe_protocol_result_t bobe_protocol_build_playback_ack(
    char *output,
    size_t output_size,
    uint32_t stream_id,
    const char *lease_id,
    const char *turn_id,
    uint64_t played_samples,
    uint32_t credit_ms
)
{
    if (stream_id == 0 || lease_id == NULL || lease_id[0] == '\0' ||
        turn_id == NULL || turn_id[0] == '\0' ||
        played_samples > UINT64_C(9007199254740991) ||
        credit_ms > BOBE_MAX_PLAYBACK_CREDIT_MS) {
        return BOBE_PROTOCOL_INVALID_ARGUMENT;
    }
    cJSON *root = cJSON_CreateObject();
    if (root == NULL) {
        return BOBE_PROTOCOL_MALFORMED;
    }
    const bool ok = add_string(root, "type", "audio.playback.ack") &&
                    add_number(root, "protocol_major", BOBE_PROTOCOL_MAJOR) &&
                    add_number(root, "stream_id", stream_id) &&
                    add_string(root, "lease_id", lease_id) &&
                    add_string(root, "turn_id", turn_id) &&
                    add_number(root, "played_samples", (double)played_samples) &&
                    add_number(root, "credit_ms", credit_ms);
    if (!ok) {
        cJSON_Delete(root);
        return BOBE_PROTOCOL_MALFORMED;
    }
    return bobe_json_print(root, output, output_size);
}

bobe_protocol_result_t bobe_protocol_build_playback_drained(
    char *output,
    size_t output_size,
    uint32_t stream_id,
    const char *lease_id,
    const char *turn_id
)
{
    if (stream_id == 0 || lease_id == NULL || lease_id[0] == '\0' ||
        turn_id == NULL || turn_id[0] == '\0') {
        return BOBE_PROTOCOL_INVALID_ARGUMENT;
    }
    cJSON *root = cJSON_CreateObject();
    if (root == NULL) {
        return BOBE_PROTOCOL_MALFORMED;
    }
    const bool ok = add_string(root, "type", "audio.playback.drained") &&
                    add_number(root, "protocol_major", BOBE_PROTOCOL_MAJOR) &&
                    add_number(root, "stream_id", stream_id) &&
                    add_string(root, "lease_id", lease_id) &&
                    add_string(root, "turn_id", turn_id);
    if (!ok) {
        cJSON_Delete(root);
        return BOBE_PROTOCOL_MALFORMED;
    }
    return bobe_json_print(root, output, output_size);
}

bobe_protocol_result_t bobe_protocol_build_playback_failed(
    char *output,
    size_t output_size,
    uint32_t stream_id,
    const char *lease_id,
    const char *turn_id,
    const char *reason
)
{
    if (stream_id == 0 || lease_id == NULL || lease_id[0] == '\0' ||
        turn_id == NULL || turn_id[0] == '\0' ||
        reason == NULL || reason[0] == '\0') {
        return BOBE_PROTOCOL_INVALID_ARGUMENT;
    }
    cJSON *root = cJSON_CreateObject();
    if (root == NULL) {
        return BOBE_PROTOCOL_MALFORMED;
    }
    const bool ok = add_string(root, "type", "audio.playback.failed") &&
                    add_number(root, "protocol_major", BOBE_PROTOCOL_MAJOR) &&
                    add_number(root, "stream_id", stream_id) &&
                    add_string(root, "lease_id", lease_id) &&
                    add_string(root, "turn_id", turn_id) &&
                    add_string(root, "reason", reason);
    if (!ok) {
        cJSON_Delete(root);
        return BOBE_PROTOCOL_MALFORMED;
    }
    return bobe_json_print(root, output, output_size);
}

bobe_protocol_result_t bobe_protocol_build_update_state(
    char *output,
    size_t output_size,
    const char *phase,
    uint8_t progress_percent,
    const char *reason
)
{
    if (phase == NULL || phase[0] == '\0' || progress_percent > 100 ||
        (reason != NULL && reason[0] == '\0')) {
        return BOBE_PROTOCOL_INVALID_ARGUMENT;
    }
    cJSON *root = cJSON_CreateObject();
    if (root == NULL) {
        return BOBE_PROTOCOL_MALFORMED;
    }
    bool ok = add_string(root, "type", "update.state") &&
              add_number(root, "protocol_major", BOBE_PROTOCOL_MAJOR) &&
              add_string(root, "phase", phase) &&
              add_number(root, "progress_percent", progress_percent);
    if (reason != NULL) {
        ok = ok && add_string(root, "reason", reason);
    }
    if (!ok) {
        cJSON_Delete(root);
        return BOBE_PROTOCOL_MALFORMED;
    }
    return bobe_json_print(root, output, output_size);
}

bobe_protocol_result_t bobe_protocol_parse_control(
    const char *input,
    size_t input_size,
    bobe_control_message_t *message
)
{
    if (input == NULL || message == NULL) {
        return BOBE_PROTOCOL_INVALID_ARGUMENT;
    }
    memset(message, 0, sizeof(*message));
    if (input_size == 0 || memchr(input, '\0', input_size) != NULL ||
        contains_nul_escape(input, input_size)) {
        return BOBE_PROTOCOL_MALFORMED;
    }
    if (input_size > BOBE_CONTROL_MAX_BYTES) {
        return BOBE_PROTOCOL_TOO_LARGE;
    }
    if (!bobe_json_within_limits(input, input_size)) {
        return BOBE_PROTOCOL_MALFORMED;
    }

    const char *parse_end = NULL;
    cJSON *root = cJSON_ParseWithLengthOpts(
        input,
        input_size,
        &parse_end,
        false
    );
    if (root == NULL || parse_end == NULL || !cJSON_IsObject(root) ||
        has_duplicate_keys(root)) {
        cJSON_Delete(root);
        return BOBE_PROTOCOL_MALFORMED;
    }
    const char *input_end = input + input_size;
    while (parse_end < input_end &&
           isspace((unsigned char)*parse_end)) {
        ++parse_end;
    }
    if (parse_end != input_end) {
        cJSON_Delete(root);
        return BOBE_PROTOCOL_MALFORMED;
    }
    const cJSON *type = cJSON_GetObjectItemCaseSensitive(root, "type");
    if (!cJSON_IsString(type) || type->valuestring == NULL ||
        type->valuestring[0] == '\0') {
        cJSON_Delete(root);
        return BOBE_PROTOCOL_MALFORMED;
    }
    uint32_t protocol_major = 0;
    if (!read_u32_field(root, "protocol_major", &protocol_major)) {
        cJSON_Delete(root);
        return BOBE_PROTOCOL_MALFORMED;
    }
    if (protocol_major != BOBE_PROTOCOL_MAJOR) {
        cJSON_Delete(root);
        return BOBE_PROTOCOL_VERSION_MISMATCH;
    }

    bobe_protocol_result_t result = BOBE_PROTOCOL_OK;

    if (strcmp(type->valuestring, "welcome") == 0) {
        message->kind = BOBE_CONTROL_WELCOME;
        uint32_t protocol_minor = 0;
        if (!read_u32_field(root, "protocol_minor", &protocol_minor) ||
            protocol_minor < BOBE_PROTOCOL_MIN_MINOR || protocol_minor > BOBE_PROTOCOL_MINOR ||
            !copy_nonempty_string_field(
                root,
                "session_id",
                message->data.welcome.session_id,
                sizeof(message->data.welcome.session_id)
            ) ||
            !read_u32_field(
                root,
                "heartbeat_interval_ms",
                &message->data.welcome.heartbeat_interval_ms
            ) ||
            !read_u64_field(
                root,
                "controller_epoch",
                &message->data.welcome.controller_epoch
            )) {
            result = BOBE_PROTOCOL_MALFORMED;
        } else if (message->data.welcome.heartbeat_interval_ms < 1000 ||
                   message->data.welcome.heartbeat_interval_ms > 60000 ||
                   message->data.welcome.controller_epoch == 0) {
            result = BOBE_PROTOCOL_RANGE_ERROR;
        } else {
            message->data.welcome.protocol_minor =
                (uint8_t)protocol_minor;
        }
    } else if (strcmp(type->valuestring, "input.result") == 0) {
        message->kind = BOBE_CONTROL_INPUT_RESULT;
        const cJSON *code = cJSON_GetObjectItemCaseSensitive(root, "code");
        const cJSON *replayed = cJSON_GetObjectItemCaseSensitive(root, "replayed");
        if (!copy_string_field(root, "request_id", message->data.input_result.request_id,
                               sizeof(message->data.input_result.request_id)) ||
            !bobe_uuid_valid(message->data.input_result.request_id) ||
            !copy_string_field(root, "status", message->data.input_result.status,
                               sizeof(message->data.input_result.status)) ||
            (code != NULL && !copy_string_field(root, "code", message->data.input_result.code,
                                               sizeof(message->data.input_result.code))) ||
            (replayed != NULL && !read_bool_field(root, "replayed", &message->data.input_result.replayed))) {
            result = BOBE_PROTOCOL_MALFORMED;
        } else {
            const char *status = message->data.input_result.status;
            if (strcmp(status, "accepted") != 0 && strcmp(status, "running") != 0 &&
                strcmp(status, "completed") != 0 && strcmp(status, "rejected") != 0 &&
                strcmp(status, "cancelled") != 0 && strcmp(status, "failed") != 0) {
                result = BOBE_PROTOCOL_RANGE_ERROR;
            }
        }
    } else if (strcmp(type->valuestring, "display.text") == 0) {
        message->kind = BOBE_CONTROL_DISPLAY_TEXT;
        if (!copy_string_field(root, "request_id", message->data.display_text.request_id,
                               sizeof(message->data.display_text.request_id)) ||
            !bobe_uuid_valid(message->data.display_text.request_id) ||
            !copy_string_field(root, "text", message->data.display_text.text,
                               sizeof(message->data.display_text.text)) ||
            !read_u32_field(root, "sequence", &message->data.display_text.sequence) ||
            !read_bool_field(root, "clear", &message->data.display_text.clear) ||
            !read_bool_field(root, "final", &message->data.display_text.final)) {
            result = BOBE_PROTOCOL_MALFORMED;
        }
    } else if (strcmp(type->valuestring, "sensor.ack") == 0) {
        message->kind = BOBE_CONTROL_SENSOR_ACK;
        if (!copy_string_field(root, "name", message->data.sensor_ack.name,
                               sizeof(message->data.sensor_ack.name)) ||
            !bobe_name_valid(message->data.sensor_ack.name, BOBE_SENSOR_NAME_MAX_BYTES) ||
            !read_u32_field(root, "sequence", &message->data.sensor_ack.sequence)) {
            result = BOBE_PROTOCOL_MALFORMED;
        }
    } else if (strcmp(type->valuestring, "capture.granted") == 0) {
        message->kind = BOBE_CONTROL_CAPTURE_GRANTED;
        uint64_t daemon_lease_ttl_ms = 0;
        if (!read_u32_field(
                root,
                "stream_id",
                &message->data.capture.stream_id
            ) ||
            !read_u64_field(
                root,
                "lease_ttl_ms",
                &daemon_lease_ttl_ms
            ) ||
            !copy_nonempty_string_field(
                root,
                "request_id",
                message->data.capture.request_id,
                sizeof(message->data.capture.request_id)
            ) ||
            !copy_nonempty_string_field(
                root,
                "lease_id",
                message->data.capture.lease_id,
                sizeof(message->data.capture.lease_id)
            ) ||
            !copy_nonempty_string_field(
                root,
                "turn_id",
                message->data.capture.turn_id,
                sizeof(message->data.capture.turn_id)
            )) {
            result = BOBE_PROTOCOL_MALFORMED;
        } else if (daemon_lease_ttl_ms < 1000 ||
                   daemon_lease_ttl_ms > BOBE_MAX_LEASE_LIFETIME_MS ||
                   message->data.capture.stream_id == 0) {
            result = BOBE_PROTOCOL_RANGE_ERROR;
        }
    } else if (strcmp(type->valuestring, "capture.denied") == 0) {
        message->kind = BOBE_CONTROL_CAPTURE_DENIED;
        if (!read_u32_field(
                root,
                "stream_id",
                &message->data.capture.stream_id
            ) ||
            !copy_nonempty_string_field(
                root,
                "request_id",
                message->data.capture.request_id,
                sizeof(message->data.capture.request_id)
            ) ||
            !copy_nonempty_string_field(
                root,
                "reason",
                message->data.capture.reason,
                sizeof(message->data.capture.reason)
            )) {
            result = BOBE_PROTOCOL_MALFORMED;
        } else if (message->data.capture.stream_id == 0) {
            result = BOBE_PROTOCOL_RANGE_ERROR;
        }
    } else if (strcmp(type->valuestring, "face.set") == 0) {
        message->kind = BOBE_CONTROL_FACE_SET;
        message->data.face.accent_rgb = BOBE_FACE_THEME_ACCENT_RGB;
        char expression[32];
        if (!copy_string_field(root, "expression", expression, sizeof(expression)) ||
            !parse_face_expression(expression, &message->data.face.expression) ||
            !read_u64_field(
                root,
                "state_version",
                &message->data.face.state_version
            )) {
            result = BOBE_PROTOCOL_MALFORMED;
        } else if (message->data.face.state_version == 0) {
            result = BOBE_PROTOCOL_RANGE_ERROR;
        }
        if (result == BOBE_PROTOCOL_OK &&
            (!copy_optional_nonempty_string_field(
                 root,
                 "lease_id",
                 message->data.face.lease_id,
                 sizeof(message->data.face.lease_id)
             ) ||
             !copy_optional_nonempty_string_field(
                 root,
                 "turn_id",
                 message->data.face.turn_id,
                 sizeof(message->data.face.turn_id)
             ) ||
             ((message->data.face.lease_id[0] == '\0') !=
              (message->data.face.turn_id[0] == '\0')))) {
            result = BOBE_PROTOCOL_MALFORMED;
        }
        const cJSON *caption =
            cJSON_GetObjectItemCaseSensitive(root, "caption");
        if (result == BOBE_PROTOCOL_OK && caption != NULL) {
            if (!cJSON_IsString(caption) || caption->valuestring == NULL ||
                strlen(caption->valuestring) >=
                    sizeof(message->data.face.caption)) {
                result = BOBE_PROTOCOL_MALFORMED;
            } else {
                strcpy(message->data.face.caption, caption->valuestring);
            }
        }
        uint32_t value = 0;
        if (!read_optional_u32_field(root, "accent_rgb", &value)) {
            result = BOBE_PROTOCOL_MALFORMED;
        } else if (cJSON_GetObjectItemCaseSensitive(root, "accent_rgb") !=
                   NULL) {
            if (value > 0x00FFFFFFU) {
                result = BOBE_PROTOCOL_RANGE_ERROR;
            } else {
                message->data.face.accent_rgb = value;
            }
        }
        if (!read_optional_u32_field(root, "ttl_ms", &value)) {
            result = BOBE_PROTOCOL_MALFORMED;
        } else if (cJSON_GetObjectItemCaseSensitive(root, "ttl_ms") != NULL) {
            if (value > 600000) {
                result = BOBE_PROTOCOL_RANGE_ERROR;
            } else {
                message->data.face.ttl_ms = value;
            }
        }
    } else if (strcmp(type->valuestring, "theme.set") == 0) {
        message->kind = BOBE_CONTROL_THEME_SET;
        char theme_id[BOBE_FACE_THEME_ID_MAX_BYTES];
        if (!copy_string_field(root, "theme_id", theme_id, sizeof(theme_id)) ||
            !bobe_face_theme_parse(theme_id, &message->data.theme.theme) ||
            !read_u64_field(
                root,
                "state_version",
                &message->data.theme.state_version
            )) {
            result = BOBE_PROTOCOL_MALFORMED;
        } else if (message->data.theme.state_version == 0) {
            result = BOBE_PROTOCOL_RANGE_ERROR;
        }
    } else if (strcmp(type->valuestring, "audio.playback.open") == 0) {
        message->kind = BOBE_CONTROL_PLAYBACK_OPEN;
        uint32_t channels = 0;
        uint32_t frame_ms = 0;
        char codec[24];
        if (!copy_string_field(root, "codec", codec, sizeof(codec)) ||
            strcmp(codec, "pcm_s16le") != 0 ||
            !read_u32_field(root, "stream_id", &message->data.playback_open.stream_id) ||
            !read_u32_field(
                root,
                "sample_rate_hz",
                &message->data.playback_open.sample_rate_hz
            ) ||
            !read_u32_field(root, "channels", &channels) ||
            !read_u32_field(root, "frame_duration_ms", &frame_ms) ||
            !copy_nonempty_string_field(
                root,
                "lease_id",
                message->data.playback_open.lease_id,
                sizeof(message->data.playback_open.lease_id)
            ) ||
            !copy_nonempty_string_field(
                root,
                "turn_id",
                message->data.playback_open.turn_id,
                sizeof(message->data.playback_open.turn_id)
            ) ||
            message->data.playback_open.stream_id == 0 ||
            channels != 1 ||
            frame_ms != BOBE_AUDIO_FRAME_DURATION_MS ||
            message->data.playback_open.sample_rate_hz !=
                BOBE_SPEAKER_SAMPLE_RATE_HZ) {
            result = BOBE_PROTOCOL_RANGE_ERROR;
        } else {
            message->data.playback_open.channels = (uint8_t)channels;
            message->data.playback_open.frame_duration_ms = (uint16_t)frame_ms;
        }
    } else if (strcmp(type->valuestring, "audio.playback.close") == 0) {
        message->kind = BOBE_CONTROL_PLAYBACK_CLOSE;
        if (!read_u32_field(
                root,
                "stream_id",
                &message->data.playback_close.stream_id
            ) ||
            !copy_nonempty_string_field(
                root,
                "lease_id",
                message->data.playback_close.lease_id,
                sizeof(message->data.playback_close.lease_id)
            ) ||
            !copy_nonempty_string_field(
                root,
                "turn_id",
                message->data.playback_close.turn_id,
                sizeof(message->data.playback_close.turn_id)
            ) ||
            message->data.playback_close.stream_id == 0) {
            result = BOBE_PROTOCOL_MALFORMED;
        }
    } else if (strcmp(type->valuestring, "turn.state") == 0) {
        message->kind = BOBE_CONTROL_TURN_STATE;
        char state[32];
        if (!copy_string_field(root, "state", state, sizeof(state)) ||
            !parse_turn_state(state, &message->data.turn.state) ||
            !copy_nonempty_string_field(
                root,
                "lease_id",
                message->data.turn.lease_id,
                sizeof(message->data.turn.lease_id)
            ) ||
            !copy_nonempty_string_field(
                root,
                "turn_id",
                message->data.turn.turn_id,
                sizeof(message->data.turn.turn_id)
            )) {
            result = BOBE_PROTOCOL_MALFORMED;
        }
    } else if (strcmp(type->valuestring, "error") == 0) {
        message->kind = BOBE_CONTROL_ERROR;
        if (!copy_nonempty_string_field(
                root,
                "code",
                message->data.error.code,
                sizeof(message->data.error.code)
            ) ||
            !copy_string_field(
                root,
                "message",
                message->data.error.message,
                sizeof(message->data.error.message)
            )) {
            result = BOBE_PROTOCOL_MALFORMED;
        }
    } else {
        message->kind = BOBE_CONTROL_UNKNOWN;
        result = BOBE_PROTOCOL_UNSUPPORTED;
    }

    cJSON_Delete(root);
    return result;
}
