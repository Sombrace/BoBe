#!/usr/bin/env python3

import argparse
import base64
import os
import re
import secrets
import sys
import time
from pathlib import Path

import serial
from serial.tools import list_ports

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))

from bobe_hil.device import ESPRESSIF_USB_VID, ESP32S3_USB_SERIAL_PID
from bobe_hil.paths import exclusive_device_access


PREFIX = "@BOBE-WIFI/1 "
IDENTITY_PATTERN = re.compile(
    r"^@BOBE-WIFI/1 ID ([0-9a-f]{32}) ([0-9a-f]{12}) WB-12$"
)


def base64url(value: bytes) -> str:
    return base64.urlsafe_b64encode(value).decode("ascii").rstrip("=")


def load_env(path: Path) -> tuple[str, str]:
    values: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line or line.lstrip().startswith("#") or "=" not in line:
            continue
        name, value = line.split("=", 1)
        values[name.strip()] = value.strip().strip("'\"")
    ssid = values.get("BOBE_WIFI_SSID", "")
    password = values.get("BOBE_WIFI_PASSWORD", "")
    if not 1 <= len(ssid.encode()) <= 32:
        raise ValueError("BOBE_WIFI_SSID must contain 1 to 32 UTF-8 bytes")
    if password and not 8 <= len(password.encode()) <= 63:
        raise ValueError("BOBE_WIFI_PASSWORD must contain 8 to 63 UTF-8 bytes")
    return ssid, password


def read_protocol_line(port: serial.Serial, deadline: float) -> str | None:
    while time.monotonic() < deadline:
        raw = port.readline()
        if not raw:
            continue
        line = raw.decode("utf-8", errors="ignore").strip()
        if line.startswith(PREFIX):
            return line
    return None


def identify(path: str) -> tuple[serial.Serial, str] | None:
    port = serial.Serial(path, 115200, timeout=0.15, write_timeout=1.0)
    nonce = secrets.token_hex(16)
    port.write(f"{PREFIX}INFO {nonce}\n".encode())
    port.flush()
    deadline = time.monotonic() + 1.5
    while (line := read_protocol_line(port, deadline)) is not None:
        match = IDENTITY_PATTERN.fullmatch(line)
        if match is not None and match.group(1) == nonce:
            return port, match.group(2)
    port.close()
    return None


def configure(port: serial.Serial, ssid: str, password: str) -> None:
    nonce = secrets.token_hex(16)
    encoded_password = base64url(password.encode()) if password else "-"
    request = (
        f"{PREFIX}SET {nonce} {base64url(ssid.encode())} "
        f"{encoded_password}\n"
    )
    port.write(request.encode())
    port.flush()
    acknowledged = False
    deadline = time.monotonic() + 45.0
    while (line := read_protocol_line(port, deadline)) is not None:
        if line == f"{PREFIX}ACK {nonce}":
            acknowledged = True
        elif line == f"{PREFIX}OK {nonce}":
            return
        elif line.startswith(f"{PREFIX}ERR {nonce} "):
            raise RuntimeError(line.rsplit(" ", 1)[-1])
    raise TimeoutError(
        "device did not report Wi-Fi result"
        if acknowledged
        else "device did not acknowledge Wi-Fi request"
    )


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Configure a cable-connected production BoBe over USB."
    )
    parser.add_argument(
        "--env",
        type=Path,
        default=ROOT / "private/dev-wifi.env",
    )
    args = parser.parse_args()
    ssid, password = load_env(args.env)
    candidates = [
        port.device
        for port in list_ports.comports()
        if port.vid == ESPRESSIF_USB_VID and port.pid == ESP32S3_USB_SERIAL_PID
    ]
    with exclusive_device_access("device"):
        matches: list[tuple[serial.Serial, str]] = []
        for path in candidates[:8]:
            try:
                match = identify(path)
            except (OSError, serial.SerialException):
                continue
            if match is not None:
                matches.append(match)
        if len(matches) != 1:
            for port, _identity in matches:
                port.close()
            raise RuntimeError(
                f"expected one production WB-12 over USB, found {len(matches)}"
            )
        port, identity = matches[0]
        try:
            configure(port, ssid, password)
        finally:
            port.close()
    print(f"BoBe {identity[-6:]} connected to the configured Wi-Fi network.")


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, TimeoutError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1) from error
