#include "bobe_protocol.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char **argv)
{
    char json[BOBE_CONTROL_MAX_BYTES + 2U];
    if (argc == 2 && strcmp(argv[1], "decode") == 0) {
        const size_t length = fread(json, 1, sizeof(json), stdin);
        bobe_control_message_t message;
        const bobe_protocol_result_t result = bobe_protocol_parse_control(json, length, &message);
        if (result != BOBE_PROTOCOL_OK) {
            fprintf(stderr, "protocol error %u\n", (unsigned)result);
            return 1;
        }
        printf("decoded %u\n", (unsigned)message.kind);
        return 0;
    }
    if (argc == 5 && strcmp(argv[1], "hello") == 0) {
        bobe_hello_info_t hello = {
            .device_id = argv[3], .boot_id = argv[4],
            .firmware_version = "0.2.0", .hardware_revision = "sdk-protocol-probe",
        };
        if (strcmp(argv[2], "terminal") == 0) {
            hello.capabilities = (bobe_capabilities_t){.text_input=true,.text_output=true,.actions=true,.sensors=true,.text_columns=20,.text_rows=4};
        } else if (strcmp(argv[2], "sensor") == 0) {
            hello.capabilities.sensors = true;
        } else if (strcmp(argv[2], "voice") == 0) {
            hello.capabilities.microphone = true;
            hello.capabilities.speaker = true;
            hello.max_playback_credit_ms = 120;
        } else return 2;
        if (bobe_protocol_build_hello(json, sizeof(json), &hello) != BOBE_PROTOCOL_OK) return 1;
    } else if (argc == 4 && strcmp(argv[1], "text") == 0) {
        if (bobe_protocol_build_text_input(json, sizeof(json), argv[2], argv[3]) != BOBE_PROTOCOL_OK) return 1;
    } else if (argc == 5 && strcmp(argv[1], "action") == 0) {
        if (bobe_protocol_build_action(json, sizeof(json), argv[2], argv[3], argv[4]) != BOBE_PROTOCOL_OK) return 1;
    } else {
        fprintf(stderr, "Usage: bobe-protocol decode | hello terminal|sensor|voice ID BOOT_ID | text REQUEST_ID TEXT | action REQUEST_ID ID LABEL\n");
        return 2;
    }
    puts(json);
    return 0;
}
