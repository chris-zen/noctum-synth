// Raw USB Audio isochronous-IN smoke test for macOS/Linux.
//
// This bypasses CoreAudio, which can return zero-filled input when macOS
// microphone privacy has not been granted to the invoking process.

#include <libusb.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

// Keep enough frames queued that userspace callback latency does not create a
// gap in the host's periodic schedule between transfers.
enum { PACKETS_PER_TRANSFER = 128, TRANSFER_COUNT = 50 };

struct completion {
    int done;
};

static void transfer_complete(struct libusb_transfer *transfer)
{
    struct completion *completion = transfer->user_data;
    completion->done = 1;
}

static int parse_u16(const char *text, uint16_t *value)
{
    char *end = NULL;
    unsigned long parsed = strtoul(text, &end, 0);
    if (end == text || *end != '\0' || parsed > UINT16_MAX) {
        return -1;
    }
    *value = (uint16_t)parsed;
    return 0;
}

static int find_audio_input(
    libusb_device *device,
    int *interface_number,
    int *alternate_setting,
    uint8_t *endpoint_address,
    int *packet_size)
{
    struct libusb_config_descriptor *configuration = NULL;
    int result = libusb_get_active_config_descriptor(device, &configuration);
    if (result != LIBUSB_SUCCESS) {
        return result;
    }

    result = LIBUSB_ERROR_NOT_FOUND;
    for (uint8_t i = 0; i < configuration->bNumInterfaces; ++i) {
        const struct libusb_interface *interface = &configuration->interface[i];
        for (int a = 0; a < interface->num_altsetting; ++a) {
            const struct libusb_interface_descriptor *alternate = &interface->altsetting[a];
            for (uint8_t e = 0; e < alternate->bNumEndpoints; ++e) {
                const struct libusb_endpoint_descriptor *endpoint = &alternate->endpoint[e];
                if ((endpoint->bEndpointAddress & LIBUSB_ENDPOINT_DIR_MASK) != LIBUSB_ENDPOINT_IN ||
                    (endpoint->bmAttributes & LIBUSB_TRANSFER_TYPE_MASK) !=
                        LIBUSB_TRANSFER_TYPE_ISOCHRONOUS) {
                    continue;
                }
                *interface_number = alternate->bInterfaceNumber;
                *alternate_setting = alternate->bAlternateSetting;
                *endpoint_address = endpoint->bEndpointAddress;
                *packet_size = endpoint->wMaxPacketSize & 0x7ff;
                result = LIBUSB_SUCCESS;
                goto finished;
            }
        }
    }

finished:
    libusb_free_config_descriptor(configuration);
    return result;
}

int main(int argc, char **argv)
{
    if (argc != 3) {
        fprintf(stderr, "usage: %s VID PID\n", argv[0]);
        return 2;
    }

    uint16_t vendor_id = 0;
    uint16_t product_id = 0;
    if (parse_u16(argv[1], &vendor_id) != 0 || parse_u16(argv[2], &product_id) != 0) {
        fprintf(stderr, "VID and PID must be 16-bit integers (for example 0xc0de 0xcafe)\n");
        return 2;
    }

    libusb_context *context = NULL;
    int result = libusb_init(&context);
    if (result != LIBUSB_SUCCESS) {
        fprintf(stderr, "libusb_init: %s\n", libusb_error_name(result));
        return 1;
    }

    libusb_device_handle *handle = libusb_open_device_with_vid_pid(context, vendor_id, product_id);
    if (handle == NULL) {
        fprintf(stderr, "USB device %04x:%04x not found or cannot be opened\n", vendor_id, product_id);
        libusb_exit(context);
        return 1;
    }

    int interface_number = 0;
    int alternate_setting = 0;
    uint8_t endpoint_address = 0;
    int packet_size = 0;
    result = find_audio_input(
        libusb_get_device(handle),
        &interface_number,
        &alternate_setting,
        &endpoint_address,
        &packet_size);
    if (result != LIBUSB_SUCCESS) {
        fprintf(stderr, "audio isochronous IN endpoint: %s\n", libusb_error_name(result));
        goto close;
    }

    int kernel_active = libusb_kernel_driver_active(handle, interface_number);
    fprintf(stderr, "audio interface kernel driver state: %s\n",
            kernel_active == 1 ? "active" : kernel_active == 0 ? "inactive" : libusb_error_name(kernel_active));
    int detach_result = libusb_set_auto_detach_kernel_driver(handle, 1);
    if (detach_result != LIBUSB_SUCCESS && detach_result != LIBUSB_ERROR_NOT_SUPPORTED) {
        fprintf(stderr, "enable automatic kernel-driver detach: %s\n",
                libusb_error_name(detach_result));
    }
    result = libusb_claim_interface(handle, interface_number);
    if (result != LIBUSB_SUCCESS) {
        fprintf(stderr, "claim audio interface %d: %s\n", interface_number, libusb_error_name(result));
        goto close;
    }
    result = libusb_set_interface_alt_setting(handle, interface_number, alternate_setting);
    if (result != LIBUSB_SUCCESS) {
        fprintf(stderr, "select audio alternate setting %d: %s\n", alternate_setting,
                libusb_error_name(result));
        goto release;
    }

    fprintf(stderr, "capturing raw USB audio: interface=%d alt=%d endpoint=0x%02x max_packet=%d\n",
            interface_number, alternate_setting, endpoint_address, packet_size);

    size_t nonzero_bytes = 0;
    size_t received_bytes = 0;
    size_t completed_packets = 0;
    size_t failed_packets = 0;
    uint32_t peak_sample = 0;
    const int buffer_size = packet_size * PACKETS_PER_TRANSFER;
    unsigned char *buffer = malloc((size_t)buffer_size);
    struct libusb_transfer *transfer = libusb_alloc_transfer(PACKETS_PER_TRANSFER);
    if (buffer == NULL || transfer == NULL) {
        fprintf(stderr, "cannot allocate isochronous transfer\n");
        result = LIBUSB_ERROR_NO_MEM;
        free(buffer);
        if (transfer != NULL) {
            libusb_free_transfer(transfer);
        }
        goto idle;
    }

    for (int iteration = 0; iteration < TRANSFER_COUNT; ++iteration) {
        memset(buffer, 0, (size_t)buffer_size);
        struct completion completion = {0};
        libusb_fill_iso_transfer(
            transfer,
            handle,
            endpoint_address,
            buffer,
            buffer_size,
            PACKETS_PER_TRANSFER,
            transfer_complete,
            &completion,
            1000);
        libusb_set_iso_packet_lengths(transfer, (unsigned int)packet_size);

        result = libusb_submit_transfer(transfer);
        if (result != LIBUSB_SUCCESS) {
            fprintf(stderr, "submit isochronous transfer: %s\n", libusb_error_name(result));
            break;
        }
        while (!completion.done) {
            result = libusb_handle_events_completed(context, &completion.done);
            if (result != LIBUSB_SUCCESS) {
                fprintf(stderr, "handle USB events: %s\n", libusb_error_name(result));
                break;
            }
        }
        if (result != LIBUSB_SUCCESS) {
            break;
        }
        if (transfer->status != LIBUSB_TRANSFER_COMPLETED) {
            fprintf(stderr, "isochronous transfer status=%d\n", transfer->status);
            result = LIBUSB_ERROR_IO;
            break;
        }

        for (int packet_index = 0; packet_index < PACKETS_PER_TRANSFER; ++packet_index) {
            struct libusb_iso_packet_descriptor *packet = &transfer->iso_packet_desc[packet_index];
            if (packet->status != LIBUSB_TRANSFER_COMPLETED) {
                ++failed_packets;
                continue;
            }
            ++completed_packets;
            unsigned char *data = libusb_get_iso_packet_buffer_simple(transfer, packet_index);
            received_bytes += packet->actual_length;
            for (unsigned int byte = 0; byte < packet->actual_length; ++byte) {
                nonzero_bytes += data[byte] != 0;
            }
            for (unsigned int byte = 0; byte + 2 < packet->actual_length; byte += 3) {
                uint32_t raw = (uint32_t)data[byte] | ((uint32_t)data[byte + 1] << 8) |
                               ((uint32_t)data[byte + 2] << 16);
                int32_t sample = (raw & 0x00800000U) != 0
                                     ? (int32_t)(raw | 0xff000000U)
                                     : (int32_t)raw;
                uint32_t magnitude = sample < 0 ? (uint32_t)(-(int64_t)sample) : (uint32_t)sample;
                if (magnitude > peak_sample) {
                    peak_sample = magnitude;
                }
            }
        }
    }

    libusb_free_transfer(transfer);
    free(buffer);
    fprintf(stderr,
            "raw USB audio result: packets=%zu failed=%zu bytes=%zu nonzero_bytes=%zu "
            "peak=%u peak_fraction=%.8f\n",
            completed_packets, failed_packets, received_bytes, nonzero_bytes, peak_sample,
            (double)peak_sample / 8388607.0);
    if (result == LIBUSB_SUCCESS &&
        (received_bytes == 0 || nonzero_bytes == 0 || peak_sample <= 84)) {
        result = LIBUSB_ERROR_IO;
    }

idle:
    (void)libusb_set_interface_alt_setting(handle, interface_number, 0);
release:
    (void)libusb_release_interface(handle, interface_number);
close:
    libusb_close(handle);
    libusb_exit(context);
    return result == LIBUSB_SUCCESS ? 0 : 1;
}
