#include "bobe_device_config.h"
#include "bobe_update_offer.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define CHECK(condition)                                                     \
    do {                                                                     \
        if (!(condition)) {                                                  \
            fprintf(stderr, "check failed at %s:%d: %s\n",                 \
                    __FILE__, __LINE__, #condition);                         \
            exit(1);                                                         \
        }                                                                    \
    } while (0)

static const char manifest[] =
    "{\"schema\":1,\"device_model\":\"wonderboy-wb12\","
    "\"version\":\"1.4.2\",\"build_number\":42,\"image_size\":1048576,"
    "\"image_sha256\":\"0123456789abcdef0123456789abcdef"
    "0123456789abcdef0123456789abcdef\","
    "\"url\":\"https://ota.example.com/wb12.bin\","
    "\"key_id\":\"prod-2026-01\",\"not_before\":1700000000,"
    "\"expires_at\":1800000000}";

static void escape_manifest(char *output, size_t capacity, const char *input)
{
    size_t used = 0;
    for (; *input != '\0'; ++input) {
        if (*input == '"' || *input == '\\') {
            CHECK(used + 2U < capacity);
            output[used++] = '\\';
        } else {
            CHECK(used + 1U < capacity);
        }
        output[used++] = *input;
    }
    output[used] = '\0';
}

static bool verifier(
    const unsigned char *raw_manifest,
    size_t manifest_length,
    const unsigned char *signature,
    size_t signature_length,
    void *expected
)
{
    return manifest_length == strlen(manifest) &&
           memcmp(raw_manifest, manifest, manifest_length) == 0 &&
           signature_length == 3U && memcmp(signature, "sig", 3U) == 0 &&
           expected != NULL;
}

static bobe_update_offer_result_t verify(const char *offer)
{
    bobe_update_manifest_t parsed;
    bobe_update_manifest_result_t manifest_result;
    const bobe_update_manifest_context_t context = {
        .device_model = BOBE_DEVICE_MODEL, .max_image_size = BOBE_UPDATE_IMAGE_SIZE_MAX, .current_build_number = 41,
    };
    return bobe_update_offer_parse(
        offer,
        strlen(offer),
        &context,
        verifier,
        &parsed,
        &parsed,
        &manifest_result
    );
}

static void build_offer(
    char *output,
    size_t capacity,
    const char *escaped_manifest,
    const char *signature
)
{
    const int written = snprintf(
        output,
        capacity,
        "{\"type\":\"update.offer\",\"protocol_major\":1,"
        "\"manifest\":\"%s\",\"signature\":\"%s\"}",
        escaped_manifest,
        signature
    );
    CHECK(written > 0 && (size_t)written < capacity);
}

int main(void)
{
    char escaped[1024];
    char offer[1536];
    escape_manifest(escaped, sizeof(escaped), manifest);
    build_offer(offer, sizeof(offer), escaped, "c2ln");
    CHECK(verify(offer) == BOBE_UPDATE_OFFER_OK);

    char tampered[1024];
    strcpy(tampered, escaped);
    char *version = strstr(tampered, "1.4.2");
    CHECK(version != NULL);
    version[4] = '3';
    build_offer(offer, sizeof(offer), tampered, "c2ln");
    CHECK(verify(offer) == BOBE_UPDATE_OFFER_SIGNATURE_INVALID);

    build_offer(offer, sizeof(offer), escaped, "not-base64");
    CHECK(verify(offer) == BOBE_UPDATE_OFFER_SIGNATURE_INVALID);

    snprintf(
        offer,
        sizeof(offer),
        "{\"type\":\"update.offer\",\"protocol_major\":1,"
        "\"manifest\":\"%s\",\"signature\":\"c2ln\",\"extra\":1}",
        escaped
    );
    CHECK(verify(offer) == BOBE_UPDATE_OFFER_MALFORMED);
    snprintf(
        offer,
        sizeof(offer),
        "{\"type\":\"welcome\",\"protocol_major\":1,\"protocol_minor\":1}"
    );
    CHECK(verify(offer) == BOBE_UPDATE_OFFER_NOT_OFFER);
    snprintf(
        offer,
        sizeof(offer),
        "{\"type\":\"theme.set\",\"protocol_major\":1,"
        "\"state_version\":1,\"theme_id\":\"warm-ledge\"}"
    );
    CHECK(verify(offer) == BOBE_UPDATE_OFFER_NOT_OFFER);
    puts("BoBe signed update offer tests passed.");
    return 0;
}
