#include "bobe_audio.h"
#include "bobe_board_audio.h"
#include "idf_mock.h"

#include <stdio.h>
#include <stdlib.h>

#define CHECK(condition)                                                     \
    do {                                                                     \
        if (!(condition)) {                                                  \
            fprintf(                                                        \
                stderr,                                                     \
                "check failed at %s:%d: %s\n",                              \
                __FILE__,                                                    \
                __LINE__,                                                    \
                #condition                                                   \
            );                                                               \
            exit(1);                                                         \
        }                                                                    \
    } while (0)

bobe_board_audio_config_t bobe_board_audio_config(void)
{
    return (bobe_board_audio_config_t){
        .microphone_port = 1,
        .speaker_port = 0,
        .microphone = {.sample_rate_hz = 16000},
        .speaker = {.sample_rate_hz = 24000},
    };
}

static void check_released(void)
{
    CHECK(idf_mock_live_tasks() == 0);
    CHECK(idf_mock_live_i2s_channels() == 0);
    CHECK(idf_mock_live_semaphores() == 0);
    CHECK(idf_mock_live_event_groups() == 0);
    CHECK(bobe_audio_start_monitor() == ESP_ERR_INVALID_STATE);
}

static void test_failed_init_releases_everything(void)
{
    for (unsigned int task = 1; task <= 2; ++task) {
        idf_mock_reset();
        idf_mock_fail_task_creation(task);
        CHECK(bobe_audio_init() == ESP_ERR_NO_MEM);
        check_released();
    }
    for (unsigned int channel = 1; channel <= 2; ++channel) {
        idf_mock_reset();
        idf_mock_fail_i2s_new_channel(channel);
        CHECK(bobe_audio_init() == ESP_ERR_NO_MEM);
        check_released();

        idf_mock_reset();
        idf_mock_fail_i2s_init(channel);
        CHECK(bobe_audio_init() == ESP_FAIL);
        check_released();
    }
}

static void test_init_after_failure(void)
{
    idf_mock_reset();
    CHECK(bobe_audio_init() == ESP_OK);
    CHECK(idf_mock_live_tasks() == 2);
    CHECK(idf_mock_task_alive("bobe_audio_rx"));
    CHECK(idf_mock_task_alive("bobe_audio_tx"));
    CHECK(idf_mock_live_i2s_channels() == 2);
    CHECK(idf_mock_running_i2s_rx_channels() == 0);
}

static void test_mute_unmute_capture_lifecycle(void)
{
    CHECK(bobe_audio_start_monitor() == ESP_OK);
    CHECK(idf_mock_running_i2s_rx_channels() == 1);
    bobe_audio_set_muted(true);
    CHECK(idf_mock_running_i2s_rx_channels() == 0);
    CHECK(bobe_audio_stop_capture() == ESP_OK);
    CHECK(bobe_audio_stop_monitor() == ESP_OK);
    CHECK(bobe_audio_start_monitor() == ESP_ERR_INVALID_STATE);
    CHECK(bobe_audio_start_capture(7) == ESP_ERR_INVALID_STATE);

    bobe_audio_set_muted(false);
    CHECK(bobe_audio_start_monitor() == ESP_OK);
    CHECK(bobe_audio_start_capture(7) == ESP_OK);
    CHECK(bobe_audio_is_capturing());
    CHECK(idf_mock_running_i2s_rx_channels() == 1);
    CHECK(bobe_audio_stop_capture() == ESP_OK);
    CHECK(idf_mock_running_i2s_rx_channels() == 1);
    CHECK(bobe_audio_stop_monitor() == ESP_OK);
    CHECK(idf_mock_running_i2s_rx_channels() == 0);

    CHECK(bobe_audio_start_capture(8) == ESP_OK);
    CHECK(idf_mock_running_i2s_rx_channels() == 1);
    bobe_audio_set_muted(true);
    CHECK(!bobe_audio_is_capturing());
    CHECK(idf_mock_running_i2s_rx_channels() == 0);
    CHECK(bobe_audio_stop_capture() == ESP_OK);

    bobe_audio_set_muted(false);
    CHECK(bobe_audio_start_capture(9) == ESP_OK);
    CHECK(idf_mock_running_i2s_rx_channels() == 1);
    CHECK(bobe_audio_stop_capture() == ESP_OK);
    CHECK(idf_mock_running_i2s_rx_channels() == 0);

    bobe_audio_set_muted(true);
    bobe_audio_set_muted(false);
    CHECK(bobe_audio_start_monitor() == ESP_OK);
    CHECK(bobe_audio_stop_monitor() == ESP_OK);
    CHECK(idf_mock_i2s_rejected_enables() == 0);
}

int main(void)
{
    test_failed_init_releases_everything();
    test_init_after_failure();
    test_mute_unmute_capture_lifecycle();
    puts("BoBe audio lifecycle tests passed.");
    return 0;
}
