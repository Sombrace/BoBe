#include "bobe_board_audio.h"
#include "bobe_types.h"

bobe_board_audio_config_t bobe_board_audio_config(void)
{
    bobe_board_audio_config_t config = {
        .microphone_port = I2S_NUM_1,
        .speaker_port = I2S_NUM_0,
        .microphone = {
            .clk_cfg = I2S_STD_CLK_DEFAULT_CONFIG(BOBE_MIC_SAMPLE_RATE_HZ),
            .slot_cfg = I2S_STD_PHILIPS_SLOT_DEFAULT_CONFIG(
                I2S_DATA_BIT_WIDTH_32BIT, I2S_SLOT_MODE_MONO),
            .gpio_cfg = {
                .mclk = I2S_GPIO_UNUSED, .bclk = GPIO_NUM_5,
                .ws = GPIO_NUM_4, .dout = I2S_GPIO_UNUSED, .din = GPIO_NUM_6,
            },
        },
        .speaker = {
            .clk_cfg = I2S_STD_CLK_DEFAULT_CONFIG(BOBE_SPEAKER_SAMPLE_RATE_HZ),
            .slot_cfg = I2S_STD_PHILIPS_SLOT_DEFAULT_CONFIG(
                I2S_DATA_BIT_WIDTH_16BIT, I2S_SLOT_MODE_MONO),
            .gpio_cfg = {
                .mclk = I2S_GPIO_UNUSED, .bclk = GPIO_NUM_15,
                .ws = GPIO_NUM_16, .dout = GPIO_NUM_7, .din = I2S_GPIO_UNUSED,
            },
        },
    };
    config.microphone.slot_cfg.slot_mask = I2S_STD_SLOT_LEFT;
    config.speaker.slot_cfg.slot_mask = I2S_STD_SLOT_LEFT;
    config.speaker.slot_cfg.slot_bit_width = I2S_SLOT_BIT_WIDTH_32BIT;
    config.speaker.slot_cfg.ws_width = 32;
    return config;
}
