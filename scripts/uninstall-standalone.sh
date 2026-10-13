#!/bin/sh
set -eu

prefix="${BOBE_INSTALL_PREFIX:-$HOME/.local}"
source_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
remove_service=true
case "${1:-}" in
    "") ;;
    --files-only) remove_service=false ;;
    *)
        printf '%s\n' "Usage: $0 [--files-only]" >&2
        exit 2
        ;;
esac
service_cli="$prefix/bin/bobe"
if [ "$remove_service" = false ]; then
    case "$(uname -s)" in
        Darwin) service_definition="$HOME/Library/LaunchAgents/com.bobe.daemon.plist" ;;
        *)
            case "${XDG_CONFIG_HOME:-}" in
                /*) config_home="$XDG_CONFIG_HOME" ;;
                *) config_home="$HOME/.config" ;;
            esac
            service_definition="$config_home/systemd/user/bobe.service"
            ;;
    esac
    if [ -f "$service_definition" ]; then
        printf '%s\n' "Cannot remove BoBe files while the user service is installed. Run '$0' to remove the service too." >&2
        exit 1
    fi
fi
if [ "$remove_service" = true ]; then
    if [ ! -x "$service_cli" ]; then
        service_cli="$source_dir/bin/bobe"
    fi
    if [ ! -x "$service_cli" ]; then
        printf '%s\n' "Cannot remove the BoBe service: no service CLI is available." >&2
        exit 1
    fi
    "$service_cli" service uninstall
fi
rm -f "$prefix/bin/bobe"
rm -rf "$prefix/share/bobe"
if [ "$remove_service" = true ]; then
    printf '%s\n' "Removed BoBe CLI, version store, and user service. Personal data was preserved."
else
    printf '%s\n' "Removed BoBe CLI and version store. No user service was installed; personal data was preserved."
fi
