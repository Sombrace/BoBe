set(bobe_protocol_sources
    "bobe_control.c" "bobe_frame.c" "bobe_update_manifest.c" "bobe_update_offer.c"
    "bobe_json.c" "bobe_interaction.c"
)
if(COMMAND idf_component_register)
    idf_component_register(SRCS ${bobe_protocol_sources} INCLUDE_DIRS "include" REQUIRES cjson)
else()
    add_library(bobe_protocol STATIC ${bobe_protocol_sources})
    target_include_directories(bobe_protocol PUBLIC include)
    target_link_libraries(bobe_protocol PRIVATE cjson)
    target_compile_features(bobe_protocol PUBLIC c_std_11)
endif()
