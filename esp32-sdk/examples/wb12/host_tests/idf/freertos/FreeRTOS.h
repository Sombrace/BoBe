#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef int BaseType_t;
typedef unsigned int UBaseType_t;
typedef uint32_t TickType_t;

typedef struct {
    int owner;
} portMUX_TYPE;

#define portMUX_INITIALIZER_UNLOCKED {0}
#define pdFALSE ((BaseType_t)0)
#define pdTRUE ((BaseType_t)1)
#define pdPASS pdTRUE
#define pdFAIL pdFALSE
#define portMAX_DELAY ((TickType_t)0xffffffffUL)
#define pdMS_TO_TICKS(ms) ((TickType_t)(ms))
#define portENTER_CRITICAL(mux) ((void)(mux))
#define portEXIT_CRITICAL(mux) ((void)(mux))
#define portENTER_CRITICAL_ISR(mux) ((void)(mux))
#define portEXIT_CRITICAL_ISR(mux) ((void)(mux))
#define tskIDLE_PRIORITY 0
#define BIT0 0x00000001U
#define BIT1 0x00000002U
