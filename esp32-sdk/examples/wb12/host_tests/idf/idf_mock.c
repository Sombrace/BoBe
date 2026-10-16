#include "idf_mock.h"

#include "driver/i2s_std.h"
#include "esp_afe_sr_models.h"
#include "esp_err.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "esp_timer.h"
#include "freertos/event_groups.h"
#include "freertos/queue.h"
#include "freertos/semphr.h"
#include "freertos/task.h"
#include "model_path.h"
#include "nvs.h"

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define MOCK_TASKS 16U
#define MOCK_SEMAPHORES 16U
#define MOCK_EVENT_GROUPS 8U
#define MOCK_QUEUES 8U
#define MOCK_ALLOCATIONS 32U
#define MOCK_I2S_CHANNELS 8U
#define MOCK_NVS_ENTRIES 64U
#define MOCK_NVS_NAMESPACES 8U
#define MOCK_NVS_HANDLES 8U
#define MOCK_NVS_OPERATIONS 128U
#define MOCK_NAME_BYTES 16U
#define MOCK_STRING_BYTES 512U

struct idf_mock_task {
    bool alive;
    bool with_caps;
    unsigned int notifications;
    char name[MOCK_NAME_BYTES];
};

struct idf_mock_semaphore {
    bool alive;
    bool held;
};

struct idf_mock_event_group {
    bool alive;
    EventBits_t bits;
};

struct idf_mock_queue {
    bool alive;
    UBaseType_t length;
    UBaseType_t item_size;
    UBaseType_t count;
    UBaseType_t head;
    uint8_t *storage;
};

typedef enum {
    I2S_STATE_REGISTERED = 0,
    I2S_STATE_READY,
    I2S_STATE_RUNNING,
} mock_i2s_state_t;

struct i2s_channel_obj_t {
    bool alive;
    bool transmit;
    mock_i2s_state_t state;
};

struct esp_afe_sr_data_t {
    bool alive;
};

typedef enum {
    NVS_VALUE_U8,
    NVS_VALUE_U64,
    NVS_VALUE_STRING,
} mock_nvs_type_t;

typedef struct {
    bool used;
    char namespace_name[MOCK_NAME_BYTES];
    char key[MOCK_NAME_BYTES];
    mock_nvs_type_t type;
    uint64_t number;
    char string[MOCK_STRING_BYTES];
} mock_nvs_entry_t;

typedef struct {
    bool open;
    bool writable;
    char namespace_name[MOCK_NAME_BYTES];
} mock_nvs_handle_t;

static struct idf_mock_task tasks[MOCK_TASKS];
static unsigned int task_creations;
static unsigned int failing_task_creation;
static struct idf_mock_semaphore semaphores[MOCK_SEMAPHORES];
static struct idf_mock_event_group event_groups[MOCK_EVENT_GROUPS];
static struct idf_mock_queue queues[MOCK_QUEUES];
static void *allocations[MOCK_ALLOCATIONS];
static int64_t now_us;
static TickType_t now_ticks;

static struct i2s_channel_obj_t i2s_channels[MOCK_I2S_CHANNELS];
static unsigned int i2s_new_channel_calls;
static unsigned int failing_i2s_new_channel;
static unsigned int i2s_init_calls;
static unsigned int failing_i2s_init;
static unsigned int i2s_rejected_enables;

static srmodel_list_t speech_models;
static bool speech_models_loaded;
static bool wake_model_provided;
static bool afe_creation_fails;
static struct esp_afe_sr_data_t afe_instance;
static char wake_model_name[] = "wn9_bobe";

static mock_nvs_entry_t nvs_entries[MOCK_NVS_ENTRIES];
static char nvs_namespaces[MOCK_NVS_NAMESPACES][MOCK_NAME_BYTES];
static mock_nvs_handle_t nvs_handles[MOCK_NVS_HANDLES];
static size_t nvs_operations;
static size_t failing_nvs_operation;
static char nvs_operation_keys[MOCK_NVS_OPERATIONS][MOCK_NAME_BYTES];

void idf_mock_violation(const char *message)
{
    fprintf(stderr, "IDF mock violation: %s\n", message);
    exit(1);
}

static void copy_name(char *destination, const char *source)
{
    if (source == NULL) {
        destination[0] = '\0';
        return;
    }
    if (strlen(source) >= MOCK_NAME_BYTES) {
        idf_mock_violation("name exceeds the mock limit");
    }
    strcpy(destination, source);
}

void idf_mock_reset(void)
{
    for (size_t index = 0; index < MOCK_ALLOCATIONS; ++index) {
        free(allocations[index]);
    }
    memset(tasks, 0, sizeof(tasks));
    memset(semaphores, 0, sizeof(semaphores));
    memset(event_groups, 0, sizeof(event_groups));
    memset(queues, 0, sizeof(queues));
    memset(allocations, 0, sizeof(allocations));
    memset(i2s_channels, 0, sizeof(i2s_channels));
    task_creations = 0;
    failing_task_creation = 0;
    i2s_new_channel_calls = 0;
    failing_i2s_new_channel = 0;
    i2s_init_calls = 0;
    failing_i2s_init = 0;
    i2s_rejected_enables = 0;
    speech_models_loaded = false;
    wake_model_provided = false;
    afe_creation_fails = false;
    afe_instance.alive = false;
}

void idf_mock_log(const char *tag, const char *format, ...)
{
    char buffer[256];
    va_list arguments;
    va_start(arguments, format);
    (void)vsnprintf(buffer, sizeof(buffer), format, arguments);
    va_end(arguments);
    (void)tag;
}

const char *esp_err_to_name(esp_err_t code)
{
    switch (code) {
    case ESP_OK:
        return "ESP_OK";
    case ESP_FAIL:
        return "ESP_FAIL";
    case ESP_ERR_NO_MEM:
        return "ESP_ERR_NO_MEM";
    case ESP_ERR_INVALID_ARG:
        return "ESP_ERR_INVALID_ARG";
    case ESP_ERR_INVALID_STATE:
        return "ESP_ERR_INVALID_STATE";
    case ESP_ERR_TIMEOUT:
        return "ESP_ERR_TIMEOUT";
    case ESP_ERR_NVS_NOT_FOUND:
        return "ESP_ERR_NVS_NOT_FOUND";
    default:
        return "ESP_ERR_UNKNOWN";
    }
}

int64_t esp_timer_get_time(void)
{
    now_us += 1000;
    return now_us;
}

void *heap_caps_malloc(size_t size, uint32_t caps)
{
    (void)caps;
    for (size_t index = 0; index < MOCK_ALLOCATIONS; ++index) {
        if (allocations[index] == NULL) {
            allocations[index] = calloc(1, size);
            return allocations[index];
        }
    }
    idf_mock_violation("allocation table is full");
    return NULL;
}

void heap_caps_free(void *pointer)
{
    if (pointer == NULL) {
        return;
    }
    for (size_t index = 0; index < MOCK_ALLOCATIONS; ++index) {
        if (allocations[index] == pointer) {
            free(pointer);
            allocations[index] = NULL;
            return;
        }
    }
    idf_mock_violation("freeing memory the heap did not allocate");
}

size_t idf_mock_live_allocations(void)
{
    size_t count = 0;
    for (size_t index = 0; index < MOCK_ALLOCATIONS; ++index) {
        count += allocations[index] != NULL ? 1U : 0U;
    }
    return count;
}

static BaseType_t create_task(
    const char *name,
    TaskHandle_t *created_task,
    bool with_caps
)
{
    ++task_creations;
    if (task_creations == failing_task_creation) {
        return pdFAIL;
    }
    for (size_t index = 0; index < MOCK_TASKS; ++index) {
        if (!tasks[index].alive) {
            memset(&tasks[index], 0, sizeof(tasks[index]));
            tasks[index].alive = true;
            tasks[index].with_caps = with_caps;
            copy_name(tasks[index].name, name);
            if (created_task != NULL) {
                *created_task = &tasks[index];
            }
            return pdPASS;
        }
    }
    idf_mock_violation("task table is full");
    return pdFAIL;
}

BaseType_t xTaskCreatePinnedToCore(
    TaskFunction_t function,
    const char *name,
    uint32_t stack_depth,
    void *parameters,
    UBaseType_t priority,
    TaskHandle_t *created_task,
    BaseType_t core_id
)
{
    (void)function;
    (void)stack_depth;
    (void)parameters;
    (void)priority;
    (void)core_id;
    return create_task(name, created_task, false);
}

BaseType_t xTaskCreatePinnedToCoreWithCaps(
    TaskFunction_t function,
    const char *name,
    uint32_t stack_depth,
    void *parameters,
    UBaseType_t priority,
    TaskHandle_t *created_task,
    BaseType_t core_id,
    UBaseType_t memory_caps
)
{
    (void)function;
    (void)stack_depth;
    (void)parameters;
    (void)priority;
    (void)core_id;
    (void)memory_caps;
    return create_task(name, created_task, true);
}

static void delete_task(TaskHandle_t task, bool with_caps)
{
    if (task == NULL || !task->alive) {
        idf_mock_violation("deleting a task that is not alive");
    }
    if (task->with_caps != with_caps) {
        idf_mock_violation("task deleted with the wrong allocator");
    }
    task->alive = false;
}

void vTaskDelete(TaskHandle_t task)
{
    delete_task(task, false);
}

void vTaskDeleteWithCaps(TaskHandle_t task)
{
    delete_task(task, true);
}

void idf_mock_fail_task_creation(unsigned int call)
{
    failing_task_creation = call;
}

size_t idf_mock_live_tasks(void)
{
    size_t count = 0;
    for (size_t index = 0; index < MOCK_TASKS; ++index) {
        count += tasks[index].alive ? 1U : 0U;
    }
    return count;
}

bool idf_mock_task_alive(const char *name)
{
    for (size_t index = 0; index < MOCK_TASKS; ++index) {
        if (tasks[index].alive && strcmp(tasks[index].name, name) == 0) {
            return true;
        }
    }
    return false;
}

uint32_t ulTaskNotifyTake(BaseType_t clear_count_on_exit, TickType_t ticks_to_wait)
{
    (void)clear_count_on_exit;
    (void)ticks_to_wait;
    return 0;
}

BaseType_t xTaskNotifyGive(TaskHandle_t task)
{
    if (task == NULL || !task->alive) {
        idf_mock_violation("notifying a task that is not alive");
    }
    ++task->notifications;
    return pdPASS;
}

void vTaskNotifyGiveFromISR(TaskHandle_t task, BaseType_t *higher_priority_task_woken)
{
    (void)xTaskNotifyGive(task);
    if (higher_priority_task_woken != NULL) {
        *higher_priority_task_woken = pdFALSE;
    }
}

void vTaskDelay(TickType_t ticks)
{
    now_ticks += ticks;
    now_us += (int64_t)ticks * 1000;
}

TickType_t xTaskGetTickCount(void)
{
    return now_ticks;
}

SemaphoreHandle_t xSemaphoreCreateMutex(void)
{
    for (size_t index = 0; index < MOCK_SEMAPHORES; ++index) {
        if (!semaphores[index].alive) {
            semaphores[index] = (struct idf_mock_semaphore){.alive = true};
            return &semaphores[index];
        }
    }
    idf_mock_violation("semaphore table is full");
    return NULL;
}

BaseType_t xSemaphoreTake(SemaphoreHandle_t semaphore, TickType_t ticks_to_wait)
{
    if (semaphore == NULL || !semaphore->alive) {
        idf_mock_violation("taking a semaphore that is not alive");
    }
    if (semaphore->held) {
        if (ticks_to_wait == portMAX_DELAY) {
            idf_mock_violation("waiting forever on a held semaphore");
        }
        return pdFALSE;
    }
    semaphore->held = true;
    return pdTRUE;
}

BaseType_t xSemaphoreGive(SemaphoreHandle_t semaphore)
{
    if (semaphore == NULL || !semaphore->alive || !semaphore->held) {
        idf_mock_violation("giving a semaphore that is not held");
    }
    semaphore->held = false;
    return pdTRUE;
}

void vSemaphoreDelete(SemaphoreHandle_t semaphore)
{
    if (semaphore == NULL || !semaphore->alive) {
        idf_mock_violation("deleting a semaphore that is not alive");
    }
    semaphore->alive = false;
}

size_t idf_mock_live_semaphores(void)
{
    size_t count = 0;
    for (size_t index = 0; index < MOCK_SEMAPHORES; ++index) {
        count += semaphores[index].alive ? 1U : 0U;
    }
    return count;
}

EventGroupHandle_t xEventGroupCreate(void)
{
    for (size_t index = 0; index < MOCK_EVENT_GROUPS; ++index) {
        if (!event_groups[index].alive) {
            event_groups[index] = (struct idf_mock_event_group){.alive = true};
            return &event_groups[index];
        }
    }
    idf_mock_violation("event group table is full");
    return NULL;
}

static struct idf_mock_event_group *live_group(EventGroupHandle_t group)
{
    if (group == NULL || !group->alive) {
        idf_mock_violation("using an event group that is not alive");
    }
    return group;
}

EventBits_t xEventGroupSetBits(EventGroupHandle_t group, EventBits_t bits)
{
    live_group(group)->bits |= bits;
    return group->bits;
}

EventBits_t xEventGroupClearBits(EventGroupHandle_t group, EventBits_t bits)
{
    const EventBits_t previous = live_group(group)->bits;
    group->bits &= ~bits;
    return previous;
}

EventBits_t xEventGroupGetBits(EventGroupHandle_t group)
{
    return live_group(group)->bits;
}

EventBits_t xEventGroupWaitBits(
    EventGroupHandle_t group,
    EventBits_t bits,
    BaseType_t clear_on_exit,
    BaseType_t wait_for_all,
    TickType_t ticks_to_wait
)
{
    (void)ticks_to_wait;
    const EventBits_t current = live_group(group)->bits;
    const bool satisfied = wait_for_all ? (current & bits) == bits
                                        : (current & bits) != 0;
    if (satisfied && clear_on_exit) {
        group->bits &= ~bits;
    }
    return current;
}

void vEventGroupDelete(EventGroupHandle_t group)
{
    live_group(group)->alive = false;
}

size_t idf_mock_live_event_groups(void)
{
    size_t count = 0;
    for (size_t index = 0; index < MOCK_EVENT_GROUPS; ++index) {
        count += event_groups[index].alive ? 1U : 0U;
    }
    return count;
}

QueueHandle_t xQueueCreateStatic(
    UBaseType_t length,
    UBaseType_t item_size,
    uint8_t *storage,
    StaticQueue_t *queue_buffer
)
{
    if (storage == NULL || queue_buffer == NULL || length == 0) {
        idf_mock_violation("static queue needs storage");
    }
    for (size_t index = 0; index < MOCK_QUEUES; ++index) {
        if (!queues[index].alive) {
            queues[index] = (struct idf_mock_queue){
                .alive = true,
                .length = length,
                .item_size = item_size,
                .storage = storage,
            };
            return &queues[index];
        }
    }
    idf_mock_violation("queue table is full");
    return NULL;
}

static struct idf_mock_queue *live_queue(QueueHandle_t queue)
{
    if (queue == NULL || !queue->alive) {
        idf_mock_violation("using a queue that is not alive");
    }
    return queue;
}

BaseType_t xQueueSend(QueueHandle_t queue, const void *item, TickType_t ticks_to_wait)
{
    (void)ticks_to_wait;
    struct idf_mock_queue *const target = live_queue(queue);
    if (target->count == target->length) {
        return pdFALSE;
    }
    const UBaseType_t slot = (target->head + target->count) % target->length;
    memcpy(target->storage + (size_t)slot * target->item_size, item, target->item_size);
    ++target->count;
    return pdTRUE;
}

BaseType_t xQueueReceive(QueueHandle_t queue, void *item, TickType_t ticks_to_wait)
{
    (void)ticks_to_wait;
    struct idf_mock_queue *const source = live_queue(queue);
    if (source->count == 0) {
        return pdFALSE;
    }
    memcpy(item, source->storage + (size_t)source->head * source->item_size, source->item_size);
    source->head = (source->head + 1U) % source->length;
    --source->count;
    return pdTRUE;
}

BaseType_t xQueueReset(QueueHandle_t queue)
{
    struct idf_mock_queue *const target = live_queue(queue);
    target->count = 0;
    target->head = 0;
    return pdPASS;
}

UBaseType_t uxQueueMessagesWaiting(QueueHandle_t queue)
{
    return live_queue(queue)->count;
}

void vQueueDelete(QueueHandle_t queue)
{
    live_queue(queue)->alive = false;
}

size_t idf_mock_live_queues(void)
{
    size_t count = 0;
    for (size_t index = 0; index < MOCK_QUEUES; ++index) {
        count += queues[index].alive ? 1U : 0U;
    }
    return count;
}

void idf_mock_fail_i2s_new_channel(unsigned int call)
{
    failing_i2s_new_channel = call;
}

void idf_mock_fail_i2s_init(unsigned int call)
{
    failing_i2s_init = call;
}

esp_err_t i2s_new_channel(
    const i2s_chan_config_t *chan_cfg,
    i2s_chan_handle_t *ret_tx_handle,
    i2s_chan_handle_t *ret_rx_handle
)
{
    if (chan_cfg == NULL || (ret_tx_handle == NULL) == (ret_rx_handle == NULL)) {
        idf_mock_violation("the mock creates exactly one I2S direction");
    }
    ++i2s_new_channel_calls;
    if (i2s_new_channel_calls == failing_i2s_new_channel) {
        return ESP_ERR_NO_MEM;
    }
    for (size_t index = 0; index < MOCK_I2S_CHANNELS; ++index) {
        if (!i2s_channels[index].alive) {
            i2s_channels[index] = (struct i2s_channel_obj_t){
                .alive = true,
                .transmit = ret_tx_handle != NULL,
                .state = I2S_STATE_REGISTERED,
            };
            if (ret_tx_handle != NULL) {
                *ret_tx_handle = &i2s_channels[index];
            } else {
                *ret_rx_handle = &i2s_channels[index];
            }
            return ESP_OK;
        }
    }
    idf_mock_violation("I2S channel table is full");
    return ESP_FAIL;
}

static struct i2s_channel_obj_t *live_channel(i2s_chan_handle_t handle)
{
    if (handle == NULL || !handle->alive) {
        idf_mock_violation("using an I2S channel that is not alive");
    }
    return handle;
}

esp_err_t i2s_del_channel(i2s_chan_handle_t handle)
{
    if (live_channel(handle)->state == I2S_STATE_RUNNING) {
        return ESP_ERR_INVALID_STATE;
    }
    handle->alive = false;
    return ESP_OK;
}

esp_err_t i2s_channel_init_std_mode(
    i2s_chan_handle_t handle,
    const i2s_std_config_t *std_cfg
)
{
    if (std_cfg == NULL || live_channel(handle)->state != I2S_STATE_REGISTERED) {
        return ESP_ERR_INVALID_STATE;
    }
    ++i2s_init_calls;
    if (i2s_init_calls == failing_i2s_init) {
        return ESP_FAIL;
    }
    handle->state = I2S_STATE_READY;
    return ESP_OK;
}

esp_err_t i2s_channel_register_event_callback(
    i2s_chan_handle_t handle,
    const i2s_event_callbacks_t *callbacks,
    void *user_data
)
{
    (void)user_data;
    if (callbacks == NULL || live_channel(handle)->state == I2S_STATE_RUNNING) {
        return ESP_ERR_INVALID_STATE;
    }
    return ESP_OK;
}

esp_err_t i2s_channel_enable(i2s_chan_handle_t handle)
{
    if (live_channel(handle)->state != I2S_STATE_READY) {
        ++i2s_rejected_enables;
        return ESP_ERR_INVALID_STATE;
    }
    handle->state = I2S_STATE_RUNNING;
    return ESP_OK;
}

esp_err_t i2s_channel_disable(i2s_chan_handle_t handle)
{
    if (live_channel(handle)->state != I2S_STATE_RUNNING) {
        return ESP_ERR_INVALID_STATE;
    }
    handle->state = I2S_STATE_READY;
    return ESP_OK;
}

esp_err_t i2s_channel_read(
    i2s_chan_handle_t handle,
    void *dest,
    size_t size,
    size_t *bytes_read,
    uint32_t timeout_ms
)
{
    (void)timeout_ms;
    if (live_channel(handle)->state != I2S_STATE_RUNNING || handle->transmit) {
        return ESP_ERR_INVALID_STATE;
    }
    memset(dest, 0, size);
    *bytes_read = size;
    return ESP_OK;
}

esp_err_t i2s_channel_preload_data(
    i2s_chan_handle_t tx_handle,
    const void *src,
    size_t size,
    size_t *bytes_loaded
)
{
    (void)src;
    if (live_channel(tx_handle)->state != I2S_STATE_READY || !tx_handle->transmit) {
        return ESP_ERR_INVALID_STATE;
    }
    *bytes_loaded = size;
    return ESP_OK;
}

size_t idf_mock_live_i2s_channels(void)
{
    size_t count = 0;
    for (size_t index = 0; index < MOCK_I2S_CHANNELS; ++index) {
        count += i2s_channels[index].alive ? 1U : 0U;
    }
    return count;
}

size_t idf_mock_running_i2s_rx_channels(void)
{
    size_t count = 0;
    for (size_t index = 0; index < MOCK_I2S_CHANNELS; ++index) {
        count += i2s_channels[index].alive && !i2s_channels[index].transmit &&
                         i2s_channels[index].state == I2S_STATE_RUNNING
                     ? 1U
                     : 0U;
    }
    return count;
}

unsigned int idf_mock_i2s_rejected_enables(void)
{
    return i2s_rejected_enables;
}

void idf_mock_fail_afe_creation(bool fail)
{
    afe_creation_fails = fail;
}

void idf_mock_provide_wake_model(bool provide)
{
    wake_model_provided = provide;
}

bool idf_mock_speech_models_loaded(void)
{
    return speech_models_loaded;
}

bool idf_mock_afe_created(void)
{
    return afe_instance.alive;
}

srmodel_list_t *esp_srmodel_init(const char *partition_label)
{
    if (partition_label == NULL || speech_models_loaded) {
        idf_mock_violation("speech models loaded twice");
    }
    speech_models_loaded = true;
    return &speech_models;
}

void esp_srmodel_deinit(srmodel_list_t *models)
{
    if (models != &speech_models || !speech_models_loaded) {
        idf_mock_violation("releasing speech models that are not loaded");
    }
    speech_models_loaded = false;
}

char *esp_srmodel_filter(
    srmodel_list_t *models,
    const char *keyword1,
    const char *keyword2
)
{
    (void)keyword1;
    (void)keyword2;
    return models == &speech_models && wake_model_provided ? wake_model_name : NULL;
}

afe_config_t *afe_config_init(
    const char *input_format,
    srmodel_list_t *models,
    afe_type_t type,
    afe_mode_t mode
)
{
    (void)input_format;
    (void)models;
    (void)type;
    (void)mode;
    return calloc(1, sizeof(afe_config_t));
}

void afe_config_free(afe_config_t *afe_config)
{
    free(afe_config);
}

static esp_afe_sr_data_t *afe_create(afe_config_t *afe_config)
{
    if (afe_config == NULL || afe_instance.alive) {
        idf_mock_violation("AFE created twice");
    }
    if (afe_creation_fails) {
        return NULL;
    }
    afe_instance.alive = true;
    return &afe_instance;
}

static esp_afe_sr_data_t *live_afe(esp_afe_sr_data_t *afe)
{
    if (afe != &afe_instance || !afe_instance.alive) {
        idf_mock_violation("using an AFE that is not alive");
    }
    return afe;
}

static int afe_feed(esp_afe_sr_data_t *afe, const int16_t *in)
{
    (void)live_afe(afe);
    (void)in;
    return 512;
}

static afe_fetch_result_t *afe_fetch(esp_afe_sr_data_t *afe, TickType_t ticks_to_wait)
{
    (void)live_afe(afe);
    (void)ticks_to_wait;
    return NULL;
}

static int afe_reset_buffer(esp_afe_sr_data_t *afe)
{
    (void)live_afe(afe);
    return 1;
}

static int afe_feed_chunksize(esp_afe_sr_data_t *afe)
{
    (void)live_afe(afe);
    return 512;
}

static int afe_channels(esp_afe_sr_data_t *afe)
{
    (void)live_afe(afe);
    return 1;
}

static int afe_sample_rate(esp_afe_sr_data_t *afe)
{
    (void)live_afe(afe);
    return 16000;
}

static void afe_print_pipeline(esp_afe_sr_data_t *afe)
{
    (void)live_afe(afe);
}

static void afe_destroy(esp_afe_sr_data_t *afe)
{
    live_afe(afe)->alive = false;
}

static const esp_afe_sr_iface_t afe_interface = {
    .create_from_config = afe_create,
    .feed = afe_feed,
    .fetch_with_delay = afe_fetch,
    .reset_buffer = afe_reset_buffer,
    .get_feed_chunksize = afe_feed_chunksize,
    .get_feed_channel_num = afe_channels,
    .get_samp_rate = afe_sample_rate,
    .print_pipeline = afe_print_pipeline,
    .destroy = afe_destroy,
};

const esp_afe_sr_iface_t *esp_afe_handle_from_config(const afe_config_t *config)
{
    return config != NULL ? &afe_interface : NULL;
}

void idf_mock_nvs_reset(void)
{
    memset(nvs_entries, 0, sizeof(nvs_entries));
    memset(nvs_namespaces, 0, sizeof(nvs_namespaces));
    memset(nvs_handles, 0, sizeof(nvs_handles));
    idf_mock_nvs_start_counting();
}

void idf_mock_nvs_start_counting(void)
{
    nvs_operations = 0;
    failing_nvs_operation = 0;
    memset(nvs_operation_keys, 0, sizeof(nvs_operation_keys));
}

void idf_mock_nvs_fail_operation(size_t operation)
{
    failing_nvs_operation = operation;
}

size_t idf_mock_nvs_operation_count(void)
{
    return nvs_operations;
}

const char *idf_mock_nvs_operation_key(size_t operation)
{
    if (operation == 0 || operation > nvs_operations ||
        operation > MOCK_NVS_OPERATIONS) {
        idf_mock_violation("unknown NVS operation");
    }
    return nvs_operation_keys[operation - 1U];
}

static bool mutation_allowed(const char *key)
{
    ++nvs_operations;
    if (nvs_operations <= MOCK_NVS_OPERATIONS) {
        copy_name(nvs_operation_keys[nvs_operations - 1U], key);
    }
    return nvs_operations != failing_nvs_operation;
}

static bool namespace_exists(const char *namespace_name)
{
    for (size_t index = 0; index < MOCK_NVS_NAMESPACES; ++index) {
        if (strcmp(nvs_namespaces[index], namespace_name) == 0) {
            return true;
        }
    }
    return false;
}

static void create_namespace(const char *namespace_name)
{
    if (namespace_exists(namespace_name)) {
        return;
    }
    for (size_t index = 0; index < MOCK_NVS_NAMESPACES; ++index) {
        if (nvs_namespaces[index][0] == '\0') {
            copy_name(nvs_namespaces[index], namespace_name);
            return;
        }
    }
    idf_mock_violation("NVS namespace table is full");
}

static mock_nvs_handle_t *open_handle(nvs_handle_t handle)
{
    if (handle == 0 || handle > MOCK_NVS_HANDLES || !nvs_handles[handle - 1U].open) {
        idf_mock_violation("using an NVS handle that is not open");
    }
    return &nvs_handles[handle - 1U];
}

static mock_nvs_entry_t *find_entry(const char *namespace_name, const char *key)
{
    for (size_t index = 0; index < MOCK_NVS_ENTRIES; ++index) {
        if (nvs_entries[index].used &&
            strcmp(nvs_entries[index].namespace_name, namespace_name) == 0 &&
            strcmp(nvs_entries[index].key, key) == 0) {
            return &nvs_entries[index];
        }
    }
    return NULL;
}

esp_err_t nvs_open(
    const char *namespace_name,
    nvs_open_mode_t open_mode,
    nvs_handle_t *out_handle
)
{
    if (open_mode == NVS_READWRITE) {
        if (!mutation_allowed(namespace_name)) {
            return ESP_FAIL;
        }
        create_namespace(namespace_name);
    } else if (!namespace_exists(namespace_name)) {
        return ESP_ERR_NVS_NOT_FOUND;
    }
    for (size_t index = 0; index < MOCK_NVS_HANDLES; ++index) {
        if (!nvs_handles[index].open) {
            nvs_handles[index].open = true;
            nvs_handles[index].writable = open_mode == NVS_READWRITE;
            copy_name(nvs_handles[index].namespace_name, namespace_name);
            *out_handle = (nvs_handle_t)(index + 1U);
            return ESP_OK;
        }
    }
    idf_mock_violation("NVS handle table is full");
    return ESP_FAIL;
}

static esp_err_t set_value(
    nvs_handle_t handle,
    const char *key,
    mock_nvs_type_t type,
    uint64_t number,
    const char *string
)
{
    mock_nvs_handle_t *const target = open_handle(handle);
    if (!target->writable) {
        return ESP_ERR_NVS_READ_ONLY;
    }
    if (!mutation_allowed(key)) {
        return ESP_ERR_NVS_NOT_ENOUGH_SPACE;
    }
    mock_nvs_entry_t *entry = find_entry(target->namespace_name, key);
    for (size_t index = 0; entry == NULL && index < MOCK_NVS_ENTRIES; ++index) {
        if (!nvs_entries[index].used) {
            entry = &nvs_entries[index];
        }
    }
    if (entry == NULL) {
        idf_mock_violation("NVS entry table is full");
        return ESP_FAIL;
    }
    memset(entry, 0, sizeof(*entry));
    entry->used = true;
    copy_name(entry->namespace_name, target->namespace_name);
    copy_name(entry->key, key);
    entry->type = type;
    entry->number = number;
    if (string != NULL) {
        if (strlen(string) >= MOCK_STRING_BYTES) {
            idf_mock_violation("NVS string exceeds the mock limit");
        }
        strcpy(entry->string, string);
    }
    return ESP_OK;
}

esp_err_t nvs_set_u8(nvs_handle_t handle, const char *key, uint8_t value)
{
    return set_value(handle, key, NVS_VALUE_U8, value, NULL);
}

esp_err_t nvs_set_u64(nvs_handle_t handle, const char *key, uint64_t value)
{
    return set_value(handle, key, NVS_VALUE_U64, value, NULL);
}

esp_err_t nvs_set_str(nvs_handle_t handle, const char *key, const char *value)
{
    return set_value(handle, key, NVS_VALUE_STRING, 0, value);
}

static esp_err_t get_number(
    nvs_handle_t handle,
    const char *key,
    mock_nvs_type_t type,
    uint64_t *value
)
{
    const mock_nvs_entry_t *const entry =
        find_entry(open_handle(handle)->namespace_name, key);
    if (entry == NULL) {
        return ESP_ERR_NVS_NOT_FOUND;
    }
    if (entry->type != type) {
        return ESP_ERR_NVS_TYPE_MISMATCH;
    }
    *value = entry->number;
    return ESP_OK;
}

esp_err_t nvs_get_u8(nvs_handle_t handle, const char *key, uint8_t *out_value)
{
    uint64_t value = 0;
    const esp_err_t result = get_number(handle, key, NVS_VALUE_U8, &value);
    if (result == ESP_OK) {
        *out_value = (uint8_t)value;
    }
    return result;
}

esp_err_t nvs_get_u64(nvs_handle_t handle, const char *key, uint64_t *out_value)
{
    return get_number(handle, key, NVS_VALUE_U64, out_value);
}

esp_err_t nvs_erase_key(nvs_handle_t handle, const char *key)
{
    mock_nvs_handle_t *const target = open_handle(handle);
    if (!target->writable) {
        return ESP_ERR_NVS_READ_ONLY;
    }
    if (!mutation_allowed(key)) {
        return ESP_FAIL;
    }
    mock_nvs_entry_t *const entry = find_entry(target->namespace_name, key);
    if (entry == NULL) {
        return ESP_ERR_NVS_NOT_FOUND;
    }
    memset(entry, 0, sizeof(*entry));
    return ESP_OK;
}

esp_err_t nvs_commit(nvs_handle_t handle)
{
    if (!open_handle(handle)->writable) {
        return ESP_ERR_NVS_READ_ONLY;
    }
    return mutation_allowed("commit") ? ESP_OK : ESP_FAIL;
}

void nvs_close(nvs_handle_t handle)
{
    open_handle(handle)->open = false;
}

bool idf_mock_nvs_contains(const char *namespace_name, const char *key)
{
    return find_entry(namespace_name, key) != NULL;
}

const char *idf_mock_nvs_string(const char *namespace_name, const char *key)
{
    const mock_nvs_entry_t *const entry = find_entry(namespace_name, key);
    return entry != NULL && entry->type == NVS_VALUE_STRING ? entry->string : NULL;
}

size_t idf_mock_nvs_open_handles(void)
{
    size_t count = 0;
    for (size_t index = 0; index < MOCK_NVS_HANDLES; ++index) {
        count += nvs_handles[index].open ? 1U : 0U;
    }
    return count;
}
