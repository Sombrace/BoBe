import json
import re
import secrets
import time
from contextlib import contextmanager
from collections.abc import Iterator
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import serial
from serial.tools import list_ports

from bobe_hil.fixture_protocol import (
    CHORDS,
    CONTROLS,
    FIRMWARE_VERSION,
    MAX_PRESS_MS,
    MAX_RESET_MS,
    MIN_PRESS_MS,
    MIN_RESET_MS,
    FixtureClient,
    FixtureIdentity,
)
from bobe_hil.paths import exclusive_device_access, private_root, regular_file


PICO_USB_VID = 0x2E8A
PICO_RP2350_STDIO_USB_PID = 0x0009
FIXTURE_CONFIG_SCHEMA = 1
MAX_FIXTURE_LINE_BYTES = 512
DEFAULT_PRESS_MS = 250
DEFAULT_CHORD_MS = 2000
DEFAULT_RESET_MS = 120
CONTACTS = tuple(sorted(CONTROLS))
CHORD_NAMES = tuple(sorted(CHORDS))


@dataclass(frozen=True)
class FixtureConfig:
    board_id: str
    firmware_version: str
    path: Path


def config_path(root: Path | None = None) -> Path:
    return private_root(root) / "fixture.json"


def load_fixture_config(
    root: Path | None = None,
    *,
    path: Path | None = None,
) -> FixtureConfig:
    location = config_path(root) if path is None else Path(path)
    regular_file(location, "fixture configuration")
    try:
        value = json.loads(location.read_text(encoding="utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError("fixture configuration is invalid JSON") from error
    if not isinstance(value, dict):
        raise ValueError("fixture configuration must be a JSON object")
    board_id = value.get("board_id")
    firmware_version = value.get("firmware_version", FIRMWARE_VERSION)
    if (
        value.get("schema_version") != FIXTURE_CONFIG_SCHEMA
        or not isinstance(board_id, str)
        or re.fullmatch(r"[0-9a-f]{16}", board_id) is None
        or not isinstance(firmware_version, str)
    ):
        raise ValueError("fixture configuration values are invalid")
    return FixtureConfig(board_id, firmware_version, location)


def matching_fixture_ports(board_id: str) -> list[dict[str, Any]]:
    matches = []
    for port in list_ports.comports():
        serial_number = (port.serial_number or "").lower()
        if (
            port.vid == PICO_USB_VID
            and port.pid == PICO_RP2350_STDIO_USB_PID
            and serial_number == board_id
        ):
            matches.append(
                {
                    "device": port.device,
                    "description": port.description,
                    "vid": port.vid,
                    "pid": port.pid,
                    "serial_number": port.serial_number,
                }
            )
    return matches


def inspect_fixture(root: Path | None = None) -> dict[str, Any]:
    try:
        config = load_fixture_config(root)
    except (OSError, ValueError) as error:
        return {
            "configured": False,
            "identified": False,
            "available": False,
            "blocker": str(error),
            "matching_ports": [],
        }
    ports = matching_fixture_ports(config.board_id)
    identified = len(ports) == 1
    return {
        "configured": True,
        "identified": identified,
        "available": identified,
        "blocker": (
            None
            if identified
            else f"expected exactly one configured Pico fixture, found {len(ports)}"
        ),
        "board_id": config.board_id,
        "firmware_version": config.firmware_version,
        "config_path": str(config.path),
        "matching_ports": ports,
    }


class SerialFixtureTransport:
    def __init__(self, port: str) -> None:
        self.device = serial.Serial(
            port,
            115200,
            timeout=0.2,
            write_timeout=1.0,
        )
        self.device.reset_input_buffer()

    def __enter__(self) -> "SerialFixtureTransport":
        return self

    def __exit__(self, *_args: object) -> None:
        self.device.close()

    def exchange(self, line: str, timeout_seconds: float) -> str:
        encoded = (line + "\n").encode("ascii")
        written = self.device.write(encoded)
        if written != len(encoded):
            raise RuntimeError("fixture request write was incomplete")
        self.device.flush()
        deadline = time.monotonic() + timeout_seconds
        pending = bytearray()
        while time.monotonic() < deadline:
            chunk = self.device.read(self.device.in_waiting or 1)
            if not chunk:
                continue
            pending.extend(chunk)
            if len(pending) > MAX_FIXTURE_LINE_BYTES:
                raise RuntimeError("fixture response frame is invalid")
            if b"\n" not in pending:
                continue
            response, _, remainder = pending.partition(b"\n")
            if remainder.strip(b"\r\n"):
                raise RuntimeError("fixture emitted unsolicited response data")
            try:
                return response.rstrip(b"\r").decode("ascii")
            except UnicodeDecodeError as error:
                raise RuntimeError("fixture response is not ASCII") from error
        raise TimeoutError("fixture response timed out")


@contextmanager
def fixture_session(
    root: Path | None = None,
) -> Iterator[tuple[FixtureClient, FixtureIdentity, dict[str, Any]]]:
    config = load_fixture_config(root)
    ports = matching_fixture_ports(config.board_id)
    if len(ports) != 1:
        raise RuntimeError(
            f"expected exactly one configured Pico fixture, found {len(ports)}"
        )
    with exclusive_device_access("fixture", root=root):
        with SerialFixtureTransport(str(ports[0]["device"])) as transport:
            client = FixtureClient(
                transport,
                secrets.token_hex(16),
                config.board_id,
            )
            identity = client.hello()
            yield client, identity, ports[0]


def result(
    operation: str,
    identity: FixtureIdentity,
    port: dict[str, Any],
    response: str,
    **fields: Any,
) -> dict[str, Any]:
    return {
        "operation": operation,
        "fixture": {
            "board_id": identity.board_id,
            "firmware_version": identity.firmware_version,
            "protocol_version": identity.protocol_version,
            "port": port,
        },
        "response": response,
        **fields,
    }


def checked_duration(value: int, minimum: int, maximum: int, description: str) -> int:
    if type(value) is not int or not minimum <= value <= maximum:
        raise ValueError(
            f"{description} must be an integer between {minimum} and {maximum} ms"
        )
    return value


def status(root: Path | None = None) -> dict[str, Any]:
    with fixture_session(root) as (client, identity, port):
        return result("fixture-status", identity, port, client.status())


def press(
    contact: str,
    duration_ms: int = DEFAULT_PRESS_MS,
    *,
    root: Path | None = None,
) -> dict[str, Any]:
    if contact not in CONTROLS:
        raise ValueError(
            f"fixture contact must be one of: {', '.join(CONTACTS)}"
        )
    checked_duration(duration_ms, MIN_PRESS_MS, MAX_PRESS_MS, "press duration")
    with fixture_session(root) as (client, identity, port):
        response = client.press(contact, duration_ms)
        return result(
            "fixture-press",
            identity,
            port,
            response,
            contact=contact,
            duration_ms=duration_ms,
        )


def chord(
    name: str,
    duration_ms: int = DEFAULT_CHORD_MS,
    *,
    root: Path | None = None,
) -> dict[str, Any]:
    if name not in CHORDS:
        raise ValueError(
            f"fixture chord must be one of: {', '.join(CHORD_NAMES)}"
        )
    checked_duration(duration_ms, MIN_PRESS_MS, MAX_PRESS_MS, "chord duration")
    with fixture_session(root) as (client, identity, port):
        response = client.chord(name, duration_ms)
        return result(
            "fixture-chord",
            identity,
            port,
            response,
            chord=name,
            duration_ms=duration_ms,
        )


def reset(
    duration_ms: int = DEFAULT_RESET_MS,
    *,
    root: Path | None = None,
) -> dict[str, Any]:
    checked_duration(duration_ms, MIN_RESET_MS, MAX_RESET_MS, "reset pulse")
    with fixture_session(root) as (client, identity, port):
        response = client.reset(duration_ms)
        return result(
            "fixture-reset",
            identity,
            port,
            response,
            duration_ms=duration_ms,
        )


def enter_loader(root: Path | None = None) -> dict[str, Any]:
    with fixture_session(root) as (client, identity, port):
        return result(
            "fixture-enter-loader",
            identity,
            port,
            client.enter_loader(),
        )


def release_all(root: Path | None = None) -> dict[str, Any]:
    with fixture_session(root) as (client, identity, port):
        return result(
            "fixture-release-all",
            identity,
            port,
            client.release_all(),
            released=True,
        )
