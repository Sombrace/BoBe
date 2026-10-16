#pragma once

#include "freertos/FreeRTOS.h"

typedef struct idf_mock_queue *QueueHandle_t;

typedef struct {
    void *reserved[8];
} StaticQueue_t;

QueueHandle_t xQueueCreateStatic(
    UBaseType_t length,
    UBaseType_t item_size,
    uint8_t *storage,
    StaticQueue_t *queue_buffer
);
BaseType_t xQueueSend(QueueHandle_t queue, const void *item, TickType_t ticks_to_wait);
BaseType_t xQueueReceive(QueueHandle_t queue, void *item, TickType_t ticks_to_wait);
BaseType_t xQueueReset(QueueHandle_t queue);
UBaseType_t uxQueueMessagesWaiting(QueueHandle_t queue);
void vQueueDelete(QueueHandle_t queue);
