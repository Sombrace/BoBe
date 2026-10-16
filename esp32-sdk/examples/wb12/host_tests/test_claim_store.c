#include "bobe_claim_store.h"
#include "bobe_storage.h"
#include "idf_mock.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

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

static const bobe_claim_material_t FIRST_CLAIM = {
    .endpoint = "wss://hub-one.local:8767/body/v1",
    .trust_key_id = "kid-one",
    .controller_epoch = 1,
    .server_ca_pem = "server-ca-one",
    .client_cert_pem = "client-cert-one",
    .client_key_pem = "client-key-one",
};

static const bobe_claim_material_t SECOND_CLAIM = {
    .endpoint = "wss://hub-two.local:8767/body/v1",
    .trust_key_id = "kid-two",
    .controller_epoch = 2,
    .server_ca_pem = "server-ca-two",
    .client_cert_pem = "client-cert-two",
    .client_key_pem = "client-key-two",
};

static esp_err_t store_claim(const bobe_claim_material_t *material)
{
    bobe_claim_plan_t plan;
    esp_err_t result =
        bobe_claim_store_plan(material->controller_epoch, &plan);
    if (result == ESP_OK) {
        result = bobe_claim_store_commit(&plan, material);
    }
    CHECK(idf_mock_nvs_open_handles() == 0);
    return result;
}

static bool stored_string_is(
    const char *namespace_name,
    const char *key,
    const char *expected
)
{
    const char *const value = idf_mock_nvs_string(namespace_name, key);
    return value != NULL && strcmp(value, expected) == 0;
}

static void check_active(uint8_t slot, const bobe_claim_material_t *material)
{
    bobe_claim_state_t state;
    CHECK(bobe_claim_store_load_state(&state) == ESP_OK);
    CHECK(state.claim_ready);
    CHECK(state.active_slot_valid);
    CHECK(state.active_slot == slot);
    CHECK(state.active_epoch == material->controller_epoch);
    const bobe_storage_slot_keys_t *const keys = bobe_storage_slot_keys(slot);
    CHECK(stored_string_is(BOBE_CLAIM_NAMESPACE, keys->endpoint, material->endpoint));
    CHECK(stored_string_is(BOBE_CLAIM_NAMESPACE, keys->trust_key_id, material->trust_key_id));
    CHECK(stored_string_is(BOBE_TLS_NAMESPACE, keys->server_ca, material->server_ca_pem));
    CHECK(stored_string_is(BOBE_TLS_NAMESPACE, keys->client_cert, material->client_cert_pem));
    CHECK(stored_string_is(BOBE_TLS_NAMESPACE, keys->client_key, material->client_key_pem));
}

static bool slot_is_empty(uint8_t slot)
{
    const bobe_storage_slot_keys_t *const keys = bobe_storage_slot_keys(slot);
    return !idf_mock_nvs_contains(BOBE_CLAIM_NAMESPACE, keys->endpoint) &&
           !idf_mock_nvs_contains(BOBE_CLAIM_NAMESPACE, keys->trust_key_id) &&
           !idf_mock_nvs_contains(BOBE_CLAIM_NAMESPACE, keys->controller_epoch) &&
           !idf_mock_nvs_contains(BOBE_TLS_NAMESPACE, keys->server_ca) &&
           !idf_mock_nvs_contains(BOBE_TLS_NAMESPACE, keys->client_cert) &&
           !idf_mock_nvs_contains(BOBE_TLS_NAMESPACE, keys->client_key);
}

static void test_first_claim_and_rotation(void)
{
    idf_mock_nvs_reset();
    CHECK(store_claim(&FIRST_CLAIM) == ESP_OK);
    check_active(0, &FIRST_CLAIM);
    CHECK(slot_is_empty(1));

    CHECK(store_claim(&SECOND_CLAIM) == ESP_OK);
    check_active(1, &SECOND_CLAIM);
    CHECK(slot_is_empty(0));

    CHECK(store_claim(&SECOND_CLAIM) == ESP_ERR_INVALID_VERSION);
    CHECK(store_claim(&FIRST_CLAIM) == ESP_ERR_INVALID_VERSION);
    check_active(1, &SECOND_CLAIM);
}

static size_t activation_operation(void)
{
    const size_t operations = idf_mock_nvs_operation_count();
    for (size_t operation = 1; operation <= operations; ++operation) {
        if (strcmp(
                idf_mock_nvs_operation_key(operation),
                BOBE_CLAIM_ACTIVE_SLOT_KEY
            ) == 0) {
            return operation;
        }
    }
    return 0;
}

static void test_first_claim_survives_every_storage_fault(void)
{
    idf_mock_nvs_reset();
    CHECK(store_claim(&FIRST_CLAIM) == ESP_OK);
    const size_t operations = idf_mock_nvs_operation_count();
    CHECK(activation_operation() > 0);
    for (size_t failure = 1; failure <= operations; ++failure) {
        idf_mock_nvs_reset();
        idf_mock_nvs_fail_operation(failure);
        const esp_err_t result = store_claim(&FIRST_CLAIM);
        idf_mock_nvs_fail_operation(0);
        if (result != ESP_OK) {
            bobe_claim_state_t state;
            CHECK(bobe_claim_store_load_state(&state) == ESP_OK);
            CHECK(!state.claim_ready);
            CHECK(store_claim(&FIRST_CLAIM) == ESP_OK);
        }
        check_active(0, &FIRST_CLAIM);
    }
}

static void test_rotation_survives_every_storage_fault(void)
{
    idf_mock_nvs_reset();
    CHECK(store_claim(&FIRST_CLAIM) == ESP_OK);
    idf_mock_nvs_start_counting();
    CHECK(store_claim(&SECOND_CLAIM) == ESP_OK);
    const size_t operations = idf_mock_nvs_operation_count();
    const size_t activation = activation_operation();
    CHECK(activation > 0);
    CHECK(operations > activation + 2U);

    for (size_t failure = 1; failure <= operations; ++failure) {
        idf_mock_nvs_reset();
        CHECK(store_claim(&FIRST_CLAIM) == ESP_OK);
        idf_mock_nvs_start_counting();
        idf_mock_nvs_fail_operation(failure);
        const esp_err_t result = store_claim(&SECOND_CLAIM);
        idf_mock_nvs_fail_operation(0);
        if (failure <= activation) {
            CHECK(result != ESP_OK);
            check_active(0, &FIRST_CLAIM);
            CHECK(store_claim(&SECOND_CLAIM) == ESP_OK);
        } else {
            CHECK(result == ESP_OK);
        }
        check_active(1, &SECOND_CLAIM);
        CHECK(store_claim(&SECOND_CLAIM) == ESP_ERR_INVALID_VERSION);
        CHECK(bobe_claim_store_retire_inactive() == ESP_OK);
        CHECK(slot_is_empty(0));
        check_active(1, &SECOND_CLAIM);
    }
}

int main(void)
{
    test_first_claim_and_rotation();
    test_first_claim_survives_every_storage_fault();
    test_rotation_survives_every_storage_fault();
    puts("BoBe claim storage fault tests passed.");
    return 0;
}
