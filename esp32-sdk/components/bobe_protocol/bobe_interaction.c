#include "bobe_protocol.h"
#include "bobe_json.h"
#include "cJSON.h"

#include <math.h>
#include <string.h>

uint8_t bobe_capabilities_protocol_minor(const bobe_capabilities_t *capabilities)
{
    return capabilities != NULL &&
        (capabilities->text_input || capabilities->text_output ||
         capabilities->actions || capabilities->sensors ||
         !capabilities->microphone || !capabilities->speaker)
        ? 2U : 1U;
}

static cJSON *message(const char *type)
{
    cJSON *root = cJSON_CreateObject();
    if (root == NULL || cJSON_AddStringToObject(root, "type", type) == NULL ||
        cJSON_AddNumberToObject(root, "protocol_major", BOBE_PROTOCOL_MAJOR) == NULL) {
        cJSON_Delete(root);
        return NULL;
    }
    return root;
}

static bobe_protocol_result_t finish(cJSON *root, bool ok, char *output, size_t capacity)
{
    if (root == NULL || !ok) {
        cJSON_Delete(root);
        return BOBE_PROTOCOL_MALFORMED;
    }
    return bobe_json_print(root, output, capacity);
}

bobe_protocol_result_t bobe_protocol_build_text_input(
    char *output, size_t capacity, const char *request_id, const char *text
)
{
    if (!bobe_uuid_valid(request_id) || text == NULL || text[0] == '\0' ||
        strlen(text) > BOBE_TEXT_INPUT_MAX_BYTES || !bobe_utf8_valid(text, strlen(text))) {
        return BOBE_PROTOCOL_INVALID_ARGUMENT;
    }
    cJSON *root = message("input.text");
    const bool ok = root != NULL && cJSON_AddStringToObject(root, "request_id", request_id) != NULL &&
        cJSON_AddStringToObject(root, "text", text) != NULL;
    return finish(root, ok, output, capacity);
}

bobe_protocol_result_t bobe_protocol_build_action(
    char *output, size_t capacity, const char *request_id, const char *action, const char *label
)
{
    if (!bobe_uuid_valid(request_id) || !bobe_name_valid(action, BOBE_ACTION_ID_MAX_BYTES) ||
        label == NULL || strlen(label) > BOBE_ACTION_LABEL_MAX_BYTES ||
        !bobe_utf8_valid(label, strlen(label))) return BOBE_PROTOCOL_INVALID_ARGUMENT;
    cJSON *root = message("input.action");
    const bool ok = root != NULL && cJSON_AddStringToObject(root, "request_id", request_id) != NULL &&
        cJSON_AddStringToObject(root, "action", action) != NULL &&
        cJSON_AddStringToObject(root, "label", label) != NULL;
    return finish(root, ok, output, capacity);
}

bobe_protocol_result_t bobe_protocol_build_input_cancel(
    char *output, size_t capacity, const char *request_id
)
{
    if (!bobe_uuid_valid(request_id)) return BOBE_PROTOCOL_INVALID_ARGUMENT;
    cJSON *root = message("input.cancel");
    const bool ok = root != NULL && cJSON_AddStringToObject(root, "request_id", request_id) != NULL;
    return finish(root, ok, output, capacity);
}

static cJSON *sensor(const char *name, uint32_t sequence)
{
    cJSON *root = message("sensor.sample");
    if (root == NULL || cJSON_AddStringToObject(root, "name", name) == NULL ||
        cJSON_AddNumberToObject(root, "sequence", sequence) == NULL) {
        cJSON_Delete(root);
        return NULL;
    }
    return root;
}

bobe_protocol_result_t bobe_protocol_build_sensor_number(
    char *output, size_t capacity, const char *name, uint32_t sequence, double value, const char *unit
)
{
    if (!bobe_name_valid(name, BOBE_SENSOR_NAME_MAX_BYTES) || !isfinite(value) ||
        unit == NULL || strlen(unit) > BOBE_SENSOR_UNIT_MAX_BYTES ||
        !bobe_utf8_valid(unit, strlen(unit))) return BOBE_PROTOCOL_INVALID_ARGUMENT;
    for (const unsigned char *byte = (const unsigned char *)unit; *byte != 0; ++byte) {
        if (*byte < 32U || *byte == 127U) return BOBE_PROTOCOL_INVALID_ARGUMENT;
    }
    cJSON *root = sensor(name, sequence);
    const bool ok = root != NULL && cJSON_AddNumberToObject(root, "value", value) != NULL &&
        cJSON_AddStringToObject(root, "unit", unit) != NULL;
    return finish(root, ok, output, capacity);
}

bobe_protocol_result_t bobe_protocol_build_sensor_boolean(
    char *output, size_t capacity, const char *name, uint32_t sequence, bool value
)
{
    if (!bobe_name_valid(name, BOBE_SENSOR_NAME_MAX_BYTES)) return BOBE_PROTOCOL_INVALID_ARGUMENT;
    cJSON *root = sensor(name, sequence);
    const bool ok = root != NULL && cJSON_AddBoolToObject(root, "value", value) != NULL;
    return finish(root, ok, output, capacity);
}
