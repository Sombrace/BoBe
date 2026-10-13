#pragma once

#include <stddef.h>
#include <stdint.h>

#define BOBE_USB_ONBOARDING_LINE_MAX 256U
#define BOBE_USB_ONBOARDING_NONCE_BYTES 16U

typedef enum {
    BOBE_USB_REQUEST_NONE = 0,
    BOBE_USB_REQUEST_INFO,
    BOBE_USB_REQUEST_SET_WIFI,
} bobe_usb_request_kind_t;

typedef struct {
    bobe_usb_request_kind_t kind;
    char nonce[33];
    uint8_t ssid[33];
    size_t ssid_length;
    uint8_t passphrase[64];
    size_t passphrase_length;
} bobe_usb_request_t;

typedef enum {
    BOBE_USB_PARSE_OK = 0,
    BOBE_USB_PARSE_IGNORED,
    BOBE_USB_PARSE_MALFORMED,
    BOBE_USB_PARSE_SSID_LENGTH,
    BOBE_USB_PARSE_PASSPHRASE_LENGTH,
} bobe_usb_parse_result_t;

bobe_usb_parse_result_t bobe_usb_onboarding_parse(
    const char *line,
    size_t length,
    bobe_usb_request_t *request
);
