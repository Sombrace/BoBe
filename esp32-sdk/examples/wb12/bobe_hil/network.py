import ipaddress
import json
import os
import re
import secrets
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from bobe_hil.network_profiles import NETWORK_PROFILES, NetworkLease
from bobe_hil.paths import private_key_file, private_root, regular_file


NETWORK_CONFIG_SCHEMA = 1
NETWORK_STATE_SCHEMA = 1
DEFAULT_TTL_SECONDS = 60
MIN_TTL_SECONDS = 5
MAX_TTL_SECONDS = 600
RFC1918_NETWORKS = tuple(
    ipaddress.ip_network(value)
    for value in ("10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16")
)
IMPAIRMENT_PROFILES = tuple(sorted(NETWORK_PROFILES - {"clean"}))
STATUS_PATTERN = re.compile(
    r"^OK STATUS lease=(none|[0-9a-f]{32}) "
    r"profile=([a-z0-9-]+) expires=([0-9]+)$"
)


@dataclass(frozen=True)
class NetworkConfig:
    host: str
    root: Path


def network_root(root: Path | None = None) -> Path:
    return private_root(root) / "network"


def load_network_config(root: Path | None = None) -> NetworkConfig:
    location = network_root(root)
    if location.is_symlink() or not location.is_dir():
        raise ValueError("OpenWrt configuration root is unavailable")
    path = regular_file(location / "openwrt.json", "OpenWrt configuration")
    private_key_file(location / "router_ed25519", "OpenWrt SSH key")
    regular_file(location / "known_hosts", "OpenWrt known-hosts file")
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError("OpenWrt configuration is invalid JSON") from error
    if not isinstance(value, dict):
        raise ValueError("OpenWrt configuration must be a JSON object")
    host = value.get("host")
    if not isinstance(host, str):
        raise ValueError("OpenWrt host must be a fixed IP address")
    try:
        address = ipaddress.ip_address(host)
    except ValueError as error:
        raise ValueError("OpenWrt host must be a fixed IP address") from error
    if (
        value.get("schema_version") != NETWORK_CONFIG_SCHEMA
        or address.version != 4
        or not any(address in network for network in RFC1918_NETWORKS)
        or address.is_loopback
        or address.is_link_local
        or address.is_multicast
        or address.is_unspecified
    ):
        raise ValueError("OpenWrt configuration values are invalid")
    return NetworkConfig(str(address), location)


def inspect_network(root: Path | None = None) -> dict[str, Any]:
    try:
        config = load_network_config(root)
    except (OSError, ValueError) as error:
        return {
            "configured": False,
            "available": False,
            "blocker": str(error),
        }
    return {
        "configured": True,
        "available": True,
        "blocker": None,
        "host": config.host,
        "config_root": str(config.root),
    }


class OpenWrtTransport:
    def __init__(self, config: NetworkConfig) -> None:
        self.command = [
            "/usr/bin/ssh",
            "-F",
            "/dev/null",
            "-T",
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=5",
            "-o",
            "GlobalKnownHostsFile=/dev/null",
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "LogLevel=ERROR",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            f"UserKnownHostsFile={config.root / 'known_hosts'}",
            "-i",
            str(config.root / "router_ed25519"),
            f"root@{config.host}",
            "/usr/sbin/bobe-netem",
        ]

    def run(self, arguments: tuple[str, ...]) -> str:
        result = subprocess.run(
            [*self.command, *arguments],
            check=False,
            capture_output=True,
            text=True,
            timeout=15.0,
            env={
                "HOME": "/var/empty",
                "LC_ALL": "C",
                "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
            },
        )
        if result.returncode != 0:
            message = result.stderr.strip().splitlines()
            raise RuntimeError(
                "OpenWrt operation failed"
                + (f": {message[0]}" if message else "")
            )
        output = result.stdout.strip()
        if not output.startswith("OK "):
            raise RuntimeError("OpenWrt response framing is invalid")
        return output


def state_path(config: NetworkConfig) -> Path:
    return config.root / "active-lease.json"


def load_state(config: NetworkConfig) -> dict[str, Any] | None:
    path = state_path(config)
    if path.is_symlink():
        raise ValueError("OpenWrt lease state is a symbolic link")
    if not path.exists():
        return None
    regular_file(path, "OpenWrt lease state")
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError("OpenWrt lease state is invalid JSON") from error
    if (
        not isinstance(value, dict)
        or set(value) != {
            "schema_version",
            "lease",
            "profile",
            "expires",
            "phase",
        }
        or value["schema_version"] != NETWORK_STATE_SCHEMA
        or not isinstance(value["lease"], str)
        or re.fullmatch(r"[0-9a-f]{32}", value["lease"]) is None
        or value["profile"] not in NETWORK_PROFILES
        or value["phase"] not in {"active", "pending"}
        or type(value["expires"]) is not int
        or (
            value["expires"] <= 0
            if value["phase"] == "active"
            else value["expires"] != 0
        )
    ):
        raise ValueError("OpenWrt lease state fields are invalid")
    return value


def write_state(
    config: NetworkConfig,
    value: dict[str, Any],
    *,
    replace: bool = False,
) -> None:
    path = state_path(config)
    if path.is_symlink() or (path.exists() and not replace):
        raise ValueError("OpenWrt lease state already exists")
    if replace and not path.is_file():
        raise ValueError("OpenWrt lease state cannot be replaced safely")
    temporary = path.with_name(f".{path.name}.{secrets.token_hex(8)}.tmp")
    descriptor = os.open(
        temporary,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
        0o600,
    )
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as output:
            json.dump(value, output, separators=(",", ":"), sort_keys=True)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def parse_status(output: str) -> dict[str, Any]:
    first_line = output.splitlines()[0]
    match = STATUS_PATTERN.fullmatch(first_line)
    if match is None:
        raise RuntimeError("OpenWrt status response is invalid")
    return {
        "lease": None if match.group(1) == "none" else match.group(1),
        "profile": match.group(2),
        "expires": int(match.group(3)),
        "raw": output,
    }


def reconcile_status(
    config: NetworkConfig,
    transport: OpenWrtTransport,
) -> dict[str, Any]:
    remote = parse_status(transport.run(("status",)))
    local = load_state(config)
    if local is not None and remote["lease"] is None:
        state_path(config).unlink()
        local = None
    elif local is None and remote["lease"] is not None:
        raise RuntimeError(
            "OpenWrt holds a lease this repository did not record; "
            "clear it on the router before applying a new profile"
        )
    elif local is not None:
        if (
            remote["lease"] != local["lease"]
            or remote["profile"] != local["profile"]
        ):
            raise RuntimeError("OpenWrt lease state does not match local state")
        if local["phase"] == "pending":
            local = {
                **local,
                "phase": "active",
                "expires": remote["expires"],
            }
            write_state(config, local, replace=True)
        elif remote["expires"] != local["expires"]:
            raise RuntimeError("OpenWrt lease expiry does not match local state")
    return {"local": local, "remote": remote}


def status(root: Path | None = None) -> dict[str, Any]:
    config = load_network_config(root)
    transport = OpenWrtTransport(config)
    return {
        "operation": "network-status",
        "host": config.host,
        **reconcile_status(config, transport),
    }


def clear(root: Path | None = None) -> dict[str, Any]:
    config = load_network_config(root)
    transport = OpenWrtTransport(config)
    state = reconcile_status(config, transport)
    local = state["local"]
    if local is None:
        return {
            "operation": "network-clear",
            "cleared": False,
            "reason": "no-active-lease",
        }
    response = transport.run(("clear", str(local["lease"])))
    if response != f"OK CLEAR lease={local['lease']}":
        raise RuntimeError("OpenWrt clear response is invalid")
    state_path(config).unlink()
    return {
        "operation": "network-clear",
        "cleared": True,
        "lease": local["lease"],
    }


def apply(
    profile: str,
    ttl_seconds: int = DEFAULT_TTL_SECONDS,
    *,
    root: Path | None = None,
) -> dict[str, Any]:
    if profile == "clean":
        return clear(root)
    if profile not in NETWORK_PROFILES:
        raise ValueError(
            f"network profile must be clean or one of: "
            f"{', '.join(IMPAIRMENT_PROFILES)}"
        )
    if type(ttl_seconds) is not int or not (
        MIN_TTL_SECONDS <= ttl_seconds <= MAX_TTL_SECONDS
    ):
        raise ValueError(
            f"network lease TTL must be an integer between "
            f"{MIN_TTL_SECONDS} and {MAX_TTL_SECONDS} seconds"
        )
    config = load_network_config(root)
    transport = OpenWrtTransport(config)
    state = reconcile_status(config, transport)
    if state["local"] is not None:
        raise RuntimeError("an OpenWrt impairment lease is already active")
    lease = NetworkLease(secrets.token_hex(16), profile, ttl_seconds)
    pending_state = {
        "schema_version": NETWORK_STATE_SCHEMA,
        "lease": lease.lease,
        "profile": profile,
        "expires": 0,
        "phase": "pending",
    }
    try:
        write_state(config, pending_state)
    except (OSError, ValueError) as error:
        raise RuntimeError("could not persist the pending OpenWrt lease") from error
    try:
        response = transport.run(lease.apply_arguments()[1:])
        expected_prefix = (
            f"OK APPLY lease={lease.lease} profile={profile} expires="
        )
        if not response.startswith(expected_prefix):
            raise RuntimeError("OpenWrt apply response is invalid")
        try:
            expires = int(response.removeprefix(expected_prefix).splitlines()[0])
        except ValueError as error:
            raise RuntimeError("OpenWrt apply expiry is invalid") from error
        now = int(time.time())
        if not now + ttl_seconds - 2 <= expires <= now + ttl_seconds + 2:
            raise RuntimeError("OpenWrt apply expiry is outside bounds")
        active_state = {
            "schema_version": NETWORK_STATE_SCHEMA,
            "lease": lease.lease,
            "profile": profile,
            "expires": expires,
            "phase": "active",
        }
        write_state(config, active_state, replace=True)
    except (
        OSError,
        RuntimeError,
        subprocess.TimeoutExpired,
        ValueError,
    ) as error:
        try:
            cleared = transport.run(("clear", lease.lease))
            if cleared != f"OK CLEAR lease={lease.lease}":
                raise RuntimeError("OpenWrt rollback response is invalid")
        except (OSError, RuntimeError, subprocess.TimeoutExpired) as clear_error:
            raise RuntimeError(
                f"OpenWrt apply failed; rollback failed: {clear_error}"
            ) from clear_error
        state_path(config).unlink(missing_ok=True)
        raise RuntimeError("OpenWrt apply failed; rolled back") from error
    return {
        "operation": "network-apply",
        "profile": profile,
        "ttl_seconds": ttl_seconds,
        "lease": active_state,
        "response": response,
    }
