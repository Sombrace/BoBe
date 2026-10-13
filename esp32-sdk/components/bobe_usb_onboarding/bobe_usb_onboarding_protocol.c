#include "bobe_usb_onboarding_protocol.h"

#include <stdbool.h>
#include <string.h>

#define PREFIX "@BOBE-WIFI/1 "

static bool valid_nonce(const char *value)
{
    for (size_t index = 0; index < 2U * BOBE_USB_ONBOARDING_NONCE_BYTES; ++index) {
        const char character = value[index];
        if (!((character >= '0' && character <= '9') ||
              (character >= 'a' && character <= 'f'))) {
            return false;
        }
    }
    return value[2U * BOBE_USB_ONBOARDING_NONCE_BYTES] == '\0';
}

static int base64_value(char character)
{
    if (character >= 'A' && character <= 'Z') {
        return character - 'A';
    }
    if (character >= 'a' && character <= 'z') {
        return character - 'a' + 26;
    }
    if (character >= '0' && character <= '9') {
        return character - '0' + 52;
    }
    if (character == '-' || character == '+') {
        return 62;
    }
    if (character == '_' || character == '/') {
        return 63;
    }
    return -1;
}

static bool decode_base64url(
    const char *source,
    uint8_t *output,
    size_t capacity,
    size_t *output_length
)
{
    const size_t length = strlen(source);
    if (length == 0 || length % 4U == 1U) {
        return false;
    }
    uint32_t accumulator = 0;
    unsigned int bits = 0;
    size_t written = 0;
    for (size_t index = 0; index < length; ++index) {
        const int value = base64_value(source[index]);
        if (value < 0) {
            return false;
        }
        accumulator = (accumulator << 6U) | (uint32_t)value;
        bits += 6U;
        if (bits >= 8U) {
            bits -= 8U;
            if (written >= capacity) {
                return false;
            }
            output[written++] = (uint8_t)(accumulator >> bits);
            accumulator &= bits == 0U ? 0U : (UINT32_C(1) << bits) - 1U;
        }
    }
    if (accumulator != 0U) {
        return false;
    }
    *output_length = written;
    return true;
}

bobe_usb_parse_result_t bobe_usb_onboarding_parse(
    const char *line,
    size_t length,
    bobe_usb_request_t *request
)
{
    if (line == NULL || request == NULL || length == 0 ||
        length > BOBE_USB_ONBOARDING_LINE_MAX) {
        return BOBE_USB_PARSE_MALFORMED;
    }
    memset(request, 0, sizeof(*request));
    if (length < sizeof(PREFIX) - 1U ||
        memcmp(line, PREFIX, sizeof(PREFIX) - 1U) != 0) {
        return BOBE_USB_PARSE_IGNORED;
    }

    char copy[BOBE_USB_ONBOARDING_LINE_MAX + 1U];
    memcpy(copy, line, length);
    copy[length] = '\0';
    while (length > 0 && (copy[length - 1U] == '\n' ||
                          copy[length - 1U] == '\r')) {
        copy[--length] = '\0';
    }

    char *save = NULL;
    char *command = strtok_r(copy + sizeof(PREFIX) - 1U, " ", &save);
    char *nonce = strtok_r(NULL, " ", &save);
    if (command == NULL || nonce == NULL || !valid_nonce(nonce)) {
        return BOBE_USB_PARSE_MALFORMED;
    }
    memcpy(request->nonce, nonce, sizeof(request->nonce));
    if (strcmp(command, "INFO") == 0) {
        if (strtok_r(NULL, " ", &save) != NULL) {
            return BOBE_USB_PARSE_MALFORMED;
        }
        request->kind = BOBE_USB_REQUEST_INFO;
        return BOBE_USB_PARSE_OK;
    }
    if (strcmp(command, "SET") != 0) {
        return BOBE_USB_PARSE_MALFORMED;
    }

    char *ssid = strtok_r(NULL, " ", &save);
    char *passphrase = strtok_r(NULL, " ", &save);
    if (ssid == NULL || passphrase == NULL ||
        strtok_r(NULL, " ", &save) != NULL) {
        return BOBE_USB_PARSE_MALFORMED;
    }
    if (!decode_base64url(
            ssid,
            request->ssid,
            sizeof(request->ssid) - 1U,
            &request->ssid_length
        ) ||
        request->ssid_length == 0 || request->ssid_length > 32 ||
        memchr(request->ssid, '\0', request->ssid_length) != NULL) {
        return BOBE_USB_PARSE_SSID_LENGTH;
    }
    if (strcmp(passphrase, "-") != 0 &&
        (!decode_base64url(
             passphrase,
             request->passphrase,
             sizeof(request->passphrase) - 1U,
             &request->passphrase_length
         ) ||
         request->passphrase_length < 8 ||
         request->passphrase_length > 63 ||
         memchr(
             request->passphrase,
             '\0',
             request->passphrase_length
         ) != NULL)) {
        return BOBE_USB_PARSE_PASSPHRASE_LENGTH;
    }
    request->kind = BOBE_USB_REQUEST_SET_WIFI;
    return BOBE_USB_PARSE_OK;
}
