#include "bobe_text_view.h"

#include <string.h>

static size_t next_boundary(const char *text, size_t length, size_t start)
{
    while (start < length && ((unsigned char)text[start] & 0xC0U) == 0x80U) ++start;
    return start;
}

bool bobe_text_view_init(bobe_text_view_t *view, char *buffer, size_t capacity)
{
    if (view == NULL || buffer == NULL || capacity < 5U) return false;
    *view = (bobe_text_view_t){.buffer = buffer, .capacity = capacity};
    buffer[0] = '\0';
    return true;
}

bobe_text_result_t bobe_text_view_apply(bobe_text_view_t *view, const bobe_control_message_t *message)
{
    if (view == NULL || view->buffer == NULL || view->capacity < 5U ||
        view->length >= view->capacity || message == NULL ||
        message->kind != BOBE_CONTROL_DISPLAY_TEXT) return BOBE_TEXT_INVALID;
    const size_t length = strnlen(message->data.display_text.text, sizeof(message->data.display_text.text));
    if (length == sizeof(message->data.display_text.text) ||
        strnlen(message->data.display_text.request_id, sizeof(message->data.display_text.request_id)) != 36U ||
        !bobe_utf8_valid(message->data.display_text.text, length)) return BOBE_TEXT_INVALID;
    if (message->data.display_text.clear) {
        if (message->data.display_text.sequence != 0) return BOBE_TEXT_INVALID;
        view->length = 0;
        view->buffer[0] = '\0';
        view->next_sequence = 0;
        view->complete = false;
        view->truncated = false;
        memcpy(view->request_id, message->data.display_text.request_id, sizeof(view->request_id));
    } else if (strcmp(view->request_id, message->data.display_text.request_id) != 0) {
        return BOBE_TEXT_OTHER_REQUEST;
    }
    if (message->data.display_text.sequence < view->next_sequence) return BOBE_TEXT_DUPLICATE;
    if (message->data.display_text.sequence > view->next_sequence) return BOBE_TEXT_SEQUENCE_GAP;
    if (view->complete) return BOBE_TEXT_INVALID;

    const char *text = message->data.display_text.text;
    const size_t room = view->capacity - 1U;
    size_t skip = 0;
    if (length > room) {
        skip = next_boundary(text, length, length - room);
        view->length = 0;
        view->truncated = true;
    } else if (view->length + length > room) {
        const size_t discard = next_boundary(view->buffer, view->length, view->length + length - room);
        memmove(view->buffer, view->buffer + discard, view->length - discard);
        view->length -= discard;
        view->truncated = true;
    }
    memcpy(view->buffer + view->length, text + skip, length - skip);
    view->length += length - skip;
    view->buffer[view->length] = '\0';
    view->next_sequence = (uint64_t)message->data.display_text.sequence + 1U;
    view->complete = message->data.display_text.final;
    return BOBE_TEXT_APPLIED;
}
