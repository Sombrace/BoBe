#pragma once

#include "bobe_update_offer.h"

bobe_update_offer_result_t bobe_update_offer_verify_embedded(
    const char *json,
    size_t length,
    const bobe_update_manifest_context_t *context,
    bobe_update_manifest_t *manifest,
    bobe_update_manifest_result_t *manifest_result
);
