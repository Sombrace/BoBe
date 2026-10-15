#pragma once

#include "bobe_types.h"
#include "esp_err.h"

#include <stdint.h>

typedef void (*bobe_button_callback_t)(
    const bobe_button_event_t *event,
    void *context
);

esp_err_t bobe_board_early_init(void);
esp_err_t bobe_board_init(
    bobe_button_callback_t button_callback,
    void *button_context
);
esp_err_t bobe_board_set_backlight(uint8_t percent);
esp_err_t bobe_board_read_battery(bobe_battery_state_t *state);
esp_err_t bobe_board_read_temperature(float *celsius);
bool bobe_board_button_is_pressed(bobe_button_id_t button);
void bobe_board_request_shutdown(void);

const unsigned char *bobe_board_update_public_key(size_t *length);
