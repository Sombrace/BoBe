#include "bobe_text_view.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define CHECK(test) do { if (!(test)) { fprintf(stderr, "line %d: %s\n", __LINE__, #test); return EXIT_FAILURE; } } while (0)

int main(void)
{
    char buffer[13];
    bobe_text_view_t view;
    CHECK(bobe_text_view_init(&view, buffer, sizeof(buffer)));
    bobe_control_message_t frame = {.kind = BOBE_CONTROL_DISPLAY_TEXT};
    strcpy(frame.data.display_text.request_id, "00000000-0000-4000-8000-000000000001");
    strcpy(frame.data.display_text.text, "Hello 👋");
    frame.data.display_text.clear = true;
    CHECK(bobe_text_view_apply(&view, &frame) == BOBE_TEXT_APPLIED);
    CHECK(strcmp(buffer, "Hello 👋") == 0);
    frame.data.display_text.clear = false;
    frame.data.display_text.sequence = 2;
    CHECK(bobe_text_view_apply(&view, &frame) == BOBE_TEXT_SEQUENCE_GAP);
    frame.data.display_text.sequence = 1;
    strcpy(frame.data.display_text.text, " world");
    CHECK(bobe_text_view_apply(&view, &frame) == BOBE_TEXT_APPLIED);
    CHECK(view.truncated && view.length < sizeof(buffer));
    CHECK(bobe_utf8_valid(buffer, view.length));
    CHECK(bobe_text_view_apply(&view, &frame) == BOBE_TEXT_DUPLICATE);
    frame.data.display_text.sequence = 2;
    frame.data.display_text.text[0] = '\0';
    frame.data.display_text.final = true;
    CHECK(bobe_text_view_apply(&view, &frame) == BOBE_TEXT_APPLIED && view.complete);
    frame.data.display_text.request_id[35] = '2';
    CHECK(bobe_text_view_apply(&view, &frame) == BOBE_TEXT_OTHER_REQUEST);
    frame.data.display_text.sequence = 0;
    frame.data.display_text.clear = true;
    strcpy(frame.data.display_text.text, "New screen");
    CHECK(bobe_text_view_apply(&view, &frame) == BOBE_TEXT_APPLIED);
    CHECK(strcmp(buffer, "New screen") == 0 && !view.truncated);
    return EXIT_SUCCESS;
}
