#include "bobe_protocol.h"

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define CHECK(test) do { if (!(test)) { \
    fprintf(stderr, "line %d: %s\n", __LINE__, #test); return EXIT_FAILURE; \
} } while (0)

static const char request[] = "00000000-0000-4000-8000-000000000001";

int main(void)
{
    char json[4096];
    bobe_hello_info_t hello = {
        .device_id = "keyboard", .boot_id = "0123456789abcdef0123456789abcdef",
        .firmware_version = "0.2.0", .hardware_revision = "generic-terminal",
        .capabilities = {.text_input = true, .text_output = true, .actions = true,
                         .sensors = true, .text_columns = 20, .text_rows = 4},
    };
    CHECK(bobe_protocol_build_hello(json, sizeof(json), &hello) == BOBE_PROTOCOL_OK);
    CHECK(strstr(json, "\"protocol_minor\":2") != NULL);
    CHECK(strstr(json, "\"microphone\":false") != NULL);
    CHECK(strstr(json, "\"columns\":20") != NULL);
    CHECK(strstr(json, "\"max_playback_credit_ms\":0") != NULL);
    hello.capabilities = (bobe_capabilities_t){.microphone = true, .speaker = true};
    hello.max_playback_credit_ms = 120;
    CHECK(bobe_protocol_build_hello(json, sizeof(json), &hello) == BOBE_PROTOCOL_OK);
    CHECK(strstr(json, "\"protocol_minor\":1") != NULL);
    CHECK(strstr(json, "text_input") == NULL);

    CHECK(bobe_protocol_build_text_input(json, sizeof(json), request, "Hello \"BoBe\" 👋") == BOBE_PROTOCOL_OK);
    CHECK(strstr(json, "\\\"BoBe\\\"") != NULL);
    CHECK(bobe_protocol_build_text_input(json, 16, request, "hello") == BOBE_PROTOCOL_TOO_LARGE);
    CHECK(bobe_protocol_build_text_input(json, sizeof(json), "invalid", "hello") == BOBE_PROTOCOL_INVALID_ARGUMENT);
    CHECK(bobe_protocol_build_text_input(json, sizeof(json), request, "\xc0\x80") == BOBE_PROTOCOL_INVALID_ARGUMENT);
    CHECK(bobe_protocol_build_action(json, sizeof(json), request, "menu.next", "Next page") == BOBE_PROTOCOL_OK);
    CHECK(bobe_protocol_build_action(json, sizeof(json), request, "../../exec", "Next") == BOBE_PROTOCOL_INVALID_ARGUMENT);
    CHECK(bobe_protocol_build_sensor_number(json, sizeof(json), "temperature", 1, 22.5, "°C") == BOBE_PROTOCOL_OK);
    CHECK(bobe_protocol_build_sensor_number(json, sizeof(json), "temperature", 2, INFINITY, "C") == BOBE_PROTOCOL_INVALID_ARGUMENT);
    CHECK(bobe_protocol_build_sensor_number(json, sizeof(json), "temperature", 2, 22.5, "C\n") == BOBE_PROTOCOL_INVALID_ARGUMENT);
    CHECK(bobe_protocol_build_sensor_number(json, sizeof(json), "temperature", 2, 22.5, "\x7f") == BOBE_PROTOCOL_INVALID_ARGUMENT);
    CHECK(bobe_protocol_build_sensor_boolean(json, sizeof(json), "door.open", 3, true) == BOBE_PROTOCOL_OK);

    bobe_control_message_t message;
    const char *text = "{\"type\":\"display.text\",\"protocol_major\":1,"
        "\"request_id\":\"00000000-0000-4000-8000-000000000001\","
        "\"sequence\":0,\"clear\":true,\"final\":false,\"text\":\"Hello 👋\"}";
    CHECK(bobe_protocol_parse_control(text, strlen(text), &message) == BOBE_PROTOCOL_OK);
    CHECK(message.kind == BOBE_CONTROL_DISPLAY_TEXT && message.data.display_text.clear);
    CHECK(strcmp(message.data.display_text.text, "Hello 👋") == 0);
    const char *result = "{\"type\":\"input.result\",\"protocol_major\":1,"
        "\"request_id\":\"00000000-0000-4000-8000-000000000001\",\"status\":\"completed\",\"replayed\":true}";
    CHECK(bobe_protocol_parse_control(result, strlen(result), &message) == BOBE_PROTOCOL_OK);
    CHECK(message.kind == BOBE_CONTROL_INPUT_RESULT && message.data.input_result.replayed);
    const char *ack = "{\"type\":\"sensor.ack\",\"protocol_major\":1,\"name\":\"door.open\",\"sequence\":3}";
    CHECK(bobe_protocol_parse_control(ack, strlen(ack), &message) == BOBE_PROTOCOL_OK);
    CHECK(message.kind == BOBE_CONTROL_SENSOR_ACK && message.data.sensor_ack.sequence == 3);
    const char *duplicate = "{\"type\":\"sensor.ack\",\"protocol_major\":1,\"name\":\"door.open\",\"sequence\":1,\"sequence\":2}";
    CHECK(bobe_protocol_parse_control(duplicate, strlen(duplicate), &message) == BOBE_PROTOCOL_MALFORMED);
    char nested[1024];
    size_t used = 0;
    for (unsigned index = 0; index < 40; ++index) nested[used++] = '[';
    for (unsigned index = 0; index < 40; ++index) nested[used++] = ']';
    CHECK(bobe_protocol_parse_control(nested, used, &message) == BOBE_PROTOCOL_MALFORMED);
    puts("Voice compatibility, text, actions, telemetry, UTF-8, and parser bounds passed");
    return EXIT_SUCCESS;
}
