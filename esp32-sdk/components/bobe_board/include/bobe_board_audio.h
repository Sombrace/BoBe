#pragma once

#include "driver/i2s_std.h"

/* The SDK consumes microphone PCM32 (upper 16 bits) and emits PCM16.
 * Both are mono Philips I2S on separate controllers; the port owns pins,
 * clock inversion, channel selection, and physical slot width.
 */
typedef struct {
    int microphone_port;
    int speaker_port;
    i2s_std_config_t microphone;
    i2s_std_config_t speaker;
} bobe_board_audio_config_t;

bobe_board_audio_config_t bobe_board_audio_config(void);
