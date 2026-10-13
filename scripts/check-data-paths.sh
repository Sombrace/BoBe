#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
source_root="$root/BoBeService/src"

callers="$(rg -l 'bobe_data_dir\(' "$source_root" || true)"
for caller in $callers; do
  case "$caller" in
    "$source_root/config/mod.rs"|"$source_root/util/paths.rs") ;;
    *)
      printf 'Leaf runtime code must consume DataPaths, not bobe_data_dir(): %s\n' "$caller" >&2
      exit 1
      ;;
  esac
done

printf 'Canonical data path ownership OK\n'
