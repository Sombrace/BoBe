#include "bobe_update_manifest.h"

#include <string.h>

#define BOBE_UPDATE_SCRATCH_MAX (BOBE_UPDATE_URL_MAX + 1U)
#define BOBE_UPDATE_URL_PREFIX "https://"

typedef enum {
    BOBE_FIELD_SCHEMA = 0,
    BOBE_FIELD_DEVICE_MODEL,
    BOBE_FIELD_VERSION,
    BOBE_FIELD_BUILD_NUMBER,
    BOBE_FIELD_IMAGE_SIZE,
    BOBE_FIELD_IMAGE_SHA256,
    BOBE_FIELD_URL,
    BOBE_FIELD_KEY_ID,
    BOBE_FIELD_NOT_BEFORE,
    BOBE_FIELD_EXPIRES_AT,
    BOBE_FIELD_COUNT,
} bobe_update_field_t;

static const char *const field_names[BOBE_FIELD_COUNT] = {
    "schema", "device_model", "version", "build_number", "image_size",
    "image_sha256", "url", "key_id", "not_before", "expires_at",
};

typedef struct {
    const char *data;
    size_t length;
    size_t cursor;
} scanner_t;

typedef struct {
    char text[BOBE_UPDATE_SCRATCH_MAX + 1U];
    size_t length;
} scratch_t;

static bool peek(const scanner_t *scanner, char *out)
{
    if (scanner->cursor >= scanner->length) {
        return false;
    }
    *out = scanner->data[scanner->cursor];
    return true;
}

static void skip_whitespace(scanner_t *scanner)
{
    while (scanner->cursor < scanner->length) {
        const char value = scanner->data[scanner->cursor];
        if (value != ' ' && value != '\t' && value != '\n' && value != '\r') {
            break;
        }
        ++scanner->cursor;
    }
}

static bool match(scanner_t *scanner, char expected)
{
    char value = '\0';
    if (!peek(scanner, &value) || value != expected) {
        return false;
    }
    ++scanner->cursor;
    return true;
}

static bool hex_value(char value, uint8_t *nibble)
{
    if (value >= '0' && value <= '9') {
        *nibble = (uint8_t)(value - '0');
        return true;
    }
    if (value >= 'a' && value <= 'f') {
        *nibble = (uint8_t)(10 + value - 'a');
        return true;
    }
    return false;
}

static bool escape_hex_value(char value, uint8_t *nibble)
{
    if (hex_value(value, nibble)) {
        return true;
    }
    if (value >= 'A' && value <= 'F') {
        *nibble = (uint8_t)(10 + value - 'A');
        return true;
    }
    return false;
}

static bobe_update_manifest_result_t scan_string(
    scanner_t *scanner,
    scratch_t *scratch
)
{
    if (!match(scanner, '"')) {
        return BOBE_UPDATE_MANIFEST_WRONG_TYPE;
    }
    scratch->length = 0;
    for (;;) {
        if (scanner->cursor >= scanner->length) {
            return BOBE_UPDATE_MANIFEST_MALFORMED;
        }
        const char value = scanner->data[scanner->cursor++];
        if (value == '"') {
            scratch->text[scratch->length] = '\0';
            return BOBE_UPDATE_MANIFEST_OK;
        }
        char decoded = value;
        if (value == '\\') {
            if (scanner->cursor >= scanner->length) {
                return BOBE_UPDATE_MANIFEST_MALFORMED;
            }
            const char escape = scanner->data[scanner->cursor++];
            if (escape == '"' || escape == '\\' || escape == '/') {
                decoded = escape;
            } else if (escape == 'u') {
                if (scanner->cursor + 4U > scanner->length) {
                    return BOBE_UPDATE_MANIFEST_MALFORMED;
                }
                uint32_t code_point = 0;
                for (size_t index = 0; index < 4U; ++index) {
                    uint8_t nibble = 0;
                    if (!escape_hex_value(
                            scanner->data[scanner->cursor + index],
                            &nibble
                        )) {
                        return BOBE_UPDATE_MANIFEST_MALFORMED;
                    }
                    code_point = (code_point << 4) | nibble;
                }
                scanner->cursor += 4U;
                if (code_point == 0U) {
                    return BOBE_UPDATE_MANIFEST_EMBEDDED_NUL;
                }
                if (code_point < 0x20U || code_point > 0x7EU) {
                    return BOBE_UPDATE_MANIFEST_MALFORMED;
                }
                decoded = (char)code_point;
            } else {
                return BOBE_UPDATE_MANIFEST_MALFORMED;
            }
        } else {
            const unsigned char raw = (unsigned char)value;
            if (raw == 0U) {
                return BOBE_UPDATE_MANIFEST_EMBEDDED_NUL;
            }
            if (raw < 0x20U || raw > 0x7EU) {
                return BOBE_UPDATE_MANIFEST_MALFORMED;
            }
        }
        if (scratch->length >= BOBE_UPDATE_SCRATCH_MAX) {
            return BOBE_UPDATE_MANIFEST_TOO_LARGE;
        }
        scratch->text[scratch->length++] = decoded;
    }
}

static bobe_update_manifest_result_t scan_integer(
    scanner_t *scanner,
    int64_t *value
)
{
    char current = '\0';
    if (!peek(scanner, &current)) {
        return BOBE_UPDATE_MANIFEST_MALFORMED;
    }
    bool negative = false;
    if (current == '-') {
        negative = true;
        ++scanner->cursor;
        if (!peek(scanner, &current)) {
            return BOBE_UPDATE_MANIFEST_MALFORMED;
        }
    }
    if (current < '0' || current > '9') {
        return BOBE_UPDATE_MANIFEST_WRONG_TYPE;
    }
    const size_t start = scanner->cursor;
    int64_t magnitude = 0;
    while (scanner->cursor < scanner->length) {
        const char digit = scanner->data[scanner->cursor];
        if (digit < '0' || digit > '9') {
            break;
        }
        if (magnitude >
            (BOBE_UPDATE_SAFE_INTEGER_MAX - (digit - '0')) / 10) {
            return BOBE_UPDATE_MANIFEST_IMPRECISE_NUMBER;
        }
        magnitude = magnitude * 10 + digit - '0';
        ++scanner->cursor;
    }
    if (scanner->cursor - start > 1U && scanner->data[start] == '0') {
        return BOBE_UPDATE_MANIFEST_MALFORMED;
    }
    if (peek(scanner, &current) &&
        (current == '.' || current == 'e' || current == 'E')) {
        return BOBE_UPDATE_MANIFEST_IMPRECISE_NUMBER;
    }
    *value = negative ? -magnitude : magnitude;
    return BOBE_UPDATE_MANIFEST_OK;
}

static bobe_update_manifest_result_t copy_bounded(
    const scratch_t *scratch,
    size_t maximum,
    char *destination,
    bobe_update_manifest_result_t invalid
)
{
    if (scratch->length == 0U || scratch->length > maximum) {
        return invalid;
    }
    memcpy(destination, scratch->text, scratch->length + 1U);
    return BOBE_UPDATE_MANIFEST_OK;
}

static bobe_update_manifest_result_t decode_digest(
    const scratch_t *scratch,
    uint8_t digest[BOBE_UPDATE_SHA256_BYTES]
)
{
    if (scratch->length != BOBE_UPDATE_SHA256_HEX_LEN) {
        return BOBE_UPDATE_MANIFEST_DIGEST_INVALID;
    }
    for (size_t index = 0; index < BOBE_UPDATE_SHA256_BYTES; ++index) {
        uint8_t high = 0;
        uint8_t low = 0;
        if (!hex_value(scratch->text[index * 2U], &high) ||
            !hex_value(scratch->text[index * 2U + 1U], &low)) {
            return BOBE_UPDATE_MANIFEST_DIGEST_INVALID;
        }
        digest[index] = (uint8_t)((high << 4) | low);
    }
    return BOBE_UPDATE_MANIFEST_OK;
}

static bool version_is_printable(const char *version)
{
    for (const char *cursor = version; *cursor != '\0'; ++cursor) {
        const unsigned char raw = (unsigned char)*cursor;
        if (raw <= 0x20U || raw > 0x7EU) {
            return false;
        }
    }
    return true;
}

static bool field_is_string(bobe_update_field_t field)
{
    return field == BOBE_FIELD_DEVICE_MODEL || field == BOBE_FIELD_VERSION ||
           field == BOBE_FIELD_IMAGE_SHA256 || field == BOBE_FIELD_URL ||
           field == BOBE_FIELD_KEY_ID;
}

static bool lookup_field(const scratch_t *scratch, bobe_update_field_t *field)
{
    for (size_t index = 0; index < BOBE_FIELD_COUNT; ++index) {
        if (strcmp(scratch->text, field_names[index]) == 0) {
            *field = (bobe_update_field_t)index;
            return true;
        }
    }
    return false;
}

static bobe_update_manifest_result_t store_field(
    bobe_update_field_t field,
    const scratch_t *scratch,
    int64_t number,
    const bobe_update_manifest_context_t *context,
    bobe_update_manifest_t *manifest
)
{
    switch (field) {
    case BOBE_FIELD_SCHEMA:
        return number == BOBE_UPDATE_SCHEMA_VERSION
                   ? BOBE_UPDATE_MANIFEST_OK
                   : BOBE_UPDATE_MANIFEST_SCHEMA_UNSUPPORTED;
    case BOBE_FIELD_DEVICE_MODEL:
        return strcmp(scratch->text, context->device_model) == 0
                   ? BOBE_UPDATE_MANIFEST_OK
                   : BOBE_UPDATE_MANIFEST_MODEL_MISMATCH;
    case BOBE_FIELD_VERSION: {
        const bobe_update_manifest_result_t result = copy_bounded(
            scratch,
            BOBE_UPDATE_VERSION_MAX,
            manifest->version,
            BOBE_UPDATE_MANIFEST_VERSION_INVALID
        );
        return result == BOBE_UPDATE_MANIFEST_OK &&
                       !version_is_printable(manifest->version)
                   ? BOBE_UPDATE_MANIFEST_VERSION_INVALID
                   : result;
    }
    case BOBE_FIELD_BUILD_NUMBER:
        if (number <= 0 || number > UINT32_MAX) {
            return BOBE_UPDATE_MANIFEST_BUILD_INVALID;
        }
        manifest->build_number = (uint32_t)number;
        return BOBE_UPDATE_MANIFEST_OK;
    case BOBE_FIELD_IMAGE_SIZE:
        if (number <= 0 || number > context->max_image_size) {
            return BOBE_UPDATE_MANIFEST_IMAGE_SIZE_INVALID;
        }
        manifest->image_size = (uint32_t)number;
        return BOBE_UPDATE_MANIFEST_OK;
    case BOBE_FIELD_IMAGE_SHA256:
        return decode_digest(scratch, manifest->image_sha256);
    case BOBE_FIELD_URL: {
        const size_t prefix_length = strlen(BOBE_UPDATE_URL_PREFIX);
        if (scratch->length <= prefix_length ||
            strncmp(scratch->text, BOBE_UPDATE_URL_PREFIX, prefix_length) != 0) {
            return BOBE_UPDATE_MANIFEST_URL_INVALID;
        }
        return copy_bounded(
            scratch,
            BOBE_UPDATE_URL_MAX,
            manifest->url,
            BOBE_UPDATE_MANIFEST_URL_INVALID
        );
    }
    case BOBE_FIELD_KEY_ID:
        return copy_bounded(
            scratch,
            BOBE_UPDATE_KEY_ID_MAX,
            manifest->key_id,
            BOBE_UPDATE_MANIFEST_KEY_ID_INVALID
        );
    case BOBE_FIELD_NOT_BEFORE:
        if (number < 0) {
            return BOBE_UPDATE_MANIFEST_TIME_WINDOW_INVALID;
        }
        manifest->not_before = number;
        return BOBE_UPDATE_MANIFEST_OK;
    case BOBE_FIELD_EXPIRES_AT:
        if (number < 0) {
            return BOBE_UPDATE_MANIFEST_TIME_WINDOW_INVALID;
        }
        manifest->expires_at = number;
        return BOBE_UPDATE_MANIFEST_OK;
    case BOBE_FIELD_COUNT:
    default:
        return BOBE_UPDATE_MANIFEST_UNKNOWN_FIELD;
    }
}

bobe_update_manifest_result_t bobe_update_manifest_parse(
    const char *json,
    size_t length,
    const bobe_update_manifest_context_t *context,
    bobe_update_manifest_t *manifest
)
{
    if (json == NULL || context == NULL || manifest == NULL ||
        context->device_model == NULL || context->device_model[0] == '\0' ||
        context->max_image_size == 0) {
        return BOBE_UPDATE_MANIFEST_INVALID_ARGUMENT;
    }
    if (length == 0U) {
        return BOBE_UPDATE_MANIFEST_MALFORMED;
    }
    if (length > BOBE_UPDATE_MANIFEST_MAX_BYTES) {
        return BOBE_UPDATE_MANIFEST_TOO_LARGE;
    }

    bobe_update_manifest_t parsed;
    memset(&parsed, 0, sizeof(parsed));
    scanner_t scanner = {.data = json, .length = length};
    bool seen[BOBE_FIELD_COUNT] = {false};
    skip_whitespace(&scanner);
    if (!match(&scanner, '{')) {
        return BOBE_UPDATE_MANIFEST_MALFORMED;
    }
    skip_whitespace(&scanner);
    if (!match(&scanner, '}')) {
        for (;;) {
            scratch_t scratch;
            skip_whitespace(&scanner);
            bobe_update_manifest_result_t result = scan_string(&scanner, &scratch);
            if (result != BOBE_UPDATE_MANIFEST_OK) {
                return result == BOBE_UPDATE_MANIFEST_WRONG_TYPE
                           ? BOBE_UPDATE_MANIFEST_MALFORMED
                           : result;
            }
            bobe_update_field_t field = BOBE_FIELD_COUNT;
            if (!lookup_field(&scratch, &field)) {
                return BOBE_UPDATE_MANIFEST_UNKNOWN_FIELD;
            }
            if (seen[field]) {
                return BOBE_UPDATE_MANIFEST_DUPLICATE_FIELD;
            }
            seen[field] = true;
            skip_whitespace(&scanner);
            if (!match(&scanner, ':')) {
                return BOBE_UPDATE_MANIFEST_MALFORMED;
            }
            skip_whitespace(&scanner);
            int64_t number = 0;
            if (field_is_string(field)) {
                result = scan_string(&scanner, &scratch);
            } else {
                scratch.length = 0;
                scratch.text[0] = '\0';
                result = scan_integer(&scanner, &number);
            }
            if (result != BOBE_UPDATE_MANIFEST_OK) {
                return result;
            }
            result = store_field(field, &scratch, number, context, &parsed);
            if (result != BOBE_UPDATE_MANIFEST_OK) {
                return result;
            }
            skip_whitespace(&scanner);
            if (match(&scanner, ',')) {
                continue;
            }
            if (match(&scanner, '}')) {
                break;
            }
            return BOBE_UPDATE_MANIFEST_MALFORMED;
        }
    }
    skip_whitespace(&scanner);
    if (scanner.cursor != scanner.length) {
        return BOBE_UPDATE_MANIFEST_TRAILING_DATA;
    }
    for (size_t index = 0; index < BOBE_FIELD_COUNT; ++index) {
        if (!seen[index]) {
            return BOBE_UPDATE_MANIFEST_MISSING_FIELD;
        }
    }
    if (parsed.expires_at <= parsed.not_before) {
        return BOBE_UPDATE_MANIFEST_TIME_WINDOW_INVALID;
    }
    if (parsed.build_number <= context->current_build_number) {
        return BOBE_UPDATE_MANIFEST_NOT_AN_UPGRADE;
    }
    if (context->trusted_time_valid) {
        if (context->trusted_unix_time < parsed.not_before) {
            return BOBE_UPDATE_MANIFEST_NOT_YET_VALID;
        }
        if (context->trusted_unix_time >= parsed.expires_at) {
            return BOBE_UPDATE_MANIFEST_EXPIRED;
        }
    }
    *manifest = parsed;
    return BOBE_UPDATE_MANIFEST_OK;
}

const char *bobe_update_manifest_result_name(
    bobe_update_manifest_result_t result
)
{
    static const char *const names[] = {
        "ok", "invalid_argument", "too_large", "malformed",
        "trailing_data", "unknown_field", "duplicate_field",
        "missing_field", "wrong_type", "embedded_nul", "imprecise_number",
        "schema_unsupported", "model_mismatch", "version_invalid",
        "build_invalid", "image_size_invalid", "digest_invalid",
        "url_invalid", "key_id_invalid", "time_window_invalid",
        "not_yet_valid", "expired", "not_an_upgrade",
    };
    return result >= BOBE_UPDATE_MANIFEST_OK &&
                   (size_t)result < sizeof(names) / sizeof(names[0])
               ? names[result]
               : "unknown";
}
