#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

void idf_mock_reset(void);
void idf_mock_violation(const char *message);

void idf_mock_fail_task_creation(unsigned int call);
size_t idf_mock_live_tasks(void);
bool idf_mock_task_alive(const char *name);
size_t idf_mock_live_semaphores(void);
size_t idf_mock_live_event_groups(void);
size_t idf_mock_live_queues(void);
size_t idf_mock_live_allocations(void);

void idf_mock_fail_i2s_new_channel(unsigned int call);
void idf_mock_fail_i2s_init(unsigned int call);
size_t idf_mock_live_i2s_channels(void);
size_t idf_mock_running_i2s_rx_channels(void);
unsigned int idf_mock_i2s_rejected_enables(void);

void idf_mock_fail_afe_creation(bool fail);
void idf_mock_provide_wake_model(bool provide);
bool idf_mock_speech_models_loaded(void);
bool idf_mock_afe_created(void);

void idf_mock_nvs_reset(void);
void idf_mock_nvs_start_counting(void);
void idf_mock_nvs_fail_operation(size_t operation);
size_t idf_mock_nvs_operation_count(void);
const char *idf_mock_nvs_operation_key(size_t operation);
bool idf_mock_nvs_contains(const char *namespace_name, const char *key);
const char *idf_mock_nvs_string(const char *namespace_name, const char *key);
size_t idf_mock_nvs_open_handles(void);
