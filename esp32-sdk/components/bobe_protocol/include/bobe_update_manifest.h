#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define BOBE_UPDATE_SCHEMA_VERSION 1
#define BOBE_UPDATE_VERSION_MAX 31U
#define BOBE_UPDATE_URL_MAX 256U
#define BOBE_UPDATE_KEY_ID_MAX 32U
#define BOBE_UPDATE_SHA256_HEX_LEN 64U
#define BOBE_UPDATE_SHA256_BYTES 32U
#define BOBE_UPDATE_SAFE_INTEGER_MAX INT64_C(9007199254740991)
#define BOBE_UPDATE_MANIFEST_MAX_BYTES 2048U

typedef enum {
    BOBE_UPDATE_MANIFEST_OK = 0,
    BOBE_UPDATE_MANIFEST_INVALID_ARGUMENT,
    BOBE_UPDATE_MANIFEST_TOO_LARGE,
    BOBE_UPDATE_MANIFEST_MALFORMED,
    BOBE_UPDATE_MANIFEST_TRAILING_DATA,
    BOBE_UPDATE_MANIFEST_UNKNOWN_FIELD,
    BOBE_UPDATE_MANIFEST_DUPLICATE_FIELD,
    BOBE_UPDATE_MANIFEST_MISSING_FIELD,
    BOBE_UPDATE_MANIFEST_WRONG_TYPE,
    BOBE_UPDATE_MANIFEST_EMBEDDED_NUL,
    BOBE_UPDATE_MANIFEST_IMPRECISE_NUMBER,
    BOBE_UPDATE_MANIFEST_SCHEMA_UNSUPPORTED,
    BOBE_UPDATE_MANIFEST_MODEL_MISMATCH,
    BOBE_UPDATE_MANIFEST_VERSION_INVALID,
    BOBE_UPDATE_MANIFEST_BUILD_INVALID,
    BOBE_UPDATE_MANIFEST_IMAGE_SIZE_INVALID,
    BOBE_UPDATE_MANIFEST_DIGEST_INVALID,
    BOBE_UPDATE_MANIFEST_URL_INVALID,
    BOBE_UPDATE_MANIFEST_KEY_ID_INVALID,
    BOBE_UPDATE_MANIFEST_TIME_WINDOW_INVALID,
    BOBE_UPDATE_MANIFEST_NOT_YET_VALID,
    BOBE_UPDATE_MANIFEST_EXPIRED,
    BOBE_UPDATE_MANIFEST_NOT_AN_UPGRADE,
} bobe_update_manifest_result_t;

typedef struct {
    char version[BOBE_UPDATE_VERSION_MAX + 1U];
    uint32_t build_number;
    uint32_t image_size;
    uint8_t image_sha256[BOBE_UPDATE_SHA256_BYTES];
    char url[BOBE_UPDATE_URL_MAX + 1U];
    char key_id[BOBE_UPDATE_KEY_ID_MAX + 1U];
    int64_t not_before;
    int64_t expires_at;
} bobe_update_manifest_t;

typedef struct {
    const char *device_model;
    uint32_t max_image_size;
    uint32_t current_build_number;
    bool trusted_time_valid;
    int64_t trusted_unix_time;
} bobe_update_manifest_context_t;

bobe_update_manifest_result_t bobe_update_manifest_parse(
    const char *json,
    size_t length,
    const bobe_update_manifest_context_t *context,
    bobe_update_manifest_t *manifest
);

const char *bobe_update_manifest_result_name(
    bobe_update_manifest_result_t result
);

#ifdef __cplusplus
}
#endif
