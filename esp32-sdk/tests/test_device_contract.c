#include "bobe_device_config.h"
#include "bobe_protocol.h"
#include "bobe_update_manifest.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define CHECK(condition) do { \
    if (!(condition)) { \
        fprintf(stderr, "contract failed at line %d: %s\n", __LINE__, #condition); \
        return EXIT_FAILURE; \
    } \
} while (0)

static bobe_update_manifest_result_t parse_offer(const char *model, unsigned size)
{
    char json[1024];
    const int written = snprintf(json, sizeof(json),
        "{\"schema\":1,\"device_model\":\"%s\",\"version\":\"1.0.0\","
        "\"build_number\":2,\"image_size\":%u,"
        "\"image_sha256\":\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\","
        "\"url\":\"https://example.com/body.bin\",\"key_id\":\"test\","
        "\"not_before\":1700000000,\"expires_at\":1800000000}", model, size);
    if (written <= 0 || (size_t)written >= sizeof(json)) {
        return BOBE_UPDATE_MANIFEST_INVALID_ARGUMENT;
    }
    bobe_update_manifest_t manifest;
    const bobe_update_manifest_context_t context = {.device_model = BOBE_DEVICE_MODEL, .max_image_size = BOBE_UPDATE_IMAGE_SIZE_MAX, .current_build_number = 1};
    return bobe_update_manifest_parse(json, (size_t)written, &context, &manifest);
}

int main(void)
{
    char json[1024];
    const bobe_hello_info_t hello = {
        .device_id = "contract-device", .boot_id = "0123456789abcdef0123456789abcdef",
        .firmware_version = "1.0.0", .hardware_revision = BOBE_DEVICE_MODEL,
        .capabilities = {.text_input = true, .text_output = true,
                         .display_width = 320, .display_height = 240, .face_schema = 1},
    };
    CHECK(bobe_protocol_build_hello(json, sizeof(json), &hello) == BOBE_PROTOCOL_OK);
    CHECK(strstr(json, "\"width\":320") != NULL);
    CHECK(strstr(json, "\"height\":240") != NULL);
    CHECK(strstr(json, "sdk-contract-body") != NULL);
    CHECK(parse_offer("sdk-contract-body", 1048576) == BOBE_UPDATE_MANIFEST_OK);
    CHECK(parse_offer("wonderboy-wb12", 1048576) == BOBE_UPDATE_MANIFEST_MODEL_MISMATCH);
    CHECK(parse_offer("sdk-contract-body", 2097153) == BOBE_UPDATE_MANIFEST_IMAGE_SIZE_INVALID);
    puts(json);
    puts("Independent device configuration and OTA compatibility passed");
    return EXIT_SUCCESS;
}
