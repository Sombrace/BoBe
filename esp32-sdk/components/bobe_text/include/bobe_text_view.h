#pragma once

#include "bobe_protocol.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef enum {
    BOBE_TEXT_APPLIED,
    BOBE_TEXT_DUPLICATE,
    BOBE_TEXT_OTHER_REQUEST,
    BOBE_TEXT_SEQUENCE_GAP,
    BOBE_TEXT_INVALID,
} bobe_text_result_t;

typedef struct {
    char *buffer;
    size_t capacity;
    size_t length;
    char request_id[37];
    uint64_t next_sequence;
    bool complete;
    bool truncated;
} bobe_text_view_t;

bool bobe_text_view_init(bobe_text_view_t *view, char *buffer, size_t capacity);
bobe_text_result_t bobe_text_view_apply(bobe_text_view_t *view, const bobe_control_message_t *message);

#ifdef __cplusplus
}
#endif
