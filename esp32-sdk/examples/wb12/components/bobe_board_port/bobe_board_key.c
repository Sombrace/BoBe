#include "bobe_board.h"

extern const unsigned char key_start[] asm("_binary_update_signing_public_pem_start");
extern const unsigned char key_end[] asm("_binary_update_signing_public_pem_end");

const unsigned char *bobe_board_update_public_key(size_t *length)
{
    if (length == NULL) {
        return NULL;
    }
    *length = (size_t)(key_end - key_start);
    return key_start;
}
