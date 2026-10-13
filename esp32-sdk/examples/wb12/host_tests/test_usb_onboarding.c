#include "bobe_usb_onboarding_protocol.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

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

static void test_info_and_noise(void)
{
    bobe_usb_request_t request;
    const char *noise = "I (100) bobe: health\n";
    CHECK(
        bobe_usb_onboarding_parse(noise, strlen(noise), &request) ==
        BOBE_USB_PARSE_IGNORED
    );
    const char *info =
        "@BOBE-WIFI/1 INFO 0123456789abcdef0123456789abcdef\r\n";
    CHECK(
        bobe_usb_onboarding_parse(info, strlen(info), &request) ==
        BOBE_USB_PARSE_OK
    );
    CHECK(request.kind == BOBE_USB_REQUEST_INFO);
    CHECK(
        strcmp(request.nonce, "0123456789abcdef0123456789abcdef") == 0
    );
}

static void test_wifi_request(void)
{
    bobe_usb_request_t request;
    const char *set =
        "@BOBE-WIFI/1 SET 0123456789abcdef0123456789abcdef "
        "V3JvbmdOZXR3b3Jr cGFzc3dvcmQtMTIz\n";
    CHECK(
        bobe_usb_onboarding_parse(set, strlen(set), &request) ==
        BOBE_USB_PARSE_OK
    );
    CHECK(request.kind == BOBE_USB_REQUEST_SET_WIFI);
    CHECK(request.ssid_length == strlen("WrongNetwork"));
    CHECK(memcmp(request.ssid, "WrongNetwork", request.ssid_length) == 0);
    CHECK(request.passphrase_length == strlen("password-123"));
    CHECK(
        memcmp(
            request.passphrase,
            "password-123",
            request.passphrase_length
        ) == 0
    );
}

static void test_rejects_invalid_fields(void)
{
    bobe_usb_request_t request;
    const char *bad_nonce =
        "@BOBE-WIFI/1 INFO 0123456789ABCDEF0123456789ABCDEF\n";
    CHECK(
        bobe_usb_onboarding_parse(
            bad_nonce,
            strlen(bad_nonce),
            &request
        ) == BOBE_USB_PARSE_MALFORMED
    );
    const char *short_password =
        "@BOBE-WIFI/1 SET 0123456789abcdef0123456789abcdef "
        "bmV0d29yaw c2hvcnQ\n";
    CHECK(
        bobe_usb_onboarding_parse(
            short_password,
            strlen(short_password),
            &request
        ) == BOBE_USB_PARSE_PASSPHRASE_LENGTH
    );
    char oversized[BOBE_USB_ONBOARDING_LINE_MAX + 2U];
    memset(oversized, 'x', sizeof(oversized));
    CHECK(
        bobe_usb_onboarding_parse(
            oversized,
            sizeof(oversized),
            &request
        ) == BOBE_USB_PARSE_MALFORMED
    );
}

int main(void)
{
    test_info_and_noise();
    test_wifi_request();
    test_rejects_invalid_fields();
    return 0;
}
