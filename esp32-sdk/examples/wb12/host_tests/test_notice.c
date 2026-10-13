#include "bobe_notice.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define CHECK(c) do { if (!(c)) { fprintf(stderr, "check failed at %s:%d: %s\n", __FILE__, __LINE__, #c); exit(1); } } while (0)

int main(void)
{
    bobe_notice_t notice;
    CHECK(bobe_notice_volume(50, 55, &notice));
    CHECK(strcmp(notice.text, "Volume 55%") == 0);
    CHECK(bobe_notice_volume(5, 0, &notice));
    CHECK(strcmp(notice.text, "Sound off") == 0);
    CHECK(!bobe_notice_volume(100, 100, &notice));
    for (unsigned int level = 0; level <= 100U; ++level) {
        CHECK(bobe_notice_volume((uint8_t)(level == 50U ? 51U : 50U), (uint8_t)level, &notice));
        CHECK(strlen(notice.text) < BOBE_NOTICE_MAX_BYTES);
    }
    bobe_state_machine_t state = {.lifecycle = BOBE_LIFECYCLE_UNPROVISIONED};
    CHECK(bobe_notice_control_hint(&state, &notice));
    CHECK(strcmp(notice.text, "Hold +/-: setup") == 0);
    state.lifecycle = BOBE_LIFECYCLE_ONLINE;
    CHECK(bobe_notice_control_hint(&state, &notice));
    CHECK(strcmp(notice.text, "Hold O: talk") == 0);
    state.microphone_muted = true;
    CHECK(bobe_notice_control_hint(&state, &notice));
    CHECK(strcmp(notice.text, "Hold -: mic") == 0);
    CHECK(bobe_notice_update_confirmed(&notice));
    CHECK(strcmp(notice.text, "Update installed") == 0);
    puts("notice tests passed");
    return 0;
}
