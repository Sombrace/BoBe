#pragma once

#include "esp_err.h"
#include "freertos/FreeRTOS.h"

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef int i2s_port_t;

typedef enum {
    I2S_ROLE_MASTER,
    I2S_ROLE_SLAVE,
} i2s_role_t;

typedef struct i2s_channel_obj_t *i2s_chan_handle_t;

typedef struct {
    i2s_port_t id;
    i2s_role_t role;
    uint32_t dma_desc_num;
    uint32_t dma_frame_num;
    bool auto_clear_before_cb;
} i2s_chan_config_t;

#define I2S_CHANNEL_DEFAULT_CONFIG(i2s_num, i2s_role) \
    {                                                 \
        .id = (i2s_num),                              \
        .role = (i2s_role),                           \
        .dma_desc_num = 6,                            \
        .dma_frame_num = 240,                         \
        .auto_clear_before_cb = false,                \
    }

typedef struct {
    uint32_t sample_rate_hz;
} i2s_std_config_t;

typedef struct {
    void *dma_buf;
    size_t size;
} i2s_event_data_t;

typedef bool (*i2s_isr_callback_t)(
    i2s_chan_handle_t handle,
    i2s_event_data_t *event,
    void *user_ctx
);

typedef struct {
    i2s_isr_callback_t on_recv;
    i2s_isr_callback_t on_recv_q_ovf;
    i2s_isr_callback_t on_sent;
    i2s_isr_callback_t on_send_q_ovf;
} i2s_event_callbacks_t;

esp_err_t i2s_new_channel(
    const i2s_chan_config_t *chan_cfg,
    i2s_chan_handle_t *ret_tx_handle,
    i2s_chan_handle_t *ret_rx_handle
);
esp_err_t i2s_del_channel(i2s_chan_handle_t handle);
esp_err_t i2s_channel_init_std_mode(
    i2s_chan_handle_t handle,
    const i2s_std_config_t *std_cfg
);
esp_err_t i2s_channel_register_event_callback(
    i2s_chan_handle_t handle,
    const i2s_event_callbacks_t *callbacks,
    void *user_data
);
esp_err_t i2s_channel_enable(i2s_chan_handle_t handle);
esp_err_t i2s_channel_disable(i2s_chan_handle_t handle);
esp_err_t i2s_channel_read(
    i2s_chan_handle_t handle,
    void *dest,
    size_t size,
    size_t *bytes_read,
    uint32_t timeout_ms
);
esp_err_t i2s_channel_preload_data(
    i2s_chan_handle_t tx_handle,
    const void *src,
    size_t size,
    size_t *bytes_loaded
);
