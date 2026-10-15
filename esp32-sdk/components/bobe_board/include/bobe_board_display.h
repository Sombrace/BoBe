#pragma once

#include "esp_lcd_panel_io.h"
#include "esp_lcd_panel_ops.h"
#include <stdbool.h>
#include <stddef.h>

typedef struct {
    esp_lcd_panel_io_handle_t io;
    esp_lcd_panel_handle_t panel;
    bool swap_xy;
    bool mirror_x;
    bool mirror_y;
    bool swap_color_bytes;
} bobe_board_display_t;

esp_err_t bobe_board_create_display(
    size_t max_transfer_bytes,
    bobe_board_display_t *display
);
