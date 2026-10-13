#pragma once

#include "esp_err.h"

#ifdef __cplusplus
extern "C" {
#endif

/** Retain board power and schedule the body runtime once.
 * ESP_OK means startup was scheduled; readiness is reported by the body state.
 * Board implementations supply the bobe_board and bobe_device_config contracts.
 */
esp_err_t esp_bobe_start(void);

#ifdef __cplusplus
}
#endif
