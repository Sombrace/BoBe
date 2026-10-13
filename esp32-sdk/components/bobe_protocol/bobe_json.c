#include "bobe_json.h"

#include <stdint.h>
#include <limits.h>
#include <string.h>

bool bobe_utf8_valid(const char *input, size_t length)
{
    if (input == NULL) {
        return false;
    }
    for (size_t index = 0; index < length;) {
        const uint8_t first = (uint8_t)input[index++];
        if (first < 0x80U) {
            if (first == 0) return false;
            continue;
        }
        const unsigned continuation = first >= 0xC2U && first <= 0xDFU ? 1U :
            first >= 0xE0U && first <= 0xEFU ? 2U :
            first >= 0xF0U && first <= 0xF4U ? 3U : 0U;
        if (continuation == 0 || continuation > length - index) return false;
        const uint8_t second = (uint8_t)input[index];
        if ((first == 0xE0U && second < 0xA0U) ||
            (first == 0xEDU && second >= 0xA0U) ||
            (first == 0xF0U && second < 0x90U) ||
            (first == 0xF4U && second >= 0x90U)) return false;
        for (unsigned count = 0; count < continuation; ++count) {
            if (((uint8_t)input[index++] & 0xC0U) != 0x80U) return false;
        }
    }
    return true;
}

bool bobe_json_within_limits(const char *input, size_t length)
{
    if (length == 0 || length > BOBE_CONTROL_MAX_BYTES || !bobe_utf8_valid(input, length)) {
        return false;
    }
    unsigned depth = 0;
    bool quoted = false;
    bool escaped = false;
    for (size_t index = 0; index < length; ++index) {
        const char byte = input[index];
        if (quoted) {
            if (escaped) escaped = false;
            else if (byte == '\\') escaped = true;
            else if (byte == '"') quoted = false;
            continue;
        }
        if (byte == '"') quoted = true;
        else if (byte == '{' || byte == '[') {
            if (++depth > 16U) return false;
        } else if (byte == '}' || byte == ']') {
            if (depth == 0) return false;
            --depth;
        }
    }
    return !quoted && depth == 0;
}

bool bobe_uuid_valid(const char *value)
{
    if (value == NULL || strlen(value) != 36U) return false;
    for (size_t index = 0; index < 36U; ++index) {
        const char byte = value[index];
        if (index == 8 || index == 13 || index == 18 || index == 23) {
            if (byte != '-') return false;
        } else if (!((byte >= '0' && byte <= '9') ||
                     (byte >= 'a' && byte <= 'f') ||
                     (byte >= 'A' && byte <= 'F'))) return false;
    }
    return true;
}

bool bobe_name_valid(const char *value, size_t max_length)
{
    if (value == NULL || value[0] == '\0' || strlen(value) > max_length) return false;
    for (size_t index = 0; value[index] != '\0'; ++index) {
        const char byte = value[index];
        if (!((byte >= 'a' && byte <= 'z') || (byte >= 'A' && byte <= 'Z') ||
              (byte >= '0' && byte <= '9') || byte == '_' || byte == '-' || byte == '.')) {
            return false;
        }
    }
    return true;
}

bobe_protocol_result_t bobe_json_print(cJSON *root, char *output, size_t capacity)
{
    if (root == NULL || output == NULL || capacity == 0 || capacity > INT_MAX) {
        cJSON_Delete(root);
        return BOBE_PROTOCOL_INVALID_ARGUMENT;
    }
    const bool printed = cJSON_PrintPreallocated(root, output, (int)capacity, false);
    cJSON_Delete(root);
    if (!printed || strlen(output) > BOBE_CONTROL_MAX_BYTES) return BOBE_PROTOCOL_TOO_LARGE;
    return BOBE_PROTOCOL_OK;
}
