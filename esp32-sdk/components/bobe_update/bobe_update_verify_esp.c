#include "bobe_update_verify.h"
#include "bobe_board.h"

#include "mbedtls/pk.h"
#include "psa/crypto.h"

static bool verify_embedded_signature(
    const unsigned char *manifest,
    size_t manifest_length,
    const unsigned char *signature,
    size_t signature_length,
    void *unused
)
{
    (void)unused;
    unsigned char digest[32];
    size_t digest_length = 0;
    if (psa_hash_compute(
            PSA_ALG_SHA_256,
            manifest,
            manifest_length,
            digest,
            sizeof(digest),
            &digest_length
        ) != PSA_SUCCESS ||
        digest_length != sizeof(digest)) {
        return false;
    }
    size_t public_key_length = 0;
    const unsigned char *public_key =
        bobe_board_update_public_key(&public_key_length);
    if (public_key == NULL || public_key_length == 0) {
        return false;
    }
    mbedtls_pk_context key;
    mbedtls_pk_init(&key);
    const int parse_result = mbedtls_pk_parse_public_key(
        &key,
        public_key,
        public_key_length
    );
    const int verify_result = parse_result == 0
                                  ? mbedtls_pk_verify(
                                        &key,
                                        MBEDTLS_MD_SHA256,
                                        digest,
                                        sizeof(digest),
                                        signature,
                                        signature_length
                                    )
                                  : parse_result;
    mbedtls_pk_free(&key);
    return verify_result == 0;
}

bobe_update_offer_result_t bobe_update_offer_verify_embedded(
    const char *json,
    size_t length,
    const bobe_update_manifest_context_t *context,
    bobe_update_manifest_t *manifest,
    bobe_update_manifest_result_t *manifest_result
)
{
    return bobe_update_offer_parse(
        json,
        length,
        context,
        verify_embedded_signature,
        NULL,
        manifest,
        manifest_result
    );
}
