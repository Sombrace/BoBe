#pragma once

#include "freertos/FreeRTOS.h"

typedef struct idf_mock_task *TaskHandle_t;
typedef void (*TaskFunction_t)(void *parameters);

BaseType_t xTaskCreatePinnedToCore(
    TaskFunction_t function,
    const char *name,
    uint32_t stack_depth,
    void *parameters,
    UBaseType_t priority,
    TaskHandle_t *created_task,
    BaseType_t core_id
);
BaseType_t xTaskCreatePinnedToCoreWithCaps(
    TaskFunction_t function,
    const char *name,
    uint32_t stack_depth,
    void *parameters,
    UBaseType_t priority,
    TaskHandle_t *created_task,
    BaseType_t core_id,
    UBaseType_t memory_caps
);
void vTaskDelete(TaskHandle_t task);
void vTaskDeleteWithCaps(TaskHandle_t task);
uint32_t ulTaskNotifyTake(BaseType_t clear_count_on_exit, TickType_t ticks_to_wait);
BaseType_t xTaskNotifyGive(TaskHandle_t task);
void vTaskNotifyGiveFromISR(TaskHandle_t task, BaseType_t *higher_priority_task_woken);
void vTaskDelay(TickType_t ticks);
TickType_t xTaskGetTickCount(void);
