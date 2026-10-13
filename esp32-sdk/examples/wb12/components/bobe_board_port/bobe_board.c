#include "bobe_board.h"

#include "bobe_battery.h"
#include "bobe_task_config.h"
#include "driver/gpio.h"
#include "driver/ledc.h"
#include "driver/rtc_io.h"
#include "esp_adc/adc_cali.h"
#include "esp_adc/adc_cali_scheme.h"
#include "esp_adc/adc_oneshot.h"
#include "driver/temperature_sensor.h"
#include "esp_check.h"
#include "esp_log.h"
#include "esp_sleep.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

#include <stdbool.h>

#define BOBE_POWER_HOLD_GPIO GPIO_NUM_21
#define BOBE_BACKLIGHT_GPIO GPIO_NUM_13
#define BOBE_CHARGE_STATUS_GPIO GPIO_NUM_38
#define BOBE_BATTERY_ADC_UNIT ADC_UNIT_2
#define BOBE_BATTERY_ADC_CHANNEL ADC_CHANNEL_6
#define BOBE_BUTTON_POLL_MS 10U
#define BOBE_BUTTON_DEBOUNCE_SAMPLES 3U
#define BOBE_BUTTON_LONG_PRESS_MS 1200U
#define BOBE_BACKLIGHT_RESOLUTION LEDC_TIMER_10_BIT
#define BOBE_BACKLIGHT_MAX_DUTY ((1U << BOBE_BACKLIGHT_RESOLUTION) - 1U)

static const char *TAG = "bobe_board";

typedef struct {
    gpio_num_t gpio;
    bobe_button_id_t id;
    int stable_level;
    int sampled_level;
    uint8_t matching_samples;
    bool long_press_sent;
    uint64_t pressed_at_us;
} button_state_t;

static button_state_t buttons[] = {
    {.gpio = GPIO_NUM_0, .id = BOBE_BUTTON_ACTION},
    {.gpio = GPIO_NUM_39, .id = BOBE_BUTTON_VOLUME_DOWN},
    {.gpio = GPIO_NUM_40, .id = BOBE_BUTTON_VOLUME_UP},
};

#define BOBE_BUTTON_COUNT (sizeof(buttons) / sizeof(buttons[0]))

static adc_oneshot_unit_handle_t adc_handle;
static adc_cali_handle_t adc_calibration;
static temperature_sensor_handle_t temperature_sensor;
static bool adc_calibrated;
static bobe_button_callback_t button_callback;
static void *button_callback_context;
static uint32_t button_event_id;

static const button_state_t *find_button(bobe_button_id_t id)
{
    for (size_t index = 0;
         index < BOBE_BUTTON_COUNT;
         ++index) {
        if (buttons[index].id == id) {
            return &buttons[index];
        }
    }
    return NULL;
}

static void emit_button(
    const button_state_t *button,
    bobe_button_action_t action,
    uint64_t now_us
)
{
    if (button_callback == NULL) {
        return;
    }
    const bobe_button_event_t event = {
        .button = button->id,
        .action = action,
        .event_id = ++button_event_id,
        .monotonic_us = now_us,
    };
    button_callback(&event, button_callback_context);
}

static void button_task(void *unused)
{
    (void)unused;
    for (;;) {
        const uint64_t now_us = esp_timer_get_time();
        for (size_t index = 0; index < BOBE_BUTTON_COUNT; ++index) {
            button_state_t *button = &buttons[index];
            const int level = gpio_get_level(button->gpio);
            if (level == button->sampled_level) {
                if (button->matching_samples < BOBE_BUTTON_DEBOUNCE_SAMPLES) {
                    ++button->matching_samples;
                }
            } else {
                button->sampled_level = level;
                button->matching_samples = 1;
            }

            if (button->matching_samples >= BOBE_BUTTON_DEBOUNCE_SAMPLES &&
                button->stable_level != level) {
                button->stable_level = level;
                if (level == 0) {
                    button->pressed_at_us = now_us;
                    button->long_press_sent = false;
                    emit_button(button, BOBE_BUTTON_PRESSED, now_us);
                } else {
                    emit_button(button, BOBE_BUTTON_RELEASED, now_us);
                }
            }

            if (button->stable_level == 0 && !button->long_press_sent &&
                now_us - button->pressed_at_us >=
                    BOBE_BUTTON_LONG_PRESS_MS * UINT64_C(1000)) {
                button->long_press_sent = true;
                emit_button(button, BOBE_BUTTON_LONG_PRESS, now_us);
            }
        }
        vTaskDelay(pdMS_TO_TICKS(BOBE_BUTTON_POLL_MS));
    }
}

esp_err_t bobe_board_early_init(void)
{
    ESP_RETURN_ON_ERROR(
        rtc_gpio_init(BOBE_POWER_HOLD_GPIO),
        TAG,
        "initialize power hold"
    );
    ESP_RETURN_ON_ERROR(
        rtc_gpio_set_direction(
            BOBE_POWER_HOLD_GPIO,
            RTC_GPIO_MODE_OUTPUT_ONLY
        ),
        TAG,
        "configure power hold"
    );
    ESP_RETURN_ON_ERROR(
        rtc_gpio_set_level(BOBE_POWER_HOLD_GPIO, 1),
        TAG,
        "preset power hold"
    );
    ESP_RETURN_ON_ERROR(
        rtc_gpio_hold_dis(BOBE_POWER_HOLD_GPIO),
        TAG,
        "release retained power hold"
    );

    const gpio_config_t input_config = {
        .pin_bit_mask = BIT64(GPIO_NUM_0) |
                        BIT64(GPIO_NUM_38) |
                        BIT64(GPIO_NUM_39) |
                        BIT64(GPIO_NUM_40),
        .mode = GPIO_MODE_INPUT,
        .pull_up_en = GPIO_PULLUP_ENABLE,
        .pull_down_en = GPIO_PULLDOWN_DISABLE,
        .intr_type = GPIO_INTR_DISABLE,
    };
    return gpio_config(&input_config);
}

static esp_err_t init_backlight(void)
{
    const ledc_timer_config_t timer_config = {
        .speed_mode = LEDC_LOW_SPEED_MODE,
        .duty_resolution = BOBE_BACKLIGHT_RESOLUTION,
        .timer_num = LEDC_TIMER_0,
        .freq_hz = 25000,
        .clk_cfg = LEDC_USE_XTAL_CLK,
    };
    ESP_RETURN_ON_ERROR(
        ledc_timer_config(&timer_config),
        TAG,
        "configure backlight timer"
    );

    const ledc_channel_config_t channel_config = {
        .gpio_num = BOBE_BACKLIGHT_GPIO,
        .speed_mode = LEDC_LOW_SPEED_MODE,
        .channel = LEDC_CHANNEL_0,
        .timer_sel = LEDC_TIMER_0,
        .duty = 0,
        .hpoint = 0,
        .sleep_mode = LEDC_SLEEP_MODE_NO_ALIVE_NO_PD,
        .flags.output_invert = 0,
    };
    return ledc_channel_config(&channel_config);
}

static esp_err_t init_battery_adc(void)
{
    const adc_oneshot_unit_init_cfg_t unit_config = {
        .unit_id = BOBE_BATTERY_ADC_UNIT,
        .clk_src = ADC_RTC_CLK_SRC_DEFAULT,
        .ulp_mode = ADC_ULP_MODE_DISABLE,
    };
    ESP_RETURN_ON_ERROR(
        adc_oneshot_new_unit(&unit_config, &adc_handle),
        TAG,
        "create ADC unit"
    );

    const adc_oneshot_chan_cfg_t channel_config = {
        .atten = ADC_ATTEN_DB_12,
        .bitwidth = ADC_BITWIDTH_12,
    };
    ESP_RETURN_ON_ERROR(
        adc_oneshot_config_channel(
            adc_handle,
            BOBE_BATTERY_ADC_CHANNEL,
            &channel_config
        ),
        TAG,
        "configure battery ADC"
    );

#if ADC_CALI_SCHEME_CURVE_FITTING_SUPPORTED
    const adc_cali_curve_fitting_config_t calibration_config = {
        .unit_id = BOBE_BATTERY_ADC_UNIT,
        .chan = BOBE_BATTERY_ADC_CHANNEL,
        .atten = ADC_ATTEN_DB_12,
        .bitwidth = ADC_BITWIDTH_12,
    };
    adc_calibrated =
        adc_cali_create_scheme_curve_fitting(
            &calibration_config,
            &adc_calibration
        ) == ESP_OK;
#endif
    return ESP_OK;
}

static esp_err_t init_temperature_sensor(void)
{
    const temperature_sensor_config_t config =
        TEMPERATURE_SENSOR_CONFIG_DEFAULT(-10, 80);
    ESP_RETURN_ON_ERROR(
        temperature_sensor_install(&config, &temperature_sensor),
        TAG,
        "install temperature sensor"
    );
    return temperature_sensor_enable(temperature_sensor);
}

esp_err_t bobe_board_init(
    bobe_button_callback_t callback,
    void *context
)
{
    button_callback = callback;
    button_callback_context = context;

    ESP_RETURN_ON_ERROR(init_backlight(), TAG, "initialize backlight");
    ESP_RETURN_ON_ERROR(init_battery_adc(), TAG, "initialize battery ADC");
    ESP_RETURN_ON_ERROR(
        init_temperature_sensor(),
        TAG,
        "initialize temperature sensor"
    );

    for (size_t index = 0; index < BOBE_BUTTON_COUNT; ++index) {
        buttons[index].stable_level = gpio_get_level(buttons[index].gpio);
        buttons[index].sampled_level = buttons[index].stable_level;
        buttons[index].matching_samples = BOBE_BUTTON_DEBOUNCE_SAMPLES;
        if (buttons[index].stable_level == 0) {
            buttons[index].pressed_at_us = (uint64_t)esp_timer_get_time();
        }
    }

    const BaseType_t created = xTaskCreatePinnedToCore(
        button_task,
        "bobe_buttons",
        3072,
        NULL,
        BOBE_TASK_PRIORITY_BACKGROUND,
        NULL,
        BOBE_REALTIME_CORE
    );
    return created == pdPASS ? ESP_OK : ESP_ERR_NO_MEM;
}

esp_err_t bobe_board_set_backlight(uint8_t percent)
{
    if (percent > 100) {
        return ESP_ERR_INVALID_ARG;
    }
    const uint32_t duty = (BOBE_BACKLIGHT_MAX_DUTY * percent) / 100U;
    ESP_RETURN_ON_ERROR(
        ledc_set_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_0, duty),
        TAG,
        "set backlight duty"
    );
    return ledc_update_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_0);
}

esp_err_t bobe_board_read_battery(bobe_battery_state_t *state)
{
    if (state == NULL || adc_handle == NULL) {
        return ESP_ERR_INVALID_ARG;
    }

    int raw_total = 0;
    const int samples = 16;
    for (int sample = 0; sample < samples; ++sample) {
        int raw = 0;
        ESP_RETURN_ON_ERROR(
            adc_oneshot_read(adc_handle, BOBE_BATTERY_ADC_CHANNEL, &raw),
            TAG,
            "read battery ADC"
        );
        raw_total += raw;
    }

    state->raw_adc = raw_total / samples;
    state->percent = bobe_battery_percent_from_raw(state->raw_adc);
    state->charging = gpio_get_level(BOBE_CHARGE_STATUS_GPIO) == 0;

    if (adc_calibrated) {
        int millivolts = 0;
        if (adc_cali_raw_to_voltage(
                adc_calibration,
                state->raw_adc,
                &millivolts
            ) == ESP_OK) {
            ESP_LOGD(TAG, "battery divider input: %d mV", millivolts);
        }
    }
    return ESP_OK;
}

esp_err_t bobe_board_read_temperature(float *celsius)
{
    if (celsius == NULL || temperature_sensor == NULL) {
        return ESP_ERR_INVALID_ARG;
    }
    return temperature_sensor_get_celsius(temperature_sensor, celsius);
}

bool bobe_board_button_is_pressed(bobe_button_id_t button)
{
    const button_state_t *state = find_button(button);
    return state != NULL && gpio_get_level(state->gpio) == 0;
}

void bobe_board_request_shutdown(void)
{
    ESP_LOGI(TAG, "shutdown requested");
    (void)bobe_board_set_backlight(0);
    vTaskDelay(pdMS_TO_TICKS(100));
    ESP_ERROR_CHECK_WITHOUT_ABORT(rtc_gpio_init(BOBE_POWER_HOLD_GPIO));
    ESP_ERROR_CHECK_WITHOUT_ABORT(
        rtc_gpio_set_direction(
            BOBE_POWER_HOLD_GPIO,
            RTC_GPIO_MODE_OUTPUT_ONLY
        )
    );
    ESP_ERROR_CHECK_WITHOUT_ABORT(
        rtc_gpio_set_level(BOBE_POWER_HOLD_GPIO, 0)
    );
    ESP_ERROR_CHECK_WITHOUT_ABORT(rtc_gpio_hold_en(BOBE_POWER_HOLD_GPIO));
    esp_deep_sleep_start();
}
