#pragma once

#include <stddef.h>
#include <stdint.h>

#define MALLOC_CAP_8BIT (1U << 2)
#define MALLOC_CAP_INTERNAL (1U << 11)
#define MALLOC_CAP_SPIRAM (1U << 10)

void *heap_caps_malloc(size_t size, uint32_t caps);
void heap_caps_free(void *pointer);
