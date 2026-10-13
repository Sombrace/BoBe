#!/usr/bin/env python3

import argparse
import json
import os
import signal
import sys
import time
from datetime import UTC, datetime
from pathlib import Path

import serial
from serial.tools import list_ports


stop_requested = False


def request_stop(signum: int, frame: object) -> None:
    del signum, frame
    global stop_requested
    stop_requested = True


def timestamped(line: bytes) -> bytes:
    timestamp = datetime.now(UTC).isoformat(timespec="milliseconds")
    text = line.decode("utf-8", errors="backslashreplace").rstrip("\r\n")
    return f"{timestamp} {text}\n".encode()


def write_line(output, line: bytes) -> None:
    rendered = timestamped(line)
    output.write(rendered)
    sys.stdout.buffer.write(rendered)
    sys.stdout.buffer.flush()


def open_output(path: Path, append: bool):
    if path.is_symlink():
        raise ValueError("serial log output must not be a symbolic link")
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    flags = os.O_WRONLY | os.O_CREAT
    flags |= os.O_APPEND if append else os.O_EXCL
    descriptor = os.open(path, flags, 0o600)
    return os.fdopen(descriptor, "ab", buffering=0)


def port_identity(port: str) -> dict[str, object] | None:
    matches = [item for item in list_ports.comports() if item.device == port]
    if len(matches) != 1:
        return None
    match = matches[0]
    return {
        "vid": match.vid,
        "pid": match.pid,
        "serial_number": match.serial_number,
        "location": match.location,
    }


def require_same_identity(
    port: str,
    expected: dict[str, object] | None,
) -> None:
    if expected is None:
        return
    actual = port_identity(port)
    if actual is None:
        raise serial.SerialException("serial USB identity is unavailable")
    if actual != expected:
        raise ValueError("serial USB identity changed during capture")


def capture_connection(
    port: str,
    baud: int,
    output,
    deadline: float | None,
    identity: dict[str, object] | None,
) -> None:
    require_same_identity(port, identity)
    with serial.Serial(port, baud, timeout=0.2) as device:
        write_line(output, f"[serial connected: {port} at {baud} baud]".encode())
        pending = bytearray()
        try:
            while not stop_requested and (
                deadline is None or time.monotonic() < deadline
            ):
                chunk = device.read(device.in_waiting or 1)
                if not chunk:
                    continue
                pending.extend(chunk)
                while b"\n" in pending:
                    line, _, remainder = pending.partition(b"\n")
                    pending = bytearray(remainder)
                    write_line(output, bytes(line))
        finally:
            if pending:
                write_line(output, bytes(pending))


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Capture timestamped ESP serial output to a durable log."
    )
    parser.add_argument("port")
    parser.add_argument("output", type=Path)
    parser.add_argument("--baud", type=int, default=115200)
    parser.add_argument(
        "--duration",
        type=float,
        default=0,
        help="Seconds to capture; zero runs until interrupted.",
    )
    parser.add_argument("--append", action="store_true")
    parser.add_argument("--expected-serial-number")
    parser.add_argument(
        "--require-usb-identity",
        action="store_true",
        help="Reject ports without stable USB descriptor metadata.",
    )
    parser.add_argument(
        "--reconnect",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="Keep retrying the same port after USB resets (default: enabled).",
    )
    parser.add_argument(
        "--reconnect-delay",
        type=float,
        default=1.0,
        help="Seconds between reconnect attempts.",
    )
    args = parser.parse_args()

    if not args.port.startswith(("/dev/cu.", "/dev/tty.")):
        raise ValueError("serial port must be a /dev/cu.* or /dev/tty.* device")
    if args.baud <= 0 or args.duration < 0 or args.reconnect_delay <= 0:
        raise ValueError(
            "baud and reconnect delay must be positive; duration must not be "
            "negative"
        )

    signal.signal(signal.SIGINT, request_stop)
    signal.signal(signal.SIGTERM, request_stop)
    deadline = time.monotonic() + args.duration if args.duration else None
    identity = port_identity(args.port)
    if args.require_usb_identity and identity is None:
        raise ValueError("serial port has no unique USB identity")
    if args.expected_serial_number is not None:
        actual_serial = (
            identity.get("serial_number") if identity is not None else None
        )
        if actual_serial != args.expected_serial_number:
            raise ValueError("serial port does not match the expected identity")

    with open_output(args.output, args.append) as output:
        write_line(
            output,
            (
                "[serial identity: "
                + json.dumps(identity, sort_keys=True)
                + "]"
            ).encode(),
        )
        while not stop_requested and (
            deadline is None or time.monotonic() < deadline
        ):
            try:
                capture_connection(
                    args.port,
                    args.baud,
                    output,
                    deadline,
                    identity,
                )
            except (OSError, serial.SerialException) as error:
                write_line(
                    output,
                    f"[serial unavailable: {error}]".encode(
                        "utf-8",
                        errors="backslashreplace",
                    ),
                )
                if not args.reconnect:
                    raise
                remaining = (
                    args.reconnect_delay
                    if deadline is None
                    else min(
                        args.reconnect_delay,
                        max(0.0, deadline - time.monotonic()),
                    )
                )
                if remaining > 0:
                    time.sleep(remaining)


if __name__ == "__main__":
    main()
