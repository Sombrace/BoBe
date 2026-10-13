#include "bobe_notice.h"

#include <stdio.h>
#include <string.h>

static void set_text(bobe_notice_t *notice, const char *text)
{
    size_t length = strlen(text);
    if (length >= sizeof(notice->text)) {
        length = sizeof(notice->text) - 1U;
    }
    memcpy(notice->text, text, length);
    notice->text[length] = '\0';
}

bool bobe_notice_volume(uint8_t previous, uint8_t next, bobe_notice_t *notice)
{
    if (notice == NULL) return false;
    memset(notice, 0, sizeof(*notice));
    if (previous == next) return false;
    notice->ttl_ms = BOBE_NOTICE_CONTROL_TTL_MS;
    if (next == 0U) {
        notice->kind = BOBE_NOTICE_SILENT;
        set_text(notice, "Sound off");
        return true;
    }
    notice->kind = BOBE_NOTICE_VOLUME;
    (void)snprintf(notice->text, sizeof(notice->text), "Volume %u%%", (unsigned int)next);
    return true;
}

bool bobe_notice_control_hint(const bobe_state_machine_t *state, bobe_notice_t *notice)
{
    if (notice == NULL) return false;
    memset(notice, 0, sizeof(*notice));
    if (state == NULL) return false;
    const char *text = NULL;
    switch (state->lifecycle) {
    case BOBE_LIFECYCLE_UNPROVISIONED: text = "Hold +/-: setup"; break;
    case BOBE_LIFECYCLE_JOINING_WIFI:
    case BOBE_LIFECYCLE_DISCOVERING_HUB:
    case BOBE_LIFECYCLE_CONNECTING:
    case BOBE_LIFECYCLE_RECONNECTING: text = "Waiting for Wi-Fi"; break;
    case BOBE_LIFECYCLE_ONLINE:
        text = state->microphone_muted ? "Hold -: mic"
                                      : state->turn == BOBE_TURN_IDLE ? "Hold O: talk" : NULL;
        break;
    case BOBE_LIFECYCLE_LOW_BATTERY: text = "Please charge me"; break;
    default: break;
    }
    if (text == NULL) return false;
    notice->kind = BOBE_NOTICE_HINT;
    notice->ttl_ms = BOBE_NOTICE_HINT_TTL_MS;
    set_text(notice, text);
    return true;
}

bool bobe_notice_update_confirmed(bobe_notice_t *notice)
{
    if (notice == NULL) return false;
    memset(notice, 0, sizeof(*notice));
    notice->kind = BOBE_NOTICE_UPDATE;
    notice->ttl_ms = BOBE_NOTICE_UPDATE_TTL_MS;
    set_text(notice, "Update installed");
    return true;
}
