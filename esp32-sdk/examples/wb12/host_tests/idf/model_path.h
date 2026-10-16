#pragma once

typedef struct {
    int num;
} srmodel_list_t;

srmodel_list_t *esp_srmodel_init(const char *partition_label);
void esp_srmodel_deinit(srmodel_list_t *models);
char *esp_srmodel_filter(
    srmodel_list_t *models,
    const char *keyword1,
    const char *keyword2
);
