#pragma once

#include <stdbool.h>
#include <stdint.h>

#define BOBE_WIFI_CREDENTIAL_FAILURE_LIMIT 3U
#define BOBE_WIFI_AP_UNAVAILABLE_FAILURE_LIMIT 6U
#define BOBE_WIFI_AP_UNAVAILABLE_MINIMUM_US UINT64_C(60000000)

typedef enum {
    BOBE_WIFI_FAILURE_OTHER = 0,
    BOBE_WIFI_FAILURE_CREDENTIAL,
    BOBE_WIFI_FAILURE_AP_UNAVAILABLE,
} bobe_wifi_failure_class_t;

typedef struct {
    unsigned int credential_failures;
    unsigned int ap_unavailable_failures;
    uint64_t ap_unavailable_since_us;
} bobe_wifi_recovery_t;

void bobe_wifi_recovery_reset(bobe_wifi_recovery_t *recovery);
bool bobe_wifi_recovery_observe(
    bobe_wifi_recovery_t *recovery,
    bobe_wifi_failure_class_t failure,
    uint64_t now_us
);
