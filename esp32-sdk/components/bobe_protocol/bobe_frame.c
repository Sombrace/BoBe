#include "bobe_protocol.h"

static void write_u16_be(uint8_t *output, uint16_t value)
{
    output[0] = (uint8_t)(value >> 8);
    output[1] = (uint8_t)value;
}

static void write_u32_be(uint8_t *output, uint32_t value)
{
    output[0] = (uint8_t)(value >> 24);
    output[1] = (uint8_t)(value >> 16);
    output[2] = (uint8_t)(value >> 8);
    output[3] = (uint8_t)value;
}

static void write_u64_be(uint8_t *output, uint64_t value)
{
    for (size_t index = 0; index < 8; ++index) {
        output[index] = (uint8_t)(value >> (56U - index * 8U));
    }
}

static uint16_t read_u16_be(const uint8_t *input)
{
    return (uint16_t)(((uint16_t)input[0] << 8) | input[1]);
}

static uint32_t read_u32_be(const uint8_t *input)
{
    return ((uint32_t)input[0] << 24) |
           ((uint32_t)input[1] << 16) |
           ((uint32_t)input[2] << 8) |
           input[3];
}

static uint64_t read_u64_be(const uint8_t *input)
{
    uint64_t value = 0;
    for (size_t index = 0; index < 8; ++index) {
        value = (value << 8) | input[index];
    }
    return value;
}

void bobe_media_header_encode(
    const bobe_media_header_t *header,
    uint8_t output[BOBE_MEDIA_HEADER_BYTES]
)
{
    output[0] = header->protocol_major;
    output[1] = (uint8_t)header->kind;
    write_u16_be(&output[2], header->flags);
    write_u32_be(&output[4], header->stream_id);
    write_u32_be(&output[8], header->sequence);
    write_u64_be(&output[12], header->monotonic_timestamp_us);
}

bobe_protocol_result_t bobe_media_header_decode(
    const uint8_t *input,
    size_t input_size,
    bobe_media_header_t *header
)
{
    if (input == NULL || header == NULL) {
        return BOBE_PROTOCOL_INVALID_ARGUMENT;
    }
    if (input_size < BOBE_MEDIA_HEADER_BYTES) {
        return BOBE_PROTOCOL_MALFORMED;
    }
    if (input[0] != BOBE_PROTOCOL_MAJOR) {
        return BOBE_PROTOCOL_UNSUPPORTED;
    }
    if (input[1] < BOBE_MEDIA_MICROPHONE_PCM || input[1] > BOBE_MEDIA_DIAGNOSTIC) {
        return BOBE_PROTOCOL_UNSUPPORTED;
    }

    header->protocol_major = input[0];
    header->kind = (bobe_media_kind_t)input[1];
    header->flags = read_u16_be(&input[2]);
    header->stream_id = read_u32_be(&input[4]);
    header->sequence = read_u32_be(&input[8]);
    header->monotonic_timestamp_us = read_u64_be(&input[12]);
    if ((header->flags & ~BOBE_MEDIA_FLAG_DISCONTINUITY) != 0 ||
        header->stream_id == 0) {
        return BOBE_PROTOCOL_MALFORMED;
    }
    return BOBE_PROTOCOL_OK;
}

bobe_protocol_result_t bobe_media_validate_payload(
    const bobe_media_header_t *header,
    size_t payload_size
)
{
    if (header == NULL) {
        return BOBE_PROTOCOL_INVALID_ARGUMENT;
    }

    switch (header->kind) {
    case BOBE_MEDIA_MICROPHONE_PCM:
        return payload_size == BOBE_MIC_FRAME_SAMPLES * sizeof(int16_t)
                   ? BOBE_PROTOCOL_OK
                   : BOBE_PROTOCOL_RANGE_ERROR;
    case BOBE_MEDIA_SPEAKER_PCM:
        return payload_size == BOBE_SPEAKER_FRAME_SAMPLES * sizeof(int16_t)
                   ? BOBE_PROTOCOL_OK
                   : BOBE_PROTOCOL_RANGE_ERROR;
    case BOBE_MEDIA_OPUS:
        return payload_size > 0 && payload_size <= 512
                   ? BOBE_PROTOCOL_OK
                   : BOBE_PROTOCOL_RANGE_ERROR;
    case BOBE_MEDIA_DIAGNOSTIC:
        return payload_size <= 1024 ? BOBE_PROTOCOL_OK : BOBE_PROTOCOL_TOO_LARGE;
    default:
        return BOBE_PROTOCOL_UNSUPPORTED;
    }
}
