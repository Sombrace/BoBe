#pragma once

#include "esp_afe_config.h"
#include "freertos/FreeRTOS.h"

#include <stdint.h>

typedef struct esp_afe_sr_data_t esp_afe_sr_data_t;

typedef struct afe_fetch_result_t {
    int16_t *data;
    int data_size;
    float data_volume;
    wakenet_state_t wakeup_state;
    int wake_word_index;
    vad_state_t vad_state;
} afe_fetch_result_t;

typedef struct {
    esp_afe_sr_data_t *(*create_from_config)(afe_config_t *afe_config);
    int (*feed)(esp_afe_sr_data_t *afe, const int16_t *in);
    afe_fetch_result_t *(*fetch_with_delay)(
        esp_afe_sr_data_t *afe,
        TickType_t ticks_to_wait
    );
    int (*reset_buffer)(esp_afe_sr_data_t *afe);
    int (*get_feed_chunksize)(esp_afe_sr_data_t *afe);
    int (*get_feed_channel_num)(esp_afe_sr_data_t *afe);
    int (*get_samp_rate)(esp_afe_sr_data_t *afe);
    void (*print_pipeline)(esp_afe_sr_data_t *afe);
    void (*destroy)(esp_afe_sr_data_t *afe);
} esp_afe_sr_iface_t;
