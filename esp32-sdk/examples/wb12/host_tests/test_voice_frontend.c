#include "bobe_voice_frontend.h"
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

static bool output_sink(const bobe_audio_frame_t *frame, void *context)
{
    (void)frame;
    (void)context;
    return true;
}

static void check_released(void)
{
    CHECK(idf_mock_live_tasks() == 0);
    CHECK(idf_mock_live_queues() == 0);
    CHECK(idf_mock_live_semaphores() == 0);
    CHECK(idf_mock_live_event_groups() == 0);
    CHECK(idf_mock_live_allocations() == 0);
    CHECK(!idf_mock_afe_created());
    CHECK(!idf_mock_speech_models_loaded());
    CHECK(!bobe_voice_frontend_wake_available());
    CHECK(bobe_voice_frontend_begin_stream(1) == ESP_ERR_INVALID_STATE);
}

static void test_failed_init_releases_everything(void)
{
    for (unsigned int task = 1; task <= 2; ++task) {
        idf_mock_reset();
        idf_mock_provide_wake_model(true);
        idf_mock_fail_task_creation(task);
        CHECK(
            bobe_voice_frontend_init(output_sink, NULL, NULL, NULL) ==
            ESP_ERR_NO_MEM
        );
        check_released();
    }

    idf_mock_reset();
    idf_mock_fail_afe_creation(true);
    CHECK(bobe_voice_frontend_init(output_sink, NULL, NULL, NULL) == ESP_FAIL);
    check_released();
}

static void test_init_after_failure(void)
{
    idf_mock_reset();
    idf_mock_provide_wake_model(true);
    CHECK(bobe_voice_frontend_init(output_sink, NULL, NULL, NULL) == ESP_OK);
    CHECK(idf_mock_live_tasks() == 2);
    CHECK(idf_mock_task_alive("bobe_afe_feed"));
    CHECK(idf_mock_task_alive("bobe_afe_fetch"));
    CHECK(idf_mock_live_queues() == 1);
    CHECK(idf_mock_afe_created());
    CHECK(bobe_voice_frontend_wake_available());
}

int main(void)
{
    test_failed_init_releases_everything();
    test_init_after_failure();
    puts("BoBe voice front end lifecycle tests passed.");
    return 0;
}
