#include "esp_heap_caps.h"
#include "lvgl.h"

#include <string.h>

#define BOBE_LVGL_MEMORY_CAPS (MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT)

void bobe_lvgl_memory_link(void)
{
}

void lv_mem_init(void)
{
}

void lv_mem_deinit(void)
{
}

lv_mem_pool_t lv_mem_add_pool(void *memory, size_t bytes)
{
    (void)memory;
    (void)bytes;
    return NULL;
}

void lv_mem_remove_pool(lv_mem_pool_t pool)
{
    (void)pool;
}

void *lv_malloc_core(size_t size)
{
    return heap_caps_malloc(size, BOBE_LVGL_MEMORY_CAPS);
}

void *lv_realloc_core(void *pointer, size_t size)
{
    return heap_caps_realloc(
        pointer,
        size,
        BOBE_LVGL_MEMORY_CAPS
    );
}

void lv_free_core(void *pointer)
{
    heap_caps_free(pointer);
}

void lv_mem_monitor_core(lv_mem_monitor_t *monitor)
{
    if (monitor == NULL) {
        return;
    }
    memset(monitor, 0, sizeof(*monitor));
    monitor->total_size = heap_caps_get_total_size(BOBE_LVGL_MEMORY_CAPS);
    monitor->free_size = heap_caps_get_free_size(BOBE_LVGL_MEMORY_CAPS);
    monitor->free_biggest_size =
        heap_caps_get_largest_free_block(BOBE_LVGL_MEMORY_CAPS);
    const size_t minimum_free =
        heap_caps_get_minimum_free_size(BOBE_LVGL_MEMORY_CAPS);
    monitor->max_used =
        monitor->total_size > minimum_free
            ? monitor->total_size - minimum_free
            : 0;
    monitor->used_pct =
        monitor->total_size > 0
            ? (uint8_t)(
                  100U -
                  monitor->free_size * 100U / monitor->total_size
              )
            : 0;
    monitor->frag_pct =
        monitor->free_size > 0
            ? (uint8_t)(
                  100U -
                  monitor->free_biggest_size * 100U / monitor->free_size
              )
            : 0;
}

lv_result_t lv_mem_test_core(void)
{
    return heap_caps_check_integrity(BOBE_LVGL_MEMORY_CAPS, false)
               ? LV_RESULT_OK
               : LV_RESULT_INVALID;
}
