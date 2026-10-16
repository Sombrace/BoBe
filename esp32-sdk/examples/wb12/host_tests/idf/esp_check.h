#pragma once

#include "esp_err.h"
#include "esp_log.h"

#define ESP_RETURN_ON_ERROR(x, log_tag, ...)              \
    do {                                                  \
        const esp_err_t idf_mock_check_result = (x);      \
        if (idf_mock_check_result != ESP_OK) {            \
            idf_mock_log(log_tag, __VA_ARGS__);           \
            return idf_mock_check_result;                 \
        }                                                 \
    } while (0)
