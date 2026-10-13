#include "bobe_protocol.h"
#include "bobe_update_offer.h"
#include "bobe_text_view.h"

#include <cstring>

int main()
{
    char output[1024]{};
    char display[64]{};
    bobe_text_view_t view{};
    if (!bobe_text_view_init(&view, display, sizeof(display))) return 1;
    const auto result = bobe_protocol_build_text_input(
        output, sizeof(output), "00000000-0000-4000-8000-000000000001", "Hello from C++");
    return result == BOBE_PROTOCOL_OK && std::strstr(output, "Hello from C++") != nullptr ? 0 : 1;
}
