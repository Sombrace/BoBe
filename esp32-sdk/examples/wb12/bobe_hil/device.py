import hashlib
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from contextlib import nullcontext
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

import serial
from serial.tools import list_ports

from bobe_hil import fixture as fixture_control
from bobe_hil.image import load_manifest, parse_otadata, sha256_file
from bobe_hil.paths import (
    APP_OFFSET,
    REPOSITORY_ROOT,
    RUNS_ROOT,
    exclusive_device_access,
    regular_file,
)


ESPRESSIF_USB_VID = 0x303A
ESP32S3_USB_SERIAL_PID = 0x1001
ESPTOOL_BAUD = "230400"
DEFAULT_OBSERVE_SECONDS = 18.0
MAX_LOG_BYTES = 256 * 1024
BOOT_REQUIRED = (
    "Starting BoBe WB-12 body",
    "BoBe body startup completed on CPU1",
    "health uptime=10s",
)
BOOT_REJECTED = (
    "Guru Meditation",
    "Task watchdog",
    "CORRUPT HEAP",
    "assert failed:",
)
BOOT_RESET_MARKERS = (
    "ESP-ROM:esp32s3-",
    "rst:0x",
)
DEVICE_ID_PATTERN = (
    r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-"
    r"[0-9a-f]{4}-[0-9a-f]{12}"
)
ESPTOOL_MAC_PATTERN = re.compile(
    r"^MAC:\s*((?:[0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2})\s*$",
    re.MULTILINE,
)
USB_SERIAL_MAC_PATTERN = re.compile(
    r"(?:[0-9a-f]{2}:){5}[0-9a-f]{2}|[0-9a-f]{12}",
    re.IGNORECASE,
)


def usb_serial_mac(serial_number: str | None) -> str | None:
    """Return the MAC in a USB serial number as aa:bb:cc:dd:ee:ff, or None."""

    if (
        serial_number is None
        or USB_SERIAL_MAC_PATTERN.fullmatch(serial_number) is None
    ):
        return None
    digits = serial_number.replace(":", "").lower()
    return ":".join(digits[index : index + 2] for index in range(0, 12, 2))


def connected_boards() -> list[dict[str, Any]]:
    boards: list[dict[str, Any]] = []
    for port in list_ports.comports():
        if port.vid != ESPRESSIF_USB_VID or port.pid != ESP32S3_USB_SERIAL_PID:
            continue
        mac = usb_serial_mac(port.serial_number)
        if mac is None:
            continue
        boards.append(
            {
                "device": port.device,
                "description": port.description,
                "vid": port.vid,
                "pid": port.pid,
                "serial_number": port.serial_number,
                "mac": mac,
            }
        )
    return boards


def matching_ports(identity: str) -> list[dict[str, Any]]:
    serial_identity = identity.replace(":", "").lower()
    return [
        board
        for board in connected_boards()
        if board["mac"].replace(":", "") == serial_identity
    ]


def board_count_blocker(count: int) -> str | None:
    if count == 0:
        return "no WB-12 is connected over USB"
    if count > 1:
        return f"{count} WB-12 boards are connected; pass --port"
    return None


def single_connected_board() -> dict[str, Any]:
    boards = connected_boards()
    blocker = board_count_blocker(len(boards))
    if blocker is not None:
        raise RuntimeError(blocker)
    return boards[0]


def inspect_device() -> dict[str, Any]:
    boards = connected_boards()
    blocker = board_count_blocker(len(boards))
    return {
        "available": blocker is None,
        "blocker": blocker,
        "boards": boards,
    }


def resolve_port(
    port: str | None = None,
    *,
    identity: str | None = None,
) -> tuple[str, str]:
    if identity:
        if port is not None:
            return identity, port
        ports = matching_ports(identity)
        if len(ports) != 1:
            raise RuntimeError(
                f"expected exactly one WB-12 port with MAC {identity}, "
                f"found {len(ports)}"
            )
        return identity, str(ports[0]["device"])
    if port is None:
        board = single_connected_board()
        return str(board["mac"]), str(board["device"])
    for board in connected_boards():
        if board["device"] == port:
            return str(board["mac"]), port
    raise RuntimeError(f"{port} is not a connected WB-12 USB Serial/JTAG port")


def identify(*, port: str | None = None) -> dict[str, Any]:
    if port is None:
        board = single_connected_board()
        return {
            "operation": "identify",
            "port": board,
            "mac": board["mac"],
            "identity_basis": "USB descriptor",
        }
    with exclusive_device_access("device"):
        result = run_esptool(
            port,
            ["--before", "usb-reset", "--after", "hard-reset", "read-mac"],
            timeout=30.0,
        )
    match = ESPTOOL_MAC_PATTERN.search(result.stdout)
    if result.returncode != 0 or match is None:
        raise RuntimeError(
            "connected ESP did not return a valid MAC address"
        )
    return {
        "operation": "identify",
        "port": port,
        "mac": match.group(1).lower(),
        "identity_basis": "ESP32-S3 ROM read-mac",
    }


def ordered_boot_markers(text: str, *, expected_elf: str) -> bool:
    if re.fullmatch(r"[0-9a-f]{64}", expected_elf) is None:
        return False
    marker = f"ELF file SHA256:  {expected_elf[:9]}"
    position = text.find(marker)
    if position < 0:
        return False
    offset = position + len(marker)

    start_marker = "Starting BoBe WB-12 body"
    start = text.find(start_marker, offset)
    if start < 0:
        return False
    identity = boot_identity_match(
        text,
        expected_elf=expected_elf,
        offset=start + len(start_marker),
    )
    if identity is None:
        return False
    offset = identity.end()
    for marker in (
        "BoBe body startup completed on CPU1",
        "health uptime=10s",
    ):
        position = text.find(marker, offset)
        if position < 0:
            return False
        offset = position + len(marker)
    return True


def boot_identity_match(
    text: str,
    *,
    expected_elf: str,
    offset: int = 0,
) -> re.Match[str] | None:
    pattern = re.compile(
        rf"device=(?P<device>{DEVICE_ID_PATTERN}) "
        rf"boot=(?P<boot>[0-9a-f]{{32}}) "
        rf"firmware=(?P<firmware>\S+) "
        rf"project=bobe_wb12"
        rf"(?: elf={re.escape(expected_elf)})?(?:\s|$)"
    )
    return pattern.search(text, offset)


def reboot_marker_after_start(text: str) -> bool:
    start_marker = "Starting BoBe WB-12 body"
    start = text.find(start_marker)
    if start < 0:
        return False
    after_start = text[start + len(start_marker) :]
    return start_marker in after_start or any(
        marker in after_start for marker in BOOT_RESET_MARKERS
    )


def retained_log(payload: bytes, prefix: str, root: Path | None = None) -> str:
    run_root = RUNS_ROOT if root is None else root
    run_root.mkdir(mode=0o700, parents=True, exist_ok=True)
    if run_root.is_symlink():
        raise RuntimeError("HIL run directory is a symbolic link")
    name = datetime.now(UTC).strftime(f"{prefix}-%Y%m%dT%H%M%S.%fZ.log")
    descriptor = os.open(
        run_root / name,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
        0o600,
    )
    with os.fdopen(descriptor, "wb") as output:
        output.write(payload)
    return name


class SerialMonitor:
    """Continuously buffer device output across resets and re-enumeration."""

    def __init__(
        self,
        identity: str,
        *,
        port: str | None = None,
        max_bytes: int = MAX_LOG_BYTES,
    ) -> None:
        self.identity = identity
        self.fixed_port = port
        self.max_bytes = max_bytes
        self.payload = bytearray()
        self.disconnects = 0
        self.connected = False
        self.overflowed = False
        self._lock = threading.Lock()
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None

    def current_port(self) -> str | None:
        if self.fixed_port is not None:
            return self.fixed_port
        ports = matching_ports(self.identity)
        return str(ports[0]["device"]) if len(ports) == 1 else None

    def _run(self) -> None:
        while not self._stop.is_set():
            port = self.current_port()
            if port is None:
                self._stop.wait(0.2)
                continue
            try:
                with serial.Serial(port, 115200, timeout=0.2) as device:
                    with self._lock:
                        if self.connected:
                            self.disconnects += 1
                        self.connected = True
                    while not self._stop.is_set():
                        chunk = device.read(device.in_waiting or 1)
                        if not chunk:
                            continue
                        with self._lock:
                            if len(self.payload) + len(chunk) > self.max_bytes:
                                self.overflowed = True
                                self._stop.set()
                                break
                            self.payload.extend(chunk)
            except (OSError, serial.SerialException):
                self._stop.wait(0.2)

    def start(self) -> "SerialMonitor":
        if self._thread is not None:
            raise RuntimeError("serial monitor is already running")
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()
        return self

    def snapshot(self) -> bytes:
        with self._lock:
            return bytes(self.payload)

    def text(self) -> str:
        return self.snapshot().decode("utf-8", errors="backslashreplace")

    def stop(self) -> bytes:
        self._stop.set()
        if self._thread is not None:
            self._thread.join(timeout=5.0)
            self._thread = None
        with self._lock:
            return bytes(self.payload)


def observe(
    *,
    seconds: float = DEFAULT_OBSERVE_SECONDS,
    expect: tuple[str, ...] = BOOT_REQUIRED,
    reject: tuple[str, ...] = BOOT_REJECTED,
    port: str | None = None,
    identity: str | None = None,
    monitor: SerialMonitor | None = None,
    retain: bool = True,
    log_prefix: str = "observe",
    lock_device: bool = True,
) -> dict[str, Any]:
    """Watch device output for caller-selected markers within a time bound."""

    if not 0 < seconds <= 3600:
        raise ValueError("observation duration must be within 0-3600 seconds")
    owned = monitor is None
    access = (
        exclusive_device_access("device")
        if owned and lock_device
        else nullcontext()
    )
    with access:
        if monitor is None:
            identity, port = resolve_port(port, identity=identity)
            monitor = SerialMonitor(identity, port=port).start()
        deadline = time.monotonic() + seconds
        found: dict[str, bool] = {}
        rejected: list[str] = []
        try:
            while True:
                text = monitor.text()
                found = {marker: marker in text for marker in expect}
                rejected = [marker for marker in reject if marker in text]
                if rejected or (found and all(found.values())):
                    break
                if time.monotonic() >= deadline:
                    break
                time.sleep(0.1)
            payload = monitor.snapshot()
        finally:
            if owned:
                monitor.stop()
    artifact = retained_log(payload, log_prefix) if retain else None
    return {
        "operation": "observe",
        "duration_seconds": seconds,
        "expected": found,
        "rejected": rejected,
        "passed": bool(found) and all(found.values()) and not rejected,
        "device_bytes": len(payload),
        "log_sha256": hashlib.sha256(payload).hexdigest(),
        "disconnects": monitor.disconnects,
        "overflowed": monitor.overflowed,
        "private_artifact": artifact,
    }


def run_esptool(
    port: str,
    arguments: list[str],
    *,
    timeout: float = 60.0,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
            sys.executable,
            "-m",
            "esptool",
            "--chip",
            "esp32s3",
            "--port",
            port,
            "--baud",
            ESPTOOL_BAUD,
            *arguments,
        ],
        check=False,
        capture_output=True,
        text=True,
        timeout=timeout,
    )


def assert_same_usb_identity(port: str, identity: str) -> None:
    matches = matching_ports(identity)
    if len(matches) != 1 or matches[0]["device"] != port:
        raise RuntimeError("WB-12 USB identity changed during the operation")
    read_mac = run_esptool(
        port,
        ["--before", "no-reset", "--after", "no-reset", "read-mac"],
        timeout=20.0,
    )
    if (
        read_mac.returncode != 0
        or (match := ESPTOOL_MAC_PATTERN.search(read_mac.stdout)) is None
        or match.group(1).lower() != identity
    ):
        raise RuntimeError("WB-12 MAC changed during the operation")


def enter_verified_loader(
    port: str,
    identity: str,
    working: Path,
    *,
    use_fixture: bool = True,
    fixture_root: Path | None = None,
) -> dict[str, Any]:
    """Enter the ROM loader and verify identity, security and OTA state."""

    before = "usb-reset"
    fixture_result: dict[str, Any] | None = None
    if use_fixture and fixture_control.inspect_fixture(fixture_root)["available"]:
        fixture_result = fixture_control.enter_loader(fixture_root)
        deadline = time.monotonic() + 10.0
        while time.monotonic() < deadline:
            ports = matching_ports(identity)
            if len(ports) == 1:
                port = str(ports[0]["device"])
                break
            time.sleep(0.2)
        else:
            raise RuntimeError("WB-12 did not enumerate after fixture loader entry")
        before = "no-reset"
    read_mac = run_esptool(
        port,
        ["--before", before, "--after", "no-reset", "read-mac"],
        timeout=30.0,
    )
    if (
        read_mac.returncode != 0
        or (match := ESPTOOL_MAC_PATTERN.search(read_mac.stdout)) is None
        or match.group(1).lower() != identity
    ):
        raise RuntimeError("ROM loader identity verification failed")
    security = run_esptool(
        port,
        ["--before", "no-reset", "--after", "no-reset", "get-security-info"],
    )
    warnings: list[str] = []
    if security.returncode != 0:
        warnings.append("device security state could not be read")
    else:
        if "Secure Boot: Disabled" not in security.stdout:
            warnings.append(
                "secure boot is enabled; a plain write can brick this device"
            )
        if "Flash Encryption: Disabled" not in security.stdout:
            warnings.append(
                "flash encryption is enabled; a plain write can brick this device"
            )
    otadata = working / "otadata.bin"
    read_ota = run_esptool(
        port,
        [
            "--before",
            "no-reset",
            "--after",
            "no-reset",
            "read-flash",
            "0xf000",
            "0x2000",
            str(otadata),
        ],
    )
    if read_ota.returncode != 0:
        raise RuntimeError("failed to read the OTA selection")
    ota_selection = parse_otadata(otadata.read_bytes())
    return {
        "port": port,
        "entered_with": before,
        "fixture": fixture_result,
        "ota": ota_selection,
        "security_info": security.stdout if security.returncode == 0 else None,
        "warnings": warnings,
    }


def write_image(
    port: str,
    identity: str,
    image: Path,
    working: Path,
    *,
    offset: int = APP_OFFSET,
    expected_sha256: str | None = None,
) -> dict[str, Any]:
    """Write, then read back and verify, one image at a caller-chosen offset."""

    regular_file(image, "flash image")
    if not 0 <= offset <= 0xFFFFFFFF or offset % 0x1000:
        raise ValueError("flash offset must be a 4 KiB aligned flash address")
    staged = working / "staged-image.bin"
    shutil.copyfile(image, staged)
    staged.chmod(0o400)
    digest = sha256_file(staged)
    if expected_sha256 is not None and digest != expected_sha256:
        raise RuntimeError("image digest does not match the manifest")
    assert_same_usb_identity(port, identity)
    write = run_esptool(
        port,
        [
            "--before",
            "no-reset",
            "--after",
            "no-reset",
            "write-flash",
            "--flash-mode",
            "keep",
            "--flash-freq",
            "keep",
            "--flash-size",
            "keep",
            hex(offset),
            str(staged),
        ],
        timeout=120.0,
    )
    if write.returncode != 0:
        raise RuntimeError("flash write failed")
    assert_same_usb_identity(port, identity)
    verify = run_esptool(
        port,
        [
            "--before",
            "no-reset",
            "--after",
            "no-reset",
            "verify-flash",
            "--flash-mode",
            "keep",
            "--flash-freq",
            "keep",
            "--flash-size",
            "keep",
            hex(offset),
            str(staged),
        ],
        timeout=90.0,
    )
    if verify.returncode != 0:
        raise RuntimeError("flash read-back verification failed")
    return {
        "image": str(image),
        "offset": hex(offset),
        "image_sha256": digest,
        "size": staged.stat().st_size,
    }


def reset_verified_application(port: str, identity: str) -> None:
    reset = run_esptool(
        port,
        ["--before", "usb-reset", "--after", "hard-reset", "read-mac"],
        timeout=30.0,
    )
    if (
        reset.returncode != 0
        or (match := ESPTOOL_MAC_PATTERN.search(reset.stdout)) is None
        or match.group(1).lower() != identity
    ):
        raise RuntimeError("identity-gated USB reset failed")


def observe_started_app(
    port: str,
    *,
    expected_elf: str | None,
    working: Path,
    seconds: float = DEFAULT_OBSERVE_SECONDS,
) -> dict[str, Any]:
    """Check that the written application booted."""

    deadline = time.monotonic() + seconds
    payload = bytearray()
    disconnected_after_start = False
    rebooted = False
    markers_observed = False
    terminal_failure = False
    connected = False
    while time.monotonic() < deadline:
        try:
            with serial.Serial(
                port,
                115200,
                timeout=0.2,
            ) as device:
                if connected:
                    disconnected_after_start = True
                    break
                connected = True
                while time.monotonic() < deadline:
                    chunk = device.read(device.in_waiting or 1)
                    if not chunk:
                        continue
                    payload.extend(chunk)
                    if len(payload) > MAX_LOG_BYTES:
                        raise RuntimeError("boot observation log exceeded bound")
                    text = payload.decode("utf-8", errors="backslashreplace")
                    if reboot_marker_after_start(text):
                        rebooted = True
                        terminal_failure = True
                        break
                    required_now = (
                        ordered_boot_markers(text, expected_elf=expected_elf)
                        if expected_elf is not None
                        else all(marker in text for marker in BOOT_REQUIRED)
                    )
                    if any(marker in text for marker in BOOT_REJECTED):
                        terminal_failure = True
                        break
                    if required_now:
                        markers_observed = True
                if terminal_failure:
                    break
        except (OSError, serial.SerialException):
            if connected:
                disconnected_after_start = True
                break
            time.sleep(0.2)
    log_path = working / "boot.log"
    log_path.write_bytes(payload)
    log_path.chmod(0o400)
    artifact = retained_log(bytes(payload), "flash-boot")
    text = payload.decode("utf-8", errors="backslashreplace")
    start = text.find("Starting BoBe WB-12 body")
    identity = (
        boot_identity_match(
            text,
            expected_elf=expected_elf,
            offset=max(start, 0),
        )
        if expected_elf is not None
        else None
    )
    required = {
        "startup": "BoBe body startup completed on CPU1" in text,
        "boot_start": "Starting BoBe WB-12 body" in text,
        "health": "health uptime=10s" in text,
    }
    if expected_elf is not None:
        required.update(
            {
                "identity": identity is not None,
                "elf": f"ELF file SHA256:  {expected_elf[:9]}" in text,
                "ordered": ordered_boot_markers(
                    text,
                    expected_elf=expected_elf,
                ),
            }
        )
    rejected = [marker for marker in BOOT_REJECTED if marker in text]
    return {
        "passed": (
            markers_observed
            and all(required.values())
            and not rejected
            and not rebooted
            and not disconnected_after_start
        ),
        "required": required,
        "rejected": rejected,
        "rebooted": rebooted,
        "disconnected_after_start": disconnected_after_start,
        "boot_id": identity.group("boot") if identity is not None else None,
        "device_bytes": len(payload),
        "log_sha256": hashlib.sha256(payload).hexdigest(),
        "private_artifact": artifact,
    }


def image_manifest(image: Path, manifest: Path | None) -> dict[str, Any] | None:
    path = manifest
    if path is None:
        default = Path(f"{image}.manifest.json")
        if not default.is_file() or default.is_symlink():
            return None
        path = default
    return dict(load_manifest(path, image, require_signature=False))


def ota_slot_for_offset(offset: int) -> int | None:
    for line in (REPOSITORY_ROOT / "partitions.csv").read_text(
        encoding="ascii"
    ).splitlines():
        if not line or line.lstrip().startswith("#"):
            continue
        fields = [field.strip() for field in line.split(",")]
        if len(fields) < 4 or not re.fullmatch(r"ota_[01]", fields[0]):
            continue
        if int(fields[3], 0) == offset:
            return int(fields[0].split("_", 1)[1])
    return None


def boot_observe(
    *,
    seconds: float = DEFAULT_OBSERVE_SECONDS,
    expect: tuple[str, ...] = BOOT_REQUIRED,
    reject: tuple[str, ...] = BOOT_REJECTED,
    port: str | None = None,
    identity: str | None = None,
) -> dict[str, Any]:
    """Reset the WB-12 through the ROM loader and observe its boot."""

    identity, port = resolve_port(port, identity=identity)
    with exclusive_device_access("device"):
        reset_verified_application(port, identity)
        result = observe(
            seconds=seconds,
            expect=expect,
            reject=reject,
            port=port,
            identity=identity,
            log_prefix="boot",
            lock_device=False,
        )
    return {**result, "operation": "boot-observe", "mac": identity}


def flash(
    image: Path,
    *,
    manifest: Path | None = None,
    offset: int = APP_OFFSET,
    rollback_image: Path | None = None,
    rollback_manifest: Path | None = None,
    observe_seconds: float = DEFAULT_OBSERVE_SECONDS,
    port: str | None = None,
    use_fixture: bool = True,
) -> dict[str, Any]:
    """Write, verify and boot-prove an image, optionally rolling back on failure."""

    identity, port = resolve_port(port)
    parsed = image_manifest(image, manifest)
    expected_elf = str(parsed["app_elf_sha256"]) if parsed else None
    RUNS_ROOT.mkdir(mode=0o700, parents=True, exist_ok=True)
    with exclusive_device_access("device"):
        with tempfile.TemporaryDirectory(dir=RUNS_ROOT) as directory:
            working = Path(directory)
            loader = enter_verified_loader(
                port,
                identity,
                working,
                use_fixture=use_fixture,
            )
            port = str(loader["port"])
            target_slot = ota_slot_for_offset(offset)
            selected_for_boot = bool(
                target_slot is not None
                and loader["ota"]["selected_slot"] == target_slot
                and loader["ota"]["selected_state"] == "valid"
            )
            expected_boot_elf = expected_elf if selected_for_boot else None
            original_error: Exception | None = None
            try:
                written = write_image(
                    port,
                    identity,
                    image,
                    working,
                    offset=offset,
                    expected_sha256=(
                        str(parsed["image_sha256"]) if parsed else None
                    ),
                )
                reset_verified_application(port, identity)
                observation = observe_started_app(
                    port,
                    expected_elf=expected_boot_elf,
                    working=working,
                    seconds=observe_seconds,
                )
                if selected_for_boot and not observation["passed"]:
                    raise RuntimeError("application boot observation failed")
                booted_written_image: bool | None = (
                    bool(observation["passed"])
                    if selected_for_boot and parsed is not None
                    else False
                    if target_slot is not None and not selected_for_boot
                    else None
                )
                return {
                    "operation": "flash",
                    "mac": identity,
                    "written": written,
                    "loader": loader,
                    "manifest": parsed,
                    "observation": observation,
                    "selected_for_boot": selected_for_boot,
                    "booted_written_image": booted_written_image,
                    "rolled_back": False,
                    "passed": True,
                }
            except (
                OSError,
                RuntimeError,
                subprocess.TimeoutExpired,
                ValueError,
            ) as error:
                original_error = error
            if rollback_image is None:
                raise RuntimeError(str(original_error)) from original_error
            try:
                restore_working = working / "rollback"
                restore_working.mkdir(mode=0o700)
                rollback_parsed = image_manifest(
                    rollback_image,
                    rollback_manifest,
                )
                loader = enter_verified_loader(
                    port,
                    identity,
                    restore_working,
                    use_fixture=use_fixture,
                )
                port = str(loader["port"])
                rollback_slot = ota_slot_for_offset(offset)
                rollback_selected = bool(
                    rollback_slot is not None
                    and loader["ota"]["selected_slot"] == rollback_slot
                    and loader["ota"]["selected_state"] == "valid"
                )
                write_image(
                    port,
                    identity,
                    rollback_image,
                    restore_working,
                    offset=offset,
                    expected_sha256=(
                        str(rollback_parsed["image_sha256"])
                        if rollback_parsed
                        else None
                    ),
                )
                reset_verified_application(port, identity)
                restored = observe_started_app(
                    port,
                    expected_elf=(
                        str(rollback_parsed["app_elf_sha256"])
                        if rollback_parsed and rollback_selected
                        else None
                    ),
                    working=restore_working,
                    seconds=observe_seconds,
                )
                if rollback_selected and not restored["passed"]:
                    raise RuntimeError("rollback boot observation failed")
            except (
                OSError,
                RuntimeError,
                subprocess.TimeoutExpired,
                ValueError,
            ) as restore_error:
                raise RuntimeError(
                    f"{original_error}; rollback failed: {restore_error}"
                ) from restore_error
            raise RuntimeError(
                f"{original_error}; the rollback image was written, verified, "
                "and observed"
            ) from original_error
