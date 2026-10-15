#pragma once

#include "driver/i2s_std.h"

typedef struct {
    int microphone_port;
    int speaker_port;
    i2s_std_config_t microphone;
    i2s_std_config_t speaker;
} bobe_board_audio_config_t;

bobe_board_audio_config_t bobe_board_audio_config(void);
