#include "bobe_battery.h"

#include <stddef.h>

uint8_t bobe_battery_percent_from_raw(int raw_adc)
{
    static const struct {
        int raw;
        uint8_t percent;
    } points[] = {
        {1970, 0},
        {2062, 20},
        {2154, 40},
        {2246, 60},
        {2338, 80},
        {2430, 100},
    };

    if (raw_adc <= points[0].raw) {
        return 0;
    }
    for (size_t index = 1; index < sizeof(points) / sizeof(points[0]); ++index) {
        if (raw_adc <= points[index].raw) {
            const int raw_span = points[index].raw - points[index - 1].raw;
            const int percent_span =
                points[index].percent - points[index - 1].percent;
            return (uint8_t)(
                points[index - 1].percent +
                ((raw_adc - points[index - 1].raw) * percent_span) / raw_span
            );
        }
    }
    return 100;
}
