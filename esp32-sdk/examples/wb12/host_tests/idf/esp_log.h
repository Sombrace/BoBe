#pragma once

void idf_mock_log(const char *tag, const char *format, ...)
    __attribute__((format(printf, 2, 3)));

#define ESP_LOGE(...) idf_mock_log(__VA_ARGS__)
#define ESP_LOGW(...) idf_mock_log(__VA_ARGS__)
#define ESP_LOGI(...) idf_mock_log(__VA_ARGS__)
#define ESP_LOGD(...) idf_mock_log(__VA_ARGS__)
