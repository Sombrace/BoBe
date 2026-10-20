set(bobe_speech_requires "")
set(bobe_speech_fragments "")
if(CONFIG_BOBE_ENABLE_ESP_SR)
    list(APPEND bobe_speech_requires esp-sr)
    list(APPEND bobe_speech_fragments "linker.lf")
endif()

idf_component_register(
    SRCS "bobe_voice_frontend.c"
    INCLUDE_DIRS "include"
    REQUIRES
        bobe_platform bobe_common
        ${bobe_speech_requires}
        esp_timer
        freertos
        heap
        log
    LDFRAGMENTS ${bobe_speech_fragments}
)
