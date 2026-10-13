#include "bobe_wifi_recovery.h"

#include <stdio.h>
#include <stdlib.h>

#define CHECK(condition)                                                     \
    do {                                                                     \
        if (!(condition)) {                                                  \
            fprintf(                                                        \
                stderr,                                                     \
                "check failed at %s:%d: %s\n",                              \
                __FILE__,                                                    \
                __LINE__,                                                    \
                #condition                                                   \
            );                                                               \
            exit(1);                                                         \
        }                                                                    \
    } while (0)

static void test_credential_failures_recover_quickly(void)
{
    bobe_wifi_recovery_t recovery = {0};
    CHECK(!bobe_wifi_recovery_observe(
        &recovery,
        BOBE_WIFI_FAILURE_CREDENTIAL,
        0
    ));
    CHECK(!bobe_wifi_recovery_observe(
        &recovery,
        BOBE_WIFI_FAILURE_CREDENTIAL,
        1000
    ));
    CHECK(bobe_wifi_recovery_observe(
        &recovery,
        BOBE_WIFI_FAILURE_CREDENTIAL,
        2000
    ));
}

static void test_missing_ap_requires_count_and_time(void)
{
    bobe_wifi_recovery_t recovery = {0};
    for (unsigned int index = 0;
         index < BOBE_WIFI_AP_UNAVAILABLE_FAILURE_LIMIT;
         ++index) {
        CHECK(!bobe_wifi_recovery_observe(
            &recovery,
            BOBE_WIFI_FAILURE_AP_UNAVAILABLE,
            (uint64_t)index * UINT64_C(10000000)
        ));
    }
    CHECK(bobe_wifi_recovery_observe(
        &recovery,
        BOBE_WIFI_FAILURE_AP_UNAVAILABLE,
        BOBE_WIFI_AP_UNAVAILABLE_MINIMUM_US
    ));
}

static void test_unrelated_failure_breaks_a_streak(void)
{
    bobe_wifi_recovery_t recovery = {0};
    CHECK(!bobe_wifi_recovery_observe(
        &recovery,
        BOBE_WIFI_FAILURE_CREDENTIAL,
        0
    ));
    CHECK(!bobe_wifi_recovery_observe(
        &recovery,
        BOBE_WIFI_FAILURE_OTHER,
        1
    ));
    CHECK(!bobe_wifi_recovery_observe(
        &recovery,
        BOBE_WIFI_FAILURE_CREDENTIAL,
        2
    ));
    CHECK(!bobe_wifi_recovery_observe(
        &recovery,
        BOBE_WIFI_FAILURE_CREDENTIAL,
        3
    ));
}

static void test_reset_and_clock_regression_restart_recovery(void)
{
    bobe_wifi_recovery_t recovery = {0};
    CHECK(!bobe_wifi_recovery_observe(
        &recovery,
        BOBE_WIFI_FAILURE_AP_UNAVAILABLE,
        UINT64_C(70000000)
    ));
    CHECK(!bobe_wifi_recovery_observe(
        &recovery,
        BOBE_WIFI_FAILURE_AP_UNAVAILABLE,
        UINT64_C(10000000)
    ));
    CHECK(recovery.ap_unavailable_failures == 1);
    bobe_wifi_recovery_reset(&recovery);
    CHECK(recovery.ap_unavailable_failures == 0);
    CHECK(recovery.credential_failures == 0);
}

int main(void)
{
    test_credential_failures_recover_quickly();
    test_missing_ap_requires_count_and_time();
    test_unrelated_failure_breaks_a_streak();
    test_reset_and_clock_regression_restart_recovery();
    return 0;
}
