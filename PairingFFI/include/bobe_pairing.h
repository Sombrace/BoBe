#ifndef BOBE_PAIRING_H
#define BOBE_PAIRING_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef enum {
    BOBE_PAIRING_OK = 0,
    BOBE_PAIRING_INVALID_ARGUMENT = 1,
    BOBE_PAIRING_INVALID_MESSAGE = 2,
    BOBE_PAIRING_AUTHENTICATION_FAILED = 3,
    BOBE_PAIRING_INTERNAL_ERROR = 4,
} bobe_pairing_status;

typedef struct {
    uint8_t *data;
    size_t len;
} bobe_pairing_buffer;

typedef struct bobe_pairing_client bobe_pairing_client;

bobe_pairing_status bobe_pairing_client_start(
    const uint8_t *code,
    size_t code_len,
    bobe_pairing_client **client,
    bobe_pairing_buffer *request
);

bobe_pairing_status bobe_pairing_client_finish(
    bobe_pairing_client *client,
    const uint8_t *code,
    size_t code_len,
    const uint8_t *response,
    size_t response_len,
    const uint8_t *context,
    size_t context_len,
    bobe_pairing_buffer *finalization,
    bobe_pairing_buffer *session_key
);

bobe_pairing_status bobe_pairing_spki_sha256(
    const uint8_t *certificate_der,
    size_t certificate_der_len,
    uint8_t output[32]
);

void bobe_pairing_client_destroy(bobe_pairing_client *client);
void bobe_pairing_buffer_destroy(bobe_pairing_buffer *buffer);

#ifdef __cplusplus
}
#endif

#endif
