#include "bobe_wifi_recovery.h"

#include <limits.h>
#include <string.h>

static void increment_saturated(unsigned int *value)
{
    if (*value < UINT_MAX) {
        ++*value;
    }
}

void bobe_wifi_recovery_reset(bobe_wifi_recovery_t *recovery)
{
    memset(recovery, 0, sizeof(*recovery));
}

bool bobe_wifi_recovery_observe(
    bobe_wifi_recovery_t *recovery,
    bobe_wifi_failure_class_t failure,
    uint64_t now_us
)
{
    if (failure == BOBE_WIFI_FAILURE_CREDENTIAL) {
        recovery->ap_unavailable_failures = 0;
        recovery->ap_unavailable_since_us = 0;
        increment_saturated(&recovery->credential_failures);
        return recovery->credential_failures >=
               BOBE_WIFI_CREDENTIAL_FAILURE_LIMIT;
    }

    if (failure == BOBE_WIFI_FAILURE_AP_UNAVAILABLE) {
        recovery->credential_failures = 0;
        if (recovery->ap_unavailable_failures == 0 ||
            now_us < recovery->ap_unavailable_since_us) {
            recovery->ap_unavailable_failures = 0;
            recovery->ap_unavailable_since_us = now_us;
        }
        increment_saturated(&recovery->ap_unavailable_failures);
        return recovery->ap_unavailable_failures >=
                   BOBE_WIFI_AP_UNAVAILABLE_FAILURE_LIMIT &&
               now_us - recovery->ap_unavailable_since_us >=
                   BOBE_WIFI_AP_UNAVAILABLE_MINIMUM_US;
    }

    bobe_wifi_recovery_reset(recovery);
    return false;
}
