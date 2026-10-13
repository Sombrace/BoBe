#include <libusb.h>

#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

static int parse_hex16(const char *text, uint16_t *value)
{
    char *end = NULL;
    errno = 0;
    const unsigned long parsed = strtoul(text, &end, 16);
    if (errno != 0 || end == text || *end != '\0' || parsed > UINT16_MAX) {
        return 0;
    }
    *value = (uint16_t)parsed;
    return 1;
}

int main(int argc, char **argv)
{
    if (argc != 3) {
        fprintf(stderr, "usage: sigrok-usb-identity VID PID\n");
        return 2;
    }
    uint16_t expected_vid;
    uint16_t expected_pid;
    if (!parse_hex16(argv[1], &expected_vid) ||
        !parse_hex16(argv[2], &expected_pid)) {
        fprintf(stderr, "VID and PID must be hexadecimal 16-bit values\n");
        return 2;
    }

    libusb_context *context = NULL;
    if (libusb_init(&context) != LIBUSB_SUCCESS) {
        fprintf(stderr, "could not initialize libusb\n");
        return 1;
    }
    libusb_device **devices = NULL;
    const ssize_t count = libusb_get_device_list(context, &devices);
    if (count < 0) {
        fprintf(stderr, "could not enumerate USB devices\n");
        libusb_exit(context);
        return 1;
    }

    int first = 1;
    printf("[");
    for (ssize_t index = 0; index < count; ++index) {
        struct libusb_device_descriptor descriptor;
        if (libusb_get_device_descriptor(devices[index], &descriptor) !=
            LIBUSB_SUCCESS) {
            continue;
        }
        if (descriptor.idVendor != expected_vid ||
            descriptor.idProduct != expected_pid) {
            continue;
        }
        uint8_t ports[8];
        const int port_count = libusb_get_port_numbers(
            devices[index],
            ports,
            sizeof(ports)
        );
        if (port_count < 0) {
            continue;
        }
        if (!first) {
            printf(",");
        }
        first = 0;
        printf(
            "{\"address\":%u,\"bcd_device\":\"%04x\",\"bus\":%u,"
            "\"ports\":[",
            libusb_get_device_address(devices[index]),
            descriptor.bcdDevice,
            libusb_get_bus_number(devices[index])
        );
        for (int port = 0; port < port_count; ++port) {
            printf("%s%u", port ? "," : "", ports[port]);
        }
        printf(
            "],\"speed\":%d,\"vid\":\"%04x\",\"pid\":\"%04x\"}",
            libusb_get_device_speed(devices[index]),
            descriptor.idVendor,
            descriptor.idProduct
        );
    }
    printf("]\n");
    libusb_free_device_list(devices, 1);
    libusb_exit(context);
    return 0;
}
