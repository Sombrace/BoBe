#pragma once

#include "esp_wn_models.h"
#include "model_path.h"

#include <stdbool.h>

typedef enum {
    AFE_MODE_LOW_COST = 0,
    AFE_MODE_HIGH_PERF = 1,
} afe_mode_t;

typedef enum {
    AFE_TYPE_SR = 0,
} afe_type_t;

typedef enum {
    AFE_MEMORY_ALLOC_MORE_INTERNAL = 1,
    AFE_MEMORY_ALLOC_INTERNAL_PSRAM_BALANCE = 2,
    AFE_MEMORY_ALLOC_MORE_PSRAM = 3,
} afe_memory_alloc_mode_t;

typedef enum {
    AFE_NS_MODE_WEBRTC = 0,
    AFE_NS_MODE_NET = 1,
} afe_ns_mode_t;

typedef enum {
    DET_MODE_90 = 0,
    DET_MODE_95 = 1,
} det_mode_t;

typedef enum {
    VAD_SILENCE = 0,
    VAD_SPEECH = 1,
} vad_state_t;

typedef enum {
    WAKENET_NO_DETECT = 0,
    WAKENET_CHANNEL_VERIFIED = -1,
    WAKENET_DETECTED = 1,
} wakenet_state_t;

typedef struct {
    bool aec_init;
    bool se_init;
    bool ns_init;
    char *ns_model_name;
    afe_ns_mode_t afe_ns_mode;
    bool vad_init;
    char *vad_model_name;
    int vad_min_speech_ms;
    int vad_min_noise_ms;
    int vad_delay_ms;
    bool vad_mute_playback;
    bool wakenet_init;
    char *wakenet_model_name;
    det_mode_t wakenet_mode;
    bool agc_init;
    int agc_compression_gain_db;
    int agc_target_level_dbfs;
    int afe_perferred_core;
    int afe_perferred_priority;
    afe_memory_alloc_mode_t memory_alloc_mode;
    bool fixed_output_channel;
} afe_config_t;

afe_config_t *afe_config_init(
    const char *input_format,
    srmodel_list_t *models,
    afe_type_t type,
    afe_mode_t mode
);
void afe_config_free(afe_config_t *afe_config);
