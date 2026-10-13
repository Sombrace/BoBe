#pragma once

#include "bobe_update_manifest.h"

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

#define BOBE_UPDATE_SIGNATURE_BASE64_MAX 128U

typedef bool (*bobe_update_signature_verifier_t)(
    const unsigned char *manifest,
    size_t manifest_length,
    const unsigned char *signature,
    size_t signature_length,
    void *context
);

typedef enum {
    BOBE_UPDATE_OFFER_OK = 0,
    BOBE_UPDATE_OFFER_NOT_OFFER,
    BOBE_UPDATE_OFFER_INVALID_ARGUMENT,
    BOBE_UPDATE_OFFER_MALFORMED,
    BOBE_UPDATE_OFFER_SIGNATURE_INVALID,
    BOBE_UPDATE_OFFER_MANIFEST_INVALID,
} bobe_update_offer_result_t;

bobe_update_offer_result_t bobe_update_offer_parse(
    const char *json,
    size_t length,
    const bobe_update_manifest_context_t *context,
    bobe_update_signature_verifier_t verify_signature,
    void *verifier_context,
    bobe_update_manifest_t *manifest,
    bobe_update_manifest_result_t *manifest_result
);

const char *bobe_update_offer_result_name(bobe_update_offer_result_t result);

#ifdef __cplusplus
}
#endif
