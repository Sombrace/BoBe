if(NOT MODE STREQUAL "compare" AND NOT MODE STREQUAL "regenerate")
    message(FATAL_ERROR "MODE must be compare or regenerate")
endif()

file(REMOVE_RECURSE "${OUTPUT_DIR}")
file(MAKE_DIRECTORY "${OUTPUT_DIR}")
execute_process(
    COMMAND "${RENDERER}" "${OUTPUT_DIR}"
    RESULT_VARIABLE render_result
    OUTPUT_VARIABLE rendered_cases
    ERROR_VARIABLE render_error
)
if(NOT render_result EQUAL 0)
    message(FATAL_ERROR "UI renderer failed:\n${render_error}")
endif()

string(REPLACE "\r\n" "\n" rendered_cases "${rendered_cases}")
string(REGEX REPLACE "\n$" "" rendered_cases "${rendered_cases}")
string(REPLACE "\n" ";" rendered_cases "${rendered_cases}")

set(generated_references "")
foreach(case_name IN LISTS rendered_cases)
    set(raw_file "${OUTPUT_DIR}/${case_name}.rgb565")
    if(NOT EXISTS "${raw_file}")
        message(FATAL_ERROR "Renderer did not produce ${raw_file}")
    endif()
    file(SHA256 "${raw_file}" raw_hash)
    string(APPEND generated_references "${raw_hash}  ${case_name}.rgb565\n")
endforeach()

if(MODE STREQUAL "regenerate")
    file(WRITE "${REFERENCE_FILE}" "${generated_references}")
    list(LENGTH rendered_cases case_count)
    message(STATUS "Regenerated ${case_count} canonical RGB565 hashes")
    return()
endif()

if(NOT EXISTS "${REFERENCE_FILE}")
    message(FATAL_ERROR
        "Missing UI references: ${REFERENCE_FILE}. "
        "Run the explicit bobe_ui_regenerate target."
    )
endif()
file(READ "${REFERENCE_FILE}" expected_references)
if(NOT expected_references STREQUAL generated_references)
    foreach(case_name IN LISTS rendered_cases)
        string(
            REGEX MATCH
            "[0-9a-f]+  ${case_name}\\.rgb565"
            expected_line
            "${expected_references}"
        )
        if(expected_line STREQUAL "")
            message(STATUS "Missing reference: ${case_name}")
        else()
            string(REGEX MATCH "^[0-9a-f]+" expected_hash "${expected_line}")
            set(raw_file "${OUTPUT_DIR}/${case_name}.rgb565")
            file(SHA256 "${raw_file}" actual_hash)
            if(NOT expected_hash STREQUAL actual_hash)
                message(STATUS
                    "Mismatch ${case_name}: expected ${expected_hash}, "
                    "actual ${actual_hash}"
                )
            endif()
        endif()
    endforeach()
    message(FATAL_ERROR
        "Canonical RGB565 output differs from ${REFERENCE_FILE}; "
        "rendered buffers remain in ${OUTPUT_DIR}"
    )
endif()

list(LENGTH rendered_cases case_count)
message(STATUS "Matched ${case_count} canonical RGB565 references")
