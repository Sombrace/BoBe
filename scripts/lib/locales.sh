#!/usr/bin/env bash

BOBE_LOCALES=(en de-DE el-GR es-ES fr-FR ja-JP ko-KR pt-BR zh-CN)

bobe_check_bundle_locales() {
    local resources="$1" locale table
    shift
    for locale in "${BOBE_LOCALES[@]}"; do
        for table in "$@"; do
            [[ -f "$resources/$locale.lproj/$table" ]] || {
                echo "Missing $locale localization: $resources/$locale.lproj/$table" >&2
                return 1
            }
        done
    done
}
