#include "bobe_update_offer.h"
#include "bobe_json.h"

#include "cJSON.h"

#include <ctype.h>
#include <string.h>

static int base64_value(char value)
{
    if (value >= 'A' && value <= 'Z') {
        return value - 'A';
    }
    if (value >= 'a' && value <= 'z') {
        return 26 + value - 'a';
    }
    if (value >= '0' && value <= '9') {
        return 52 + value - '0';
    }
    return value == '+' ? 62 : value == '/' ? 63 : -1;
}

static bool decode_base64(
    const char *input,
    size_t length,
    unsigned char *output,
    size_t capacity,
    size_t *output_length
)
{
    if (length == 0U || length % 4U != 0U) {
        return false;
    }
    size_t written = 0;
    for (size_t offset = 0; offset < length; offset += 4U) {
        const bool final = offset + 4U == length;
        const bool pad2 = input[offset + 2U] == '=';
        const bool pad3 = input[offset + 3U] == '=';
        if ((!final && (pad2 || pad3)) || (pad2 && !pad3)) {
            return false;
        }
        const int a = base64_value(input[offset]);
        const int b = base64_value(input[offset + 1U]);
        const int c = pad2 ? 0 : base64_value(input[offset + 2U]);
        const int d = pad3 ? 0 : base64_value(input[offset + 3U]);
        if (a < 0 || b < 0 || c < 0 || d < 0 ||
            (pad2 && (b & 0x0F) != 0) ||
            (pad3 && !pad2 && (c & 0x03) != 0)) {
            return false;
        }
        const size_t count = pad2 ? 1U : pad3 ? 2U : 3U;
        if (written + count > capacity) {
            return false;
        }
        output[written++] = (unsigned char)((a << 2) | (b >> 4));
        if (count > 1U) {
            output[written++] = (unsigned char)((b << 4) | (c >> 2));
        }
        if (count > 2U) {
            output[written++] = (unsigned char)((c << 6) | d);
        }
    }
    *output_length = written;
    return true;
}

static bool duplicate_keys(const cJSON *root)
{
    for (const cJSON *item = root->child; item != NULL; item = item->next) {
        for (const cJSON *other = item->next; other != NULL; other = other->next) {
            if (item->string != NULL && other->string != NULL &&
                strcmp(item->string, other->string) == 0) {
                return true;
            }
        }
    }
    return false;
}

static bool exact_fields(const cJSON *root)
{
    static const char *const names[] = {
        "type",
        "protocol_major",
        "manifest",
        "signature",
    };
    size_t count = 0;
    for (const cJSON *item = root->child; item != NULL; item = item->next) {
        bool known = false;
        for (size_t index = 0; index < sizeof(names) / sizeof(names[0]); ++index) {
            if (item->string != NULL && strcmp(item->string, names[index]) == 0) {
                known = true;
                break;
            }
        }
        if (!known) {
            return false;
        }
        ++count;
    }
    return count == sizeof(names) / sizeof(names[0]);
}

static bool exact_protocol_major(const cJSON *root)
{
    const cJSON *const value =
        cJSON_GetObjectItemCaseSensitive(root, "protocol_major");
    return cJSON_IsNumber(value) && value->valuedouble == 1.0 &&
           value->valueint == 1;
}

bobe_update_offer_result_t bobe_update_offer_parse(
    const char *json,
    size_t length,
    const bobe_update_manifest_context_t *context,
    bobe_update_signature_verifier_t verify_signature,
    void *verifier_context,
    bobe_update_manifest_t *manifest,
    bobe_update_manifest_result_t *manifest_result
)
{
    if (json == NULL || context == NULL || verify_signature == NULL ||
        manifest == NULL || manifest_result == NULL) {
        return BOBE_UPDATE_OFFER_INVALID_ARGUMENT;
    }
    *manifest_result = BOBE_UPDATE_MANIFEST_INVALID_ARGUMENT;
    if (!bobe_json_within_limits(json, length)) {
        return BOBE_UPDATE_OFFER_MALFORMED;
    }

    const char *end = NULL;
    cJSON *const root = cJSON_ParseWithLengthOpts(json, length, &end, false);
    if (root == NULL || end == NULL || !cJSON_IsObject(root) ||
        duplicate_keys(root)) {
        cJSON_Delete(root);
        return BOBE_UPDATE_OFFER_MALFORMED;
    }
    while (end < json + length && isspace((unsigned char)*end)) {
        ++end;
    }
    const cJSON *const type = cJSON_GetObjectItemCaseSensitive(root, "type");
    if (!cJSON_IsString(type) || type->valuestring == NULL ||
        strcmp(type->valuestring, "update.offer") != 0) {
        cJSON_Delete(root);
        return BOBE_UPDATE_OFFER_NOT_OFFER;
    }
    if (!exact_fields(root) || end != json + length ||
        !exact_protocol_major(root)) {
        cJSON_Delete(root);
        return BOBE_UPDATE_OFFER_MALFORMED;
    }

    const cJSON *const raw_manifest =
        cJSON_GetObjectItemCaseSensitive(root, "manifest");
    const cJSON *const signature =
        cJSON_GetObjectItemCaseSensitive(root, "signature");
    if (!cJSON_IsString(raw_manifest) || raw_manifest->valuestring == NULL ||
        !cJSON_IsString(signature) || signature->valuestring == NULL) {
        cJSON_Delete(root);
        return BOBE_UPDATE_OFFER_MALFORMED;
    }
    const size_t manifest_length = strlen(raw_manifest->valuestring);
    const size_t signature_length = strlen(signature->valuestring);
    if (manifest_length == 0U ||
        manifest_length > BOBE_UPDATE_MANIFEST_MAX_BYTES ||
        signature_length == 0U ||
        signature_length > BOBE_UPDATE_SIGNATURE_BASE64_MAX) {
        cJSON_Delete(root);
        return BOBE_UPDATE_OFFER_MALFORMED;
    }

    unsigned char decoded_signature[96];
    size_t decoded_length = 0;
    if (!decode_base64(
            signature->valuestring,
            signature_length,
            decoded_signature,
            sizeof(decoded_signature),
            &decoded_length
        ) ||
        decoded_length == 0U) {
        cJSON_Delete(root);
        return BOBE_UPDATE_OFFER_SIGNATURE_INVALID;
    }

    if (!verify_signature(
            (const unsigned char *)raw_manifest->valuestring,
            manifest_length,
            decoded_signature,
            decoded_length,
            verifier_context
        )) {
        cJSON_Delete(root);
        return BOBE_UPDATE_OFFER_SIGNATURE_INVALID;
    }

    *manifest_result = bobe_update_manifest_parse(
        raw_manifest->valuestring,
        manifest_length,
        context,
        manifest
    );
    cJSON_Delete(root);
    return *manifest_result == BOBE_UPDATE_MANIFEST_OK
               ? BOBE_UPDATE_OFFER_OK
               : BOBE_UPDATE_OFFER_MANIFEST_INVALID;
}

const char *bobe_update_offer_result_name(bobe_update_offer_result_t result)
{
    static const char *const names[] = {
        "ok",
        "not_offer",
        "invalid_argument",
        "malformed",
        "signature_invalid",
        "manifest_invalid",
    };
    return result >= BOBE_UPDATE_OFFER_OK &&
                   (size_t)result < sizeof(names) / sizeof(names[0])
               ? names[result]
               : "unknown";
}
