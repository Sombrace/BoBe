#include "hardware/watchdog.h"
#include "pico/stdlib.h"
#include "pico/unique_id.h"

#include <ctype.h>
#include <inttypes.h>
#include <stdbool.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define FIXTURE_PROTOCOL_VERSION 1U
#define FIXTURE_FIRMWARE_VERSION "0.3.0"
#define ACTION_PIN 2U
#define VOLUME_DOWN_PIN 3U
#define VOLUME_UP_PIN 4U
#define RESET_PIN 5U
#define LOADER_ENABLE_PIN 6U
#define LINE_CAPACITY 192U
#define TOKEN_CAPACITY 8U
#define RESPONSE_CAPACITY 256U
#define MIN_PRESS_MS 30U
#define MAX_PRESS_MS 2000U
#define MIN_RESET_MS 50U
#define MAX_RESET_MS 500U
#define RESET_SETTLE_MS 200U
#define ACTION_POST_RESET_LOCKOUT_MS 500U
#define SESSION_TIMEOUT_MS 30000U
#define LOADER_ACTION_SETTLE_MS 50U
#define LOADER_RESET_MS 120U
#define LOADER_STRAP_HOLD_MS 100U
#define LOADER_MAX_ACTIVE_MS 1000U

typedef enum {
    OUTPUT_ACTION = 0,
    OUTPUT_VOLUME_DOWN,
    OUTPUT_VOLUME_UP,
    OUTPUT_RESET,
    OUTPUT_LOADER_ENABLE,
    OUTPUT_COUNT,
} output_id_t;

typedef struct {
    uint pin;
    absolute_time_t release_at;
    bool active;
} output_state_t;

typedef enum {
    RESET_IDLE = 0,
    RESET_WAITING_TO_ASSERT,
    RESET_ASSERTED,
} reset_phase_t;

typedef enum {
    LOADER_IDLE = 0,
    LOADER_WAITING_TO_ASSERT_RESET,
    LOADER_RESET_ASSERTED,
    LOADER_WAITING_TO_RELEASE_ACTION,
} loader_phase_t;

static output_state_t outputs[OUTPUT_COUNT] = {
    [OUTPUT_ACTION] = {.pin = ACTION_PIN},
    [OUTPUT_VOLUME_DOWN] = {.pin = VOLUME_DOWN_PIN},
    [OUTPUT_VOLUME_UP] = {.pin = VOLUME_UP_PIN},
    [OUTPUT_RESET] = {.pin = RESET_PIN},
    [OUTPUT_LOADER_ENABLE] = {.pin = LOADER_ENABLE_PIN},
};
static reset_phase_t reset_phase;
static loader_phase_t loader_phase;
static absolute_time_t reset_transition_at;
static absolute_time_t loader_transition_at;
static absolute_time_t action_lockout_until;
static uint32_t reset_duration_ms;
static char session_nonce[33];
static absolute_time_t session_expires_at;
static uint32_t last_sequence;
static bool have_sequence;
static char last_response[RESPONSE_CAPACITY];

static void set_output(output_id_t output, bool active)
{
    gpio_put(outputs[output].pin, active);
    outputs[output].active = active;
    if (!active) {
        outputs[output].release_at = nil_time;
    }
}

static void release_all(void)
{
    for (size_t index = 0; index < OUTPUT_COUNT; ++index) {
        set_output((output_id_t)index, false);
    }
    reset_phase = RESET_IDLE;
    loader_phase = LOADER_IDLE;
}

static void activate_for(output_id_t output, uint32_t duration_ms)
{
    set_output(output, true);
    outputs[output].release_at = make_timeout_time_ms(duration_ms);
}

static bool valid_nonce(const char *nonce)
{
    if (strlen(nonce) != 32U) {
        return false;
    }
    for (size_t index = 0; index < 32U; ++index) {
        if (!isxdigit((unsigned char)nonce[index]) ||
            isupper((unsigned char)nonce[index])) {
            return false;
        }
    }
    return true;
}

static bool parse_u32(const char *text, uint32_t *value)
{
    char *end = NULL;
    const unsigned long parsed = strtoul(text, &end, 10);
    if (text[0] == '\0' || end == NULL || *end != '\0' ||
        parsed > UINT32_MAX) {
        return false;
    }
    *value = (uint32_t)parsed;
    return true;
}

static size_t tokenize(char *line, char *tokens[TOKEN_CAPACITY])
{
    size_t count = 0;
    char *save = NULL;
    char *token = strtok_r(line, " ", &save);
    while (token != NULL && count < TOKEN_CAPACITY) {
        tokens[count++] = token;
        token = strtok_r(NULL, " ", &save);
    }
    return token == NULL ? count : TOKEN_CAPACITY + 1U;
}

static void respond(const char *format, ...)
{
    va_list arguments;
    va_start(arguments, format);
    vsnprintf(last_response, sizeof(last_response), format, arguments);
    va_end(arguments);
    puts(last_response);
    fflush(stdout);
}

static bool session_matches(const char *nonce)
{
    return session_nonce[0] != '\0' && strcmp(session_nonce, nonce) == 0;
}

static bool begin_sequence(
    const char *sequence_text,
    const char *nonce,
    uint32_t *sequence
)
{
    if (!session_matches(nonce)) {
        respond("ERR SESSION");
        return false;
    }
    if (!parse_u32(sequence_text, sequence)) {
        respond("ERR SEQUENCE");
        return false;
    }
    if (have_sequence && *sequence == last_sequence) {
        puts(last_response);
        fflush(stdout);
        return false;
    }
    if (have_sequence && (int32_t)(*sequence - last_sequence) <= 0) {
        respond("ERR STALE_SEQUENCE");
        return false;
    }
    last_sequence = *sequence;
    have_sequence = true;
    session_expires_at = make_timeout_time_ms(SESSION_TIMEOUT_MS);
    return true;
}

static bool outputs_released(void)
{
    for (size_t index = 0; index < OUTPUT_COUNT; ++index) {
        if (outputs[index].active) {
            return false;
        }
    }
    return true;
}

static bool action_locked_out(void)
{
    return !is_nil_time(action_lockout_until) &&
           absolute_time_diff_us(get_absolute_time(), action_lockout_until) > 0;
}

static output_id_t parse_control(const char *control)
{
    if (strcmp(control, "ACTION") == 0) {
        return OUTPUT_ACTION;
    }
    if (strcmp(control, "VOLUME_DOWN") == 0) {
        return OUTPUT_VOLUME_DOWN;
    }
    if (strcmp(control, "VOLUME_UP") == 0) {
        return OUTPUT_VOLUME_UP;
    }
    return OUTPUT_COUNT;
}

static void handle_hello(char *tokens[], size_t count)
{
    uint32_t version = 0;
    if (count != 3U || !parse_u32(tokens[1], &version) ||
        version != FIXTURE_PROTOCOL_VERSION || !valid_nonce(tokens[2])) {
        respond("ERR HELLO");
        return;
    }
    release_all();
    strcpy(session_nonce, tokens[2]);
    have_sequence = false;
    session_expires_at = make_timeout_time_ms(SESSION_TIMEOUT_MS);
    pico_unique_board_id_t board_id;
    pico_get_unique_board_id(&board_id);
    char board_id_text[2U * PICO_UNIQUE_BOARD_ID_SIZE_BYTES + 1U];
    for (size_t index = 0; index < PICO_UNIQUE_BOARD_ID_SIZE_BYTES; ++index) {
        snprintf(
            &board_id_text[index * 2U],
            3U,
            "%02x",
            board_id.id[index]
        );
    }
    respond(
        "OK HELLO %u %s %s %s",
        FIXTURE_PROTOCOL_VERSION,
        session_nonce,
        FIXTURE_FIRMWARE_VERSION,
        board_id_text
    );
}

static void handle_status(char *tokens[], size_t count)
{
    uint32_t sequence = 0;
    if (count != 3U ||
        !begin_sequence(tokens[1], tokens[2], &sequence)) {
        return;
    }
    unsigned int mask = 0;
    for (size_t index = 0; index < OUTPUT_COUNT; ++index) {
        if (outputs[index].active) {
            mask |= 1U << index;
        }
    }
    respond(
        "OK STATUS %" PRIu32 " %s mask=%u reset=%u loader=%u "
        "action_lockout=%u",
        sequence,
        session_nonce,
        mask,
        (unsigned int)reset_phase,
        (unsigned int)loader_phase,
        action_locked_out() ? 1U : 0U
    );
}

static void handle_press(char *tokens[], size_t count)
{
    uint32_t sequence = 0;
    uint32_t duration_ms = 0;
    if (count != 5U ||
        !begin_sequence(tokens[1], tokens[2], &sequence)) {
        return;
    }
    const output_id_t output = parse_control(tokens[3]);
    if (output == OUTPUT_COUNT || !parse_u32(tokens[4], &duration_ms) ||
        duration_ms < MIN_PRESS_MS || duration_ms > MAX_PRESS_MS) {
        respond("ERR PRESS");
        return;
    }
    if (reset_phase != RESET_IDLE || loader_phase != LOADER_IDLE ||
        !outputs_released() ||
        (output == OUTPUT_ACTION && action_locked_out())) {
        respond("ERR INTERLOCK");
        return;
    }
    activate_for(output, duration_ms);
    respond(
        "OK PRESS %" PRIu32 " %s %s %" PRIu32,
        sequence,
        session_nonce,
        tokens[3],
        duration_ms
    );
}

static void handle_chord(char *tokens[], size_t count)
{
    uint32_t sequence = 0;
    uint32_t duration_ms = 0;
    if (count != 5U ||
        !begin_sequence(tokens[1], tokens[2], &sequence) ||
        !parse_u32(tokens[4], &duration_ms) ||
        duration_ms < MIN_PRESS_MS || duration_ms > MAX_PRESS_MS ||
        reset_phase != RESET_IDLE || loader_phase != LOADER_IDLE ||
        !outputs_released()) {
        respond("ERR CHORD");
        return;
    }
    if (strcmp(tokens[3], "SHUTDOWN") == 0) {
        if (action_locked_out()) {
            respond("ERR INTERLOCK");
            return;
        }
        activate_for(OUTPUT_ACTION, duration_ms);
        activate_for(OUTPUT_VOLUME_DOWN, duration_ms);
    } else if (strcmp(tokens[3], "PROVISION_RECOVERY") == 0) {
        activate_for(OUTPUT_VOLUME_DOWN, duration_ms);
        activate_for(OUTPUT_VOLUME_UP, duration_ms);
    } else {
        respond("ERR CHORD");
        return;
    }
    respond(
        "OK CHORD %" PRIu32 " %s %s %" PRIu32,
        sequence,
        session_nonce,
        tokens[3],
        duration_ms
    );
}

static void handle_reset(char *tokens[], size_t count)
{
    uint32_t sequence = 0;
    uint32_t duration_ms = 0;
    if (count != 4U ||
        !begin_sequence(tokens[1], tokens[2], &sequence) ||
        !parse_u32(tokens[3], &duration_ms) ||
        duration_ms < MIN_RESET_MS || duration_ms > MAX_RESET_MS ||
        !outputs_released() || reset_phase != RESET_IDLE ||
        loader_phase != LOADER_IDLE) {
        respond("ERR RESET");
        return;
    }

    reset_duration_ms = duration_ms;
    reset_phase = RESET_WAITING_TO_ASSERT;
    reset_transition_at = make_timeout_time_ms(RESET_SETTLE_MS);
    action_lockout_until = make_timeout_time_ms(
        RESET_SETTLE_MS + duration_ms + ACTION_POST_RESET_LOCKOUT_MS
    );
    respond(
        "OK RESET %" PRIu32 " %s %" PRIu32 " state=scheduled",
        sequence,
        session_nonce,
        duration_ms
    );
}

static void handle_enter_loader(char *tokens[], size_t count)
{
    uint32_t sequence = 0;
    if (count != 3U ||
        !begin_sequence(tokens[1], tokens[2], &sequence) ||
        !outputs_released() || reset_phase != RESET_IDLE ||
        loader_phase != LOADER_IDLE || action_locked_out()) {
        respond("ERR ENTER_LOADER");
        return;
    }
    activate_for(
        OUTPUT_LOADER_ENABLE,
        LOADER_MAX_ACTIVE_MS
    );
    activate_for(
        OUTPUT_ACTION,
        LOADER_MAX_ACTIVE_MS
    );
    loader_phase = LOADER_WAITING_TO_ASSERT_RESET;
    loader_transition_at = make_timeout_time_ms(LOADER_ACTION_SETTLE_MS);
    action_lockout_until = make_timeout_time_ms(
        LOADER_ACTION_SETTLE_MS + LOADER_RESET_MS +
        LOADER_STRAP_HOLD_MS + ACTION_POST_RESET_LOCKOUT_MS
    );
    respond(
        "OK ENTER_LOADER %" PRIu32 " %s state=scheduled",
        sequence,
        session_nonce
    );
}

static void handle_release_all(char *tokens[], size_t count)
{
    uint32_t sequence = 0;
    if (count != 3U ||
        !begin_sequence(tokens[1], tokens[2], &sequence)) {
        return;
    }
    release_all();
    action_lockout_until = make_timeout_time_ms(
        ACTION_POST_RESET_LOCKOUT_MS
    );
    respond(
        "OK RELEASE_ALL %" PRIu32 " %s",
        sequence,
        session_nonce
    );
}

static void process_line(char *line)
{
    char *tokens[TOKEN_CAPACITY] = {0};
    const size_t count = tokenize(line, tokens);
    if (count == 0U || count > TOKEN_CAPACITY) {
        respond("ERR FRAME");
        return;
    }
    if (strcmp(tokens[0], "HELLO") == 0) {
        handle_hello(tokens, count);
    } else if (strcmp(tokens[0], "STATUS") == 0) {
        handle_status(tokens, count);
    } else if (strcmp(tokens[0], "PRESS") == 0) {
        handle_press(tokens, count);
    } else if (strcmp(tokens[0], "CHORD") == 0) {
        handle_chord(tokens, count);
    } else if (strcmp(tokens[0], "RESET") == 0) {
        handle_reset(tokens, count);
    } else if (strcmp(tokens[0], "ENTER_LOADER") == 0) {
        handle_enter_loader(tokens, count);
    } else if (strcmp(tokens[0], "RELEASE_ALL") == 0) {
        handle_release_all(tokens, count);
    } else {
        respond("ERR OPCODE");
    }
}

static void update_outputs(void)
{
    const absolute_time_t now = get_absolute_time();
    if (session_nonce[0] != '\0' &&
        absolute_time_diff_us(now, session_expires_at) <= 0) {
        release_all();
        action_lockout_until = make_timeout_time_ms(
            ACTION_POST_RESET_LOCKOUT_MS
        );
        session_nonce[0] = '\0';
        have_sequence = false;
    }
    for (size_t index = 0; index < OUTPUT_COUNT; ++index) {
        if (outputs[index].active &&
            absolute_time_diff_us(now, outputs[index].release_at) <= 0) {
            set_output((output_id_t)index, false);
        }
    }
    if (loader_phase != LOADER_IDLE &&
        (!outputs[OUTPUT_ACTION].active ||
         !outputs[OUTPUT_LOADER_ENABLE].active)) {
        set_output(OUTPUT_RESET, false);
        set_output(OUTPUT_ACTION, false);
        set_output(OUTPUT_LOADER_ENABLE, false);
        loader_phase = LOADER_IDLE;
        action_lockout_until = make_timeout_time_ms(
            ACTION_POST_RESET_LOCKOUT_MS
        );
    }
    if (reset_phase == RESET_WAITING_TO_ASSERT &&
        absolute_time_diff_us(now, reset_transition_at) <= 0) {
        activate_for(OUTPUT_RESET, reset_duration_ms);
        reset_phase = RESET_ASSERTED;
        reset_transition_at = outputs[OUTPUT_RESET].release_at;
    } else if (reset_phase == RESET_ASSERTED &&
               !outputs[OUTPUT_RESET].active) {
        reset_phase = RESET_IDLE;
    }
    if (loader_phase == LOADER_WAITING_TO_ASSERT_RESET &&
        absolute_time_diff_us(now, loader_transition_at) <= 0) {
        activate_for(OUTPUT_RESET, LOADER_RESET_MS);
        loader_phase = LOADER_RESET_ASSERTED;
        loader_transition_at = outputs[OUTPUT_RESET].release_at;
    } else if (loader_phase == LOADER_RESET_ASSERTED &&
               !outputs[OUTPUT_RESET].active) {
        loader_phase = LOADER_WAITING_TO_RELEASE_ACTION;
        loader_transition_at = make_timeout_time_ms(
            LOADER_STRAP_HOLD_MS
        );
    } else if (loader_phase == LOADER_WAITING_TO_RELEASE_ACTION &&
               absolute_time_diff_us(now, loader_transition_at) <= 0) {
        set_output(OUTPUT_ACTION, false);
        set_output(OUTPUT_LOADER_ENABLE, false);
        loader_phase = LOADER_IDLE;
        action_lockout_until = make_timeout_time_ms(
            ACTION_POST_RESET_LOCKOUT_MS
        );
    }
}

int main(void)
{
    for (size_t index = 0; index < OUTPUT_COUNT; ++index) {
        gpio_init(outputs[index].pin);
        gpio_put(outputs[index].pin, false);
        gpio_set_dir(outputs[index].pin, GPIO_OUT);
    }
    release_all();
    stdio_init_all();
    watchdog_enable(1000U, true);

    char line[LINE_CAPACITY];
    size_t length = 0;
    bool discarding_oversize = false;
    for (;;) {
        watchdog_update();
        update_outputs();
        const int character = getchar_timeout_us(1000U);
        if (character == PICO_ERROR_TIMEOUT) {
            continue;
        }
        if (character == '\r') {
            continue;
        }
        if (character == '\n') {
            if (discarding_oversize) {
                respond("ERR TOO_LONG");
            } else {
                line[length] = '\0';
                process_line(line);
            }
            length = 0;
            discarding_oversize = false;
            continue;
        }
        if (discarding_oversize) {
            continue;
        }
        if (length + 1U >= sizeof(line)) {
            length = 0;
            discarding_oversize = true;
            continue;
        }
        line[length++] = (char)character;
    }
}
