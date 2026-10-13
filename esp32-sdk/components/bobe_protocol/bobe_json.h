#pragma once

#include "bobe_protocol.h"
#include "cJSON.h"

#include <stdbool.h>
#include <stddef.h>

/* Bound recursive parser work before entering cJSON on a small RTOS stack. */
bool bobe_json_within_limits(const char *input, size_t length);
bool bobe_uuid_valid(const char *value);
bool bobe_name_valid(const char *value, size_t max_length);
bobe_protocol_result_t bobe_json_print(cJSON *root, char *output, size_t capacity);
