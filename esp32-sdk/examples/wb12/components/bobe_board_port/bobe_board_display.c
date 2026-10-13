#include "bobe_board_display.h"
#include "driver/spi_master.h"
#include "esp_lcd_io_spi.h"
#include "esp_lcd_panel_st7789.h"
#include "esp_check.h"
#include <limits.h>

static const char *TAG = "wb12_display";

static esp_err_t initialize_panel(
    size_t max_transfer_bytes,
    esp_lcd_panel_io_handle_t *io,
    esp_lcd_panel_handle_t *panel_handle
)
{
    const spi_bus_config_t bus_config = {
        .sclk_io_num = GPIO_NUM_9,
        .mosi_io_num = GPIO_NUM_10,
        .miso_io_num = GPIO_NUM_NC,
        .quadwp_io_num = GPIO_NUM_NC,
        .quadhd_io_num = GPIO_NUM_NC,
        .max_transfer_sz =
            (int)max_transfer_bytes,
    };
    ESP_RETURN_ON_ERROR(
        spi_bus_initialize(SPI3_HOST, &bus_config, SPI_DMA_CH_AUTO),
        TAG,
        "initialize display SPI bus"
    );

    const esp_lcd_panel_io_spi_config_t io_config = {
        .cs_gpio_num = GPIO_NUM_14,
        .dc_gpio_num = GPIO_NUM_8,
        .spi_mode = 3,
        .pclk_hz = 40 * 1000 * 1000,
        .trans_queue_depth = 3,
        .lcd_cmd_bits = 8,
        .lcd_param_bits = 8,
    };
    ESP_RETURN_ON_ERROR(
        esp_lcd_new_panel_io_spi(
            (esp_lcd_spi_bus_handle_t)SPI3_HOST,
            &io_config,
            io
        ),
        TAG,
        "create panel IO"
    );

    const esp_lcd_panel_dev_config_t panel_config = {
        .rgb_ele_order = LCD_RGB_ELEMENT_ORDER_RGB,
        .data_endian = LCD_RGB_DATA_ENDIAN_BIG,
        .bits_per_pixel = 16,
        .reset_gpio_num = GPIO_NUM_18,
    };
    ESP_RETURN_ON_ERROR(
        esp_lcd_new_panel_st7789(*io, &panel_config, panel_handle),
        TAG,
        "create ST7789 panel"
    );
    ESP_RETURN_ON_ERROR(
        esp_lcd_panel_reset(*panel_handle),
        TAG,
        "reset panel"
    );
    ESP_RETURN_ON_ERROR(
        esp_lcd_panel_init(*panel_handle),
        TAG,
        "initialize panel"
    );
    ESP_RETURN_ON_ERROR(
        esp_lcd_panel_swap_xy(*panel_handle, true),
        TAG,
        "swap panel axes"
    );
    ESP_RETURN_ON_ERROR(
        esp_lcd_panel_mirror(*panel_handle, true, false),
        TAG,
        "mirror panel"
    );
    ESP_RETURN_ON_ERROR(
        esp_lcd_panel_set_gap(*panel_handle, 20, 0),
        TAG,
        "set panel gap"
    );
    ESP_RETURN_ON_ERROR(
        esp_lcd_panel_invert_color(*panel_handle, true),
        TAG,
        "invert panel colors"
    );
    return esp_lcd_panel_disp_on_off(*panel_handle, true);
}

esp_err_t bobe_board_create_display(
    size_t max_transfer_bytes,
    bobe_board_display_t *display
)
{
    if (display == NULL || max_transfer_bytes == 0 || max_transfer_bytes > INT_MAX) {
        return ESP_ERR_INVALID_ARG;
    }
    *display = (bobe_board_display_t){
        .swap_xy = true, .mirror_x = true, .mirror_y = false,
        .swap_color_bytes = true,
    };
    return initialize_panel(max_transfer_bytes, &display->io, &display->panel);
}
