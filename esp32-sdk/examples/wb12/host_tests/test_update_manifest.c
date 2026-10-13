#include "bobe_device_config.h"
#include "bobe_update_manifest.h"

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

#define DIGEST "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"

static const char valid[] =
    "{\"schema\":1,\"device_model\":\"wonderboy-wb12\","
    "\"version\":\"1.4.2\",\"build_number\":42,\"image_size\":1048576,"
    "\"image_sha256\":\"" DIGEST "\","
    "\"url\":\"https://ota.example.com/wb12.bin\","
    "\"key_id\":\"prod-2026-01\",\"not_before\":1700000000,"
    "\"expires_at\":1800000000}";

static bobe_update_manifest_context_t context(void)
{
    return (bobe_update_manifest_context_t){.device_model = BOBE_DEVICE_MODEL, .max_image_size = BOBE_UPDATE_IMAGE_SIZE_MAX, .current_build_number = 41};
}

static bobe_update_manifest_result_t parse(const char *json)
{
    bobe_update_manifest_t manifest;
    const bobe_update_manifest_context_t ctx = context();
    return bobe_update_manifest_parse(json, strlen(json), &ctx, &manifest);
}

static void replace_field(
    char *output,
    size_t capacity,
    const char *field,
    const char *replacement
)
{
    const char *start = strstr(valid, field);
    CHECK(start != NULL);
    const char *end = strchr(start, ':');
    CHECK(end != NULL);
    bool quoted = false;
    while (*end != '\0') {
        if (*end == '"') {
            quoted = !quoted;
        }
        if (!quoted && (*end == ',' || *end == '}')) {
            break;
        }
        ++end;
    }
    const int written = snprintf(
        output,
        capacity,
        "%.*s%s%s",
        (int)(start - valid),
        valid,
        replacement,
        end
    );
    CHECK(written > 0 && (size_t)written < capacity);
}

static bobe_update_manifest_result_t variant(
    const char *field,
    const char *replacement
)
{
    char json[1024];
    replace_field(json, sizeof(json), field, replacement);
    return parse(json);
}

static void test_valid(void)
{
    bobe_update_manifest_t manifest;
    const bobe_update_manifest_context_t ctx = context();
    CHECK(bobe_update_manifest_parse(valid, strlen(valid), &ctx, &manifest) ==
          BOBE_UPDATE_MANIFEST_OK);
    CHECK(strcmp(manifest.version, "1.4.2") == 0);
    CHECK(manifest.build_number == 42);
    CHECK(manifest.image_size == 1048576);
    CHECK(manifest.image_sha256[0] == 0x01);
    CHECK(manifest.image_sha256[31] == 0xef);
}

static void test_structure(void)
{
    CHECK(parse("") == BOBE_UPDATE_MANIFEST_MALFORMED);
    CHECK(parse("[]") == BOBE_UPDATE_MANIFEST_MALFORMED);
    CHECK(parse("{}") == BOBE_UPDATE_MANIFEST_MISSING_FIELD);
    CHECK(parse("{\"schema\":1,\"schema\":1}") ==
          BOBE_UPDATE_MANIFEST_DUPLICATE_FIELD);
    CHECK(parse("{\"unknown\":1}") == BOBE_UPDATE_MANIFEST_UNKNOWN_FIELD);
    char trailing[1024];
    snprintf(trailing, sizeof(trailing), "%s{}", valid);
    CHECK(parse(trailing) == BOBE_UPDATE_MANIFEST_TRAILING_DATA);
}

static void test_security_bounds(void)
{
    CHECK(variant("\"schema\"", "\"schema\":2") ==
          BOBE_UPDATE_MANIFEST_SCHEMA_UNSUPPORTED);
    CHECK(variant("\"device_model\"", "\"device_model\":\"other\"") ==
          BOBE_UPDATE_MANIFEST_MODEL_MISMATCH);
    CHECK(variant("\"version\"", "\"version\":\"1.0 beta\"") ==
          BOBE_UPDATE_MANIFEST_VERSION_INVALID);
    CHECK(variant("\"version\"", "\"version\":\"1.0\\u0000\"") ==
          BOBE_UPDATE_MANIFEST_EMBEDDED_NUL);
    CHECK(variant("\"build_number\"", "\"build_number\":42.0") ==
          BOBE_UPDATE_MANIFEST_IMPRECISE_NUMBER);
    CHECK(variant("\"build_number\"", "\"build_number\":042") ==
          BOBE_UPDATE_MANIFEST_MALFORMED);
    CHECK(variant("\"image_size\"", "\"image_size\":5242881") ==
          BOBE_UPDATE_MANIFEST_IMAGE_SIZE_INVALID);
    CHECK(variant("\"url\"", "\"url\":\"http://ota/a.bin\"") ==
          BOBE_UPDATE_MANIFEST_URL_INVALID);
    CHECK(variant("\"image_sha256\"", "\"image_sha256\":\"ABC\"") ==
          BOBE_UPDATE_MANIFEST_DIGEST_INVALID);
}

static void test_upgrade_and_time(void)
{
    bobe_update_manifest_t manifest;
    bobe_update_manifest_context_t ctx = context();
    ctx.current_build_number = 42;
    CHECK(bobe_update_manifest_parse(valid, strlen(valid), &ctx, &manifest) ==
          BOBE_UPDATE_MANIFEST_NOT_AN_UPGRADE);
    ctx.current_build_number = 41;
    ctx.trusted_time_valid = true;
    ctx.trusted_unix_time = 1699999999;
    CHECK(bobe_update_manifest_parse(valid, strlen(valid), &ctx, &manifest) ==
          BOBE_UPDATE_MANIFEST_NOT_YET_VALID);
    ctx.trusted_unix_time = 1800000000;
    CHECK(bobe_update_manifest_parse(valid, strlen(valid), &ctx, &manifest) ==
          BOBE_UPDATE_MANIFEST_EXPIRED);
    ctx.trusted_unix_time = 1700000000;
    CHECK(bobe_update_manifest_parse(valid, strlen(valid), &ctx, &manifest) ==
          BOBE_UPDATE_MANIFEST_OK);
}

static void test_arguments_and_output(void)
{
    bobe_update_manifest_t manifest;
    const bobe_update_manifest_context_t ctx = context();
    CHECK(bobe_update_manifest_parse(NULL, 1, &ctx, &manifest) ==
          BOBE_UPDATE_MANIFEST_INVALID_ARGUMENT);
    memset(&manifest, 0x5a, sizeof(manifest));
    bobe_update_manifest_t expected = manifest;
    CHECK(bobe_update_manifest_parse("{}", 2, &ctx, &manifest) !=
          BOBE_UPDATE_MANIFEST_OK);
    CHECK(memcmp(&manifest, &expected, sizeof(manifest)) == 0);
    CHECK(strcmp(bobe_update_manifest_result_name(
                     BOBE_UPDATE_MANIFEST_NOT_AN_UPGRADE),
                 "not_an_upgrade") == 0);
}

int main(void)
{
    test_valid();
    test_structure();
    test_security_bounds();
    test_upgrade_and_time();
    test_arguments_and_output();
    puts("BoBe update manifest tests passed.");
    return 0;
}
