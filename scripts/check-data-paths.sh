#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
source_root="$root/BoBeService/src"
owner="$source_root/util/paths.rs"

found_owner=0
leaves=()
while IFS= read -r -d '' caller; do
  case "$caller" in
    "$owner") found_owner=1 ;;
    "$source_root/config/mod.rs") ;;
    *) leaves+=("$caller") ;;
  esac
done < <(rg --files-with-matches --null 'bobe_data_dir\(' "$source_root")

if ((found_owner == 0)); then
  printf 'Could not find the bobe_data_dir() owner in %s\n' "$owner" >&2
  exit 1
fi
if ((${#leaves[@]} > 0)); then
  printf 'Leaf runtime code must consume DataPaths, not bobe_data_dir(): %s\n' "${leaves[@]}" >&2
  exit 1
fi
printf 'Canonical data path ownership OK\n'
