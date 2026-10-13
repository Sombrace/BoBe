#!/usr/bin/env python3

import argparse
import configparser
import hashlib
import json
import os
import plistlib
import re
import resource
import stat
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parent.parent
RUNTIME = ROOT / ".deps" / "sigrok-cli-arm64"
LOCK = ROOT / "tools" / "sigrok-cli.lock"
BINARY = RUNTIME / "bin" / "sigrok-cli"
USB_IDENTITY = RUNTIME / "bin" / "sigrok-usb-identity"
FIRMWARE_ROOT = RUNTIME / "share" / "sigrok-firmware"
MANIFEST = RUNTIME / "MANIFEST.sha256"
WRAPPER_VERSION = "1"
MAX_CAPTURE_SAMPLES = 24_000_000
MAX_CAPTURE_SECONDS = 60.0
MAX_CAPTURE_BYTES = 64 * 1024 * 1024
MAX_CONVERT_BYTES = 256 * 1024 * 1024
MAX_SHOW_BYTES = 256 * 1024
SAMPLERATES = {
    "20k": 20_000,
    "25k": 25_000,
    "50k": 50_000,
    "100k": 100_000,
    "200k": 200_000,
    "250k": 250_000,
    "500k": 500_000,
    "1m": 1_000_000,
    "2m": 2_000_000,
    "3m": 3_000_000,
    "4m": 4_000_000,
    "6m": 6_000_000,
    "8m": 8_000_000,
    "12m": 12_000_000,
    "16m": 16_000_000,
    "24m": 24_000_000,
}
KNOWN_PROFILES = {
    (0x0925, 0x3881): "saleae-logic",
    (0x08A9, 0x0005): "cwav-usbeezx",
    (0x08A9, 0x0009): "cwav-usbeesx",
    (0x08A9, 0x0014): "cwav-usbeeax",
    (0x04B4, 0x8613): "cypress-fx2",
}
SPEED_NAMES = {
    0: "unknown",
    1: "low-speed",
    2: "full-speed",
    3: "high-speed",
    4: "super-speed",
    5: "super-speed-plus",
}
CHANNEL_PATTERN = re.compile(
    r"^D[0-7](?:=[A-Za-z][A-Za-z0-9_-]{0,31})?"
    r"(?:,D[0-7](?:=[A-Za-z][A-Za-z0-9_-]{0,31})?)*$"
)
TRIGGER_PATTERN = re.compile(
    r"^D[0-7]=[01rfe](?:,D[0-7]=[01rfe])*$"
)
HEX16_PATTERN = re.compile(r"^[0-9a-fA-F]{4}$")
HEX32_PATTERN = re.compile(r"^[0-9a-fA-F]{8}$")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def parse_hex(value: str, bits: int, description: str) -> int:
    pattern = HEX16_PATTERN if bits == 16 else HEX32_PATTERN
    normalized = value.removeprefix("0x")
    if pattern.fullmatch(normalized) is None:
        raise ValueError(f"{description} must be {bits // 4} hexadecimal digits")
    return int(normalized, 16)


def ioreg_devices() -> list[dict[str, Any]]:
    result = subprocess.run(
        ["/usr/sbin/ioreg", "-a", "-p", "IOUSB", "-c", "IOUSBHostDevice"],
        check=True,
        capture_output=True,
        timeout=15.0,
    )
    try:
        root = plistlib.loads(result.stdout)
    except plistlib.InvalidFileException as error:
        raise RuntimeError("IOKit USB inventory is invalid") from error
    devices: list[dict[str, Any]] = []

    def walk(value: Any) -> None:
        if isinstance(value, dict):
            vid = value.get("idVendor")
            pid = value.get("idProduct")
            location = value.get("locationID")
            if (
                type(vid) is int
                and type(pid) is int
                and type(location) is int
            ):
                serial = value.get("USB Serial Number")
                product = value.get("USB Product Name")
                manufacturer = value.get("USB Vendor Name")
                bcd = value.get("bcdDevice")
                speed = value.get("USBSpeed")
                address = value.get("USB Address")
                devices.append(
                    {
                        "vid": f"{vid:04x}",
                        "pid": f"{pid:04x}",
                        "bcd_device": (
                            f"{bcd:04x}" if type(bcd) is int else None
                        ),
                        "location_id": f"{location:08x}",
                        "usb_address": (
                            address if type(address) is int else None
                        ),
                        "speed_code": speed if type(speed) is int else None,
                        "speed": SPEED_NAMES.get(
                            speed if type(speed) is int else 0,
                            "unknown",
                        ),
                        "manufacturer": (
                            manufacturer
                            if isinstance(manufacturer, str)
                            else None
                        ),
                        "product": product if isinstance(product, str) else None,
                        "serial_number": (
                            serial if isinstance(serial, str) else None
                        ),
                        "profile": KNOWN_PROFILES.get((vid, pid)),
                        "firmware_state": (
                            "fx2lafw"
                            if manufacturer == "sigrok"
                            and product == "fx2lafw"
                            else "original"
                        ),
                    }
                )
            for child in value.values():
                walk(child)
        elif isinstance(value, list):
            for child in value:
                walk(child)

    walk(root)
    return devices


def discover(vid: int | None, pid: int | None) -> list[dict[str, Any]]:
    devices = ioreg_devices()
    if vid is not None and pid is not None:
        return [
            device
            for device in devices
            if int(device["vid"], 16) == vid and int(device["pid"], 16) == pid
        ]
    return [
        device
        for device in devices
        if (int(device["vid"], 16), int(device["pid"], 16))
        in KNOWN_PROFILES
    ]


def libusb_devices(vid: int, pid: int) -> list[dict[str, Any]]:
    if USB_IDENTITY.is_symlink() or not os.access(USB_IDENTITY, os.X_OK):
        raise RuntimeError("sigrok USB identity helper is unavailable or unsafe")
    result = subprocess.run(
        [str(USB_IDENTITY), f"{vid:04x}", f"{pid:04x}"],
        check=True,
        capture_output=True,
        text=True,
        env=sigrok_environment(),
        timeout=15.0,
    )
    if len(result.stdout.encode("utf-8")) > 64 * 1024:
        raise RuntimeError("sigrok USB identity output exceeds bounds")
    try:
        values = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError("sigrok USB identity output is invalid") from error
    if not isinstance(values, list) or len(values) > 16:
        raise RuntimeError("sigrok USB identity result is invalid")
    for value in values:
        if (
            not isinstance(value, dict)
            or set(value) != {
                "address",
                "bcd_device",
                "bus",
                "pid",
                "ports",
                "speed",
                "vid",
            }
            or value["vid"] != f"{vid:04x}"
            or value["pid"] != f"{pid:04x}"
            or type(value["bus"]) is not int
            or not 0 <= value["bus"] <= 255
            or type(value["address"]) is not int
            or not 1 <= value["address"] <= 255
            or type(value["speed"]) is not int
            or not isinstance(value["ports"], list)
            or len(value["ports"]) > 8
            or any(type(port) is not int or not 1 <= port <= 255 for port in value["ports"])
            or not isinstance(value["bcd_device"], str)
            or HEX16_PATTERN.fullmatch(value["bcd_device"]) is None
        ):
            raise RuntimeError("sigrok USB identity values are invalid")
    return values


def parse_manifest() -> dict[str, str]:
    if (
        RUNTIME.parent.is_symlink()
        or RUNTIME.is_symlink()
        or not RUNTIME.is_dir()
        or MANIFEST.is_symlink()
        or not MANIFEST.is_file()
    ):
        raise RuntimeError("sigrok runtime manifest is unavailable or unsafe")
    if any(path.is_symlink() for path in RUNTIME.rglob("*")):
        raise RuntimeError("sigrok runtime contains a symbolic link")
    expected: dict[str, str] = {}
    for line in MANIFEST.read_text(encoding="ascii").splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  (\./.+)", line)
        if match is None or match.group(2) in expected:
            raise RuntimeError("sigrok runtime manifest is malformed")
        expected[match.group(2)] = match.group(1)
    actual = {
        f"./{path.relative_to(RUNTIME)}"
        for path in RUNTIME.rglob("*")
        if path.is_file() and path != MANIFEST
    }
    if actual != set(expected):
        raise RuntimeError("sigrok runtime file set does not match manifest")
    for relative, digest in expected.items():
        path = RUNTIME / relative.removeprefix("./")
        if path.is_symlink() or sha256_file(path) != digest:
            raise RuntimeError("sigrok runtime file hash does not match manifest")
    return expected


def lock_values(path: Path) -> dict[str, str]:
    if path.is_symlink() or not path.is_file():
        raise RuntimeError("sigrok lock is unavailable or unsafe")
    values: dict[str, str] = {}
    for line in path.read_text(encoding="ascii").splitlines():
        key, separator, value = line.partition("=")
        if (
            separator != "="
            or re.fullmatch(r"[A-Z][A-Z0-9_]*", key) is None
            or not value
            or key in values
        ):
            raise RuntimeError("sigrok lock is malformed")
        values[key] = value
    return values


def sigrok_environment() -> dict[str, str]:
    return {
        "HOME": os.environ.get("HOME", str(Path.home())),
        "LANG": "C",
        "LC_ALL": "C",
        "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
        "SIGROK_FIRMWARE_DIR": str(FIRMWARE_ROOT),
    }


def runtime_status() -> dict[str, Any]:
    current_lock = lock_values(LOCK)
    expected = parse_manifest()
    installed_lock = RUNTIME / "sigrok-cli.lock"
    if (
        installed_lock.is_symlink()
        or not installed_lock.is_file()
        or installed_lock.read_bytes() != LOCK.read_bytes()
        or sha256_file(ROOT / "tools" / "bootstrap_sigrok_cli.sh")
        != current_lock.get("BOOTSTRAP_SHA256")
        or sha256_file(ROOT / "tools" / "sigrok_usb_identity.c")
        != current_lock.get("USB_IDENTITY_SOURCE_SHA256")
    ):
        raise RuntimeError("sigrok runtime does not match current build inputs")
    if (
        BINARY.is_symlink()
        or not os.access(BINARY, os.X_OK)
        or USB_IDENTITY.is_symlink()
        or not os.access(USB_IDENTITY, os.X_OK)
    ):
        raise RuntimeError("sigrok runtime binary is unavailable or unsafe")
    architecture = subprocess.run(
        ["/usr/bin/lipo", "-archs", str(BINARY)],
        check=True,
        capture_output=True,
        text=True,
        timeout=10.0,
    ).stdout.strip()
    if architecture != "arm64":
        raise RuntimeError("sigrok runtime is not native arm64")
    version = subprocess.run(
        [str(BINARY), "--version"],
        check=True,
        capture_output=True,
        text=True,
        env=sigrok_environment(),
        timeout=10.0,
    ).stdout
    if not version.startswith("sigrok-cli 0.7.2\n"):
        raise RuntimeError("sigrok runtime version is invalid")
    firmware = FIRMWARE_ROOT / "fx2lafw-saleae-logic.fw"
    return {
        "schema_version": 1,
        "wrapper_version": WRAPPER_VERSION,
        "ready": True,
        "architecture": architecture,
        "identity_helper": str(USB_IDENTITY),
        "sigrok_version": version.splitlines()[0],
        "runtime": str(RUNTIME),
        "manifest_files": len(expected),
        "firmware": {
            "name": firmware.name,
            "size": firmware.stat().st_size,
            "sha256": sha256_file(firmware),
        },
        "protocol_decoders": False,
    }


def require_device(
    vid: int,
    pid: int,
    location: int,
) -> dict[str, Any]:
    matches = discover(vid, pid)
    exact = [
        device
        for device in matches
        if int(device["location_id"], 16) == location
    ]
    if len(matches) != 1 or len(exact) != 1:
        raise RuntimeError(
            "expected exactly one USB analyzer at the requested location"
        )
    device = exact[0]
    if device["speed"] != "high-speed":
        raise RuntimeError("requested analyzer is not connected at USB high speed")
    usb_matches = libusb_devices(vid, pid)
    correlated = [
        value
        for value in usb_matches
        if value["address"] == device["usb_address"]
        and value["bcd_device"] == device["bcd_device"]
    ]
    if len(usb_matches) != 1 or len(correlated) != 1:
        raise RuntimeError(
            "IOKit and libusb do not identify the same unique analyzer"
        )
    usb = correlated[0]
    return {
        **device,
        "libusb_bus": usb["bus"],
        "libusb_address": usb["address"],
        "libusb_ports": usb["ports"],
        "connection": f"{usb['bus']}.{usb['address']}",
    }


def limit_output(maximum_file: int):
    def apply() -> None:
        resource.setrlimit(
            resource.RLIMIT_FSIZE,
            (maximum_file, maximum_file),
        )

    return apply


def run_sigrok(
    arguments: list[str],
    *,
    timeout: float,
    output_limit: int | None = None,
) -> subprocess.CompletedProcess[str]:
    file_limit = max(output_limit or 0, MAX_SHOW_BYTES)
    with tempfile.TemporaryDirectory(prefix="sigrok-process-") as directory:
        stdout_path = Path(directory) / "stdout"
        stderr_path = Path(directory) / "stderr"
        with stdout_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
            completed = subprocess.run(
                [str(BINARY), *arguments],
                check=False,
                stdout=stdout,
                stderr=stderr,
                env=sigrok_environment(),
                timeout=timeout,
                preexec_fn=limit_output(file_limit),
            )
        if (
            stdout_path.stat().st_size > MAX_SHOW_BYTES
            or stderr_path.stat().st_size > MAX_SHOW_BYTES
        ):
            raise RuntimeError("sigrok process output exceeds bounds")
        stdout_text = stdout_path.read_text(
            encoding="utf-8",
            errors="backslashreplace",
        )
        stderr_text = stderr_path.read_text(
            encoding="utf-8",
            errors="backslashreplace",
        )
    return subprocess.CompletedProcess(
        completed.args,
        completed.returncode,
        stdout_text,
        stderr_text,
    )


def parse_samplerate(value: str) -> int:
    match = re.fullmatch(
        r"\s*([0-9]+(?:\.[0-9]+)?)\s*([kMGT]?Hz)?\s*",
        value,
    )
    if match is None:
        raise ValueError("sigrok session samplerate is invalid")
    multipliers = {
        None: 1,
        "Hz": 1,
        "kHz": 1_000,
        "MHz": 1_000_000,
        "GHz": 1_000_000_000,
        "THz": 1_000_000_000_000,
    }
    numeric = float(match.group(1)) * multipliers[match.group(2)]
    if not numeric.is_integer() or not 1 <= numeric <= 1_000_000_000_000:
        raise ValueError("sigrok session samplerate is outside bounds")
    return int(numeric)


class CaseSensitiveConfigParser(configparser.ConfigParser):
    def optionxform(self, optionstr: str) -> str:
        return optionstr


def validate_metadata(
    metadata_bytes: bytes,
) -> tuple[int, int, str, dict[str, str]]:
    if len(metadata_bytes) > 1024 * 1024:
        raise ValueError("sigrok session metadata exceeds bounds")
    metadata = CaseSensitiveConfigParser()
    metadata.read_string(metadata_bytes.decode("utf-8"))
    if set(metadata.sections()) != {"global", "device 1"}:
        raise ValueError("sigrok session metadata sections are invalid")
    if set(metadata["global"]) != {"sigrok version"}:
        raise ValueError("sigrok session global metadata is invalid")
    device = metadata["device 1"]
    required = {
        "capturefile",
        "samplerate",
        "total analog",
        "total probes",
        "unitsize",
    }
    keys = set(device)
    if not required.issubset(keys):
        raise ValueError("sigrok session device metadata is incomplete")
    total_probes = int(device["total probes"])
    total_analog = int(device["total analog"])
    unit_size = int(device["unitsize"])
    if (
        not 1 <= total_probes <= 16
        or total_analog != 0
        or unit_size not in {1, 2}
        or device["capturefile"] != "logic-1"
    ):
        raise ValueError("sigrok session metadata values are invalid")
    probe_keys = {f"probe{index}" for index in range(1, total_probes + 1)}
    if not keys.issubset(required | probe_keys):
        raise ValueError("sigrok session device metadata keys are invalid")
    if not any(key in keys for key in probe_keys):
        raise ValueError("sigrok session has no enabled probes")
    enabled_keys = sorted(
        keys & probe_keys,
        key=lambda key: int(key.removeprefix("probe")),
    )
    for key in enabled_keys:
        label = device[key]
        if (
            not 1 <= len(label) <= 32
            or not label.isascii()
            or not all(character.isprintable() for character in label)
        ):
            raise ValueError("sigrok session probe label is invalid")
    probe_map = {
        f"D{int(key.removeprefix('probe')) - 1}": device[key]
        for key in enabled_keys
    }
    if any(
        int(channel.removeprefix("D")) >= unit_size * 8
        for channel in probe_map
    ):
        raise ValueError("sigrok session probe index exceeds its unit size")
    return (
        unit_size,
        parse_samplerate(device["samplerate"]),
        "logic-1",
        probe_map,
    )


def ordered_logic_members(
    archive: zipfile.ZipFile,
    capturefile: str,
) -> list[zipfile.ZipInfo]:
    members = [
        info
        for info in archive.infolist()
        if info.filename == capturefile
        or re.fullmatch(
            rf"{re.escape(capturefile)}-[1-9][0-9]*",
            info.filename,
        )
    ]
    unchunked = [info for info in members if info.filename == capturefile]
    chunked = [
        (
            int(info.filename.removeprefix(f"{capturefile}-")),
            info,
        )
        for info in members
        if info.filename != capturefile
    ]
    if unchunked:
        if len(unchunked) != 1 or chunked:
            raise ValueError("sigrok session logic members are ambiguous")
        return unchunked
    chunked.sort(key=lambda item: item[0])
    if (
        not chunked
        or [number for number, _ in chunked]
        != list(range(1, len(chunked) + 1))
    ):
        raise ValueError("sigrok session logic chunks are not contiguous")
    return [info for _, info in chunked]


def validate_session(path: Path) -> dict[str, Any]:
    if path.is_symlink() or not path.is_file():
        raise ValueError("sigrok session is unavailable or unsafe")
    size = path.stat().st_size
    if not 0 < size <= MAX_CAPTURE_BYTES:
        raise ValueError("sigrok session size is outside bounds")
    try:
        with zipfile.ZipFile(path) as archive:
            names = archive.namelist()
            if (
                len(names) > 1024
                or len(names) != len(set(names))
                or "version" not in names
                or "metadata" not in names
                or any(
                    not name
                    or name.startswith(("/", "\\"))
                    or ".." in Path(name).parts
                    or name.endswith("/")
                    for name in names
                )
                or any(
                    stat.S_IFMT(info.external_attr >> 16) == stat.S_IFLNK
                    for info in archive.infolist()
                )
            ):
                raise ValueError("sigrok session member set is invalid")
            total = sum(info.file_size for info in archive.infolist())
            if total > MAX_CAPTURE_BYTES:
                raise ValueError("sigrok session expands beyond bounds")
            version = archive.read("version")
            if version.strip() != b"2":
                raise ValueError("sigrok session version is unsupported")
            unit_size, samplerate, capturefile, probe_map = validate_metadata(
                archive.read("metadata")
            )
            logic_members = ordered_logic_members(archive, capturefile)
            if set(names) != {
                "version",
                "metadata",
                *(info.filename for info in logic_members),
            }:
                raise ValueError("sigrok session contains unexpected members")
            streamed_bytes = 0
            for info in logic_members:
                member_bytes = 0
                with archive.open(info) as source:
                    while chunk := source.read(1024 * 1024):
                        member_bytes += len(chunk)
                        streamed_bytes += len(chunk)
                        if streamed_bytes > MAX_CAPTURE_BYTES:
                            raise ValueError(
                                "sigrok session logic expands beyond bounds"
                            )
                if member_bytes != info.file_size:
                    raise ValueError("sigrok session logic member is truncated")
                if info.file_size % unit_size:
                    raise ValueError(
                        "sigrok session logic member is misaligned"
                    )
            sample_bytes = sum(info.file_size for info in logic_members)
            if sample_bytes % unit_size:
                raise ValueError("sigrok session sample bytes are misaligned")
            sample_count = sample_bytes // unit_size
    except (
        configparser.Error,
        KeyError,
        UnicodeDecodeError,
        ValueError,
        zipfile.BadZipFile,
    ) as error:
        if isinstance(error, ValueError):
            raise
        raise ValueError("sigrok session is malformed") from error
    show = run_sigrok(
        ["--input-file", str(path), "--show"],
        timeout=15.0,
    )
    if show.returncode != 0:
        raise RuntimeError("sigrok could not reopen the retained session")
    rendered = show.stdout + show.stderr
    if len(rendered.encode("utf-8")) > MAX_SHOW_BYTES:
        raise RuntimeError("sigrok session inspection output exceeds bounds")
    return {
        "size": size,
        "sha256": sha256_file(path),
        "samplerate": samplerate,
        "unit_size": unit_size,
        "sample_count": sample_count,
        "probe_map": probe_map,
        "members": len(names),
        "show": rendered.strip(),
    }


def publish_no_clobber(source: Path, output: Path) -> None:
    if output.is_symlink() or output.exists():
        raise ValueError("output must be a new non-symlink path")
    output.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    os.link(source, output)
    descriptor = os.open(output.parent, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def copy_regular_file(
    source: Path,
    destination: Path,
    maximum: int,
) -> None:
    try:
        source_descriptor = os.open(
            source,
            os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
        )
    except OSError as error:
        raise ValueError("input file is unavailable or unsafe") from error
    try:
        metadata = os.fstat(source_descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_size <= 0
            or metadata.st_size > maximum
        ):
            raise ValueError("input file size or type is invalid")
        destination_descriptor = os.open(
            destination,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
            0o600,
        )
        try:
            remaining = metadata.st_size
            while remaining:
                chunk = os.read(
                    source_descriptor,
                    min(1024 * 1024, remaining),
                )
                if not chunk:
                    raise ValueError("input file ended before declared size")
                offset = 0
                while offset < len(chunk):
                    written = os.write(
                        destination_descriptor,
                        chunk[offset:],
                    )
                    if written <= 0:
                        raise OSError("input copy write was incomplete")
                    offset += written
                remaining -= len(chunk)
            if os.fstat(source_descriptor).st_size != metadata.st_size:
                raise ValueError("input file changed while copying")
            os.fsync(destination_descriptor)
        finally:
            os.close(destination_descriptor)
    finally:
        os.close(source_descriptor)


def normalize_channels(value: str) -> tuple[str, dict[str, str]]:
    if CHANNEL_PATTERN.fullmatch(value) is None:
        raise ValueError("channel selection is invalid")
    channels: list[str] = []
    aliases: dict[str, str] = {}
    used_names: set[str] = set()
    for item in value.split(","):
        channel, separator, label = item.partition("=")
        if channel in aliases:
            raise ValueError("channel selection contains a duplicate channel")
        effective = label if separator else channel
        if effective in used_names:
            raise ValueError("channel selection contains a duplicate label")
        used_names.add(effective)
        aliases[channel] = effective
        channels.append(item)
    physical_channels = {f"D{index}" for index in range(8)}
    if any(
        effective in physical_channels and effective != channel
        for channel, effective in aliases.items()
    ):
        raise ValueError("channel label shadows another physical channel")
    return ",".join(channels), aliases


def normalize_trigger(
    value: str | None,
    aliases: dict[str, str],
) -> str | None:
    if value is None:
        return None
    if TRIGGER_PATTERN.fullmatch(value) is None:
        raise ValueError("trigger selection is invalid")
    rendered: list[str] = []
    used: set[str] = set()
    for item in value.split(","):
        channel, condition = item.split("=", 1)
        if channel not in aliases:
            raise ValueError("trigger references a disabled channel")
        effective = aliases[channel]
        if effective in used:
            raise ValueError("trigger contains a duplicate channel")
        used.add(effective)
        rendered.append(f"{effective}={condition}")
    return ",".join(rendered)


def probe(args: argparse.Namespace) -> dict[str, Any]:
    runtime_status()
    before = require_device(args.vid, args.pid, args.location)
    if before["bcd_device"] != args.expected_bcd:
        raise RuntimeError("analyzer pre-firmware bcdDevice does not match")
    if before["firmware_state"] != args.expected_state:
        raise RuntimeError("analyzer pre-firmware state does not match")
    try:
        result = run_sigrok(
            [
                "--driver",
                f"fx2lafw:conn={before['connection']}",
                "--scan",
            ],
            timeout=30.0,
        )
        if result.returncode != 0:
            raise RuntimeError(
                "sigrok firmware probe failed: "
                + (result.stderr.strip().splitlines() or ["unknown error"])[0]
            )
        after = require_device(args.vid, args.pid, args.location)
        if after["firmware_state"] != "fx2lafw":
            raise RuntimeError(
                "analyzer did not enter the expected fx2lafw state"
            )
    except (
        OSError,
        RuntimeError,
        subprocess.SubprocessError,
        ValueError,
    ) as error:
        raise RuntimeError(
            f"{error}; the post-operation identity check did not complete, "
            "so the analyzer firmware state is unknown"
        ) from error
    return {
        "schema_version": 1,
        "status": "passed",
        "operation": "probe",
        "volatile_firmware_uploaded": True,
        "power_cycle_restores_original_firmware": True,
        "before": before,
        "after": after,
        "sigrok_output": (result.stdout + result.stderr).strip(),
    }


def capture(args: argparse.Namespace) -> dict[str, Any]:
    channels, aliases = normalize_channels(args.channels)
    trigger = normalize_trigger(args.trigger, aliases)
    output: Path = args.output
    if output.suffix.lower() != ".sr":
        raise ValueError("capture output must use the .sr suffix")
    if output.is_symlink() or output.exists():
        raise ValueError("capture output must be a new non-symlink path")
    duration = args.samples / SAMPLERATES[args.samplerate]
    if duration > MAX_CAPTURE_SECONDS:
        raise ValueError("capture duration exceeds the 60 second bound")
    runtime_status()
    before = require_device(args.vid, args.pid, args.location)
    if before["bcd_device"] != args.expected_bcd:
        raise RuntimeError("analyzer pre-firmware bcdDevice does not match")
    if before["firmware_state"] != args.expected_state:
        raise RuntimeError("analyzer pre-firmware state does not match")
    firmware_already_loaded = before["firmware_state"] == "fx2lafw"
    output.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    timeout = max(20.0, 15.0 + duration * 2.0)
    with tempfile.TemporaryDirectory(
        dir=output.parent,
        prefix=".sigrok-capture-",
    ) as directory:
        temporary = Path(directory) / "capture.sr"
        command = [
            "--driver",
            f"fx2lafw:conn={before['connection']}",
            "--config",
            f"samplerate={args.samplerate}",
            "--channels",
            channels,
            "--samples",
            str(args.samples),
            "--output-file",
            str(temporary),
        ]
        if trigger:
            command.extend(["--triggers", trigger])
        try:
            result = run_sigrok(
                command,
                timeout=timeout,
                output_limit=MAX_CAPTURE_BYTES,
            )
            if result.returncode != 0:
                raise RuntimeError(
                    "sigrok capture failed: "
                    + (
                        result.stderr.strip().splitlines()
                        or ["unknown error"]
                    )[0]
                )
            session = validate_session(temporary)
            if (
                session["sample_count"] != args.samples
                or session["samplerate"] != SAMPLERATES[args.samplerate]
                or session["unit_size"] != 1
                or session["probe_map"] != aliases
            ):
                raise RuntimeError(
                    "sigrok capture metadata does not match request"
                )
            after = require_device(args.vid, args.pid, args.location)
            if after["firmware_state"] != "fx2lafw":
                raise RuntimeError(
                    "analyzer did not remain in the expected fx2lafw state"
                )
            os.chmod(temporary, 0o600)
            publish_no_clobber(temporary, output)
        except (
            OSError,
            RuntimeError,
            subprocess.SubprocessError,
            ValueError,
            zipfile.BadZipFile,
        ) as error:
            raise RuntimeError(
                f"{error}; the post-operation identity check did not complete, "
                "so the analyzer firmware state is unknown"
            ) from error
    return {
        "schema_version": 1,
        "status": "passed",
        "operation": "capture",
        "output": str(output),
        "samplerate": args.samplerate,
        "samples": args.samples,
        "channels": channels,
        "trigger": trigger,
        "volatile_firmware_uploaded": not firmware_already_loaded,
        "firmware_already_loaded": firmware_already_loaded,
        "before": before,
        "after": after,
        "session": session,
    }


def convert(args: argparse.Namespace) -> dict[str, Any]:
    runtime_status()
    source: Path = args.input
    output: Path = args.output
    if output.is_symlink() or output.exists():
        raise ValueError("conversion output must be a new non-symlink path")
    output.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(
        dir=output.parent,
        prefix=".sigrok-convert-",
    ) as directory:
        immutable = Path(directory) / "input.sr"
        copy_regular_file(source, immutable, MAX_CAPTURE_BYTES)
        session = validate_session(immutable)
        temporary = Path(directory) / f"converted.{args.format}"
        result = run_sigrok(
            [
                "--input-file",
                str(immutable),
                "--output-format",
                args.format,
                "--output-file",
                str(temporary),
            ],
            timeout=60.0,
            output_limit=MAX_CONVERT_BYTES,
        )
        if result.returncode != 0:
            raise RuntimeError(
                "sigrok conversion failed: "
                + (result.stderr.strip().splitlines() or ["unknown error"])[0]
            )
        if temporary.is_symlink() or not temporary.is_file():
            raise RuntimeError("sigrok conversion produced no regular output")
        size = temporary.stat().st_size
        if not 0 < size <= MAX_CONVERT_BYTES:
            raise RuntimeError("sigrok conversion output size is invalid")
        os.chmod(temporary, 0o600)
        publish_no_clobber(temporary, output)
    return {
        "schema_version": 1,
        "status": "passed",
        "operation": "convert",
        "input": str(source),
        "input_sha256": session["sha256"],
        "format": args.format,
        "output": str(output),
        "output_size": size,
        "output_sha256": sha256_file(output),
    }


def build_parser() -> argparse.ArgumentParser:
    def bounded_samples(value: str) -> int:
        try:
            parsed = int(value, 10)
        except ValueError as error:
            raise argparse.ArgumentTypeError(
                "sample count must be an integer"
            ) from error
        if not 1 <= parsed <= MAX_CAPTURE_SAMPLES:
            raise argparse.ArgumentTypeError(
                f"sample count must be between 1 and {MAX_CAPTURE_SAMPLES}"
            )
        return parsed

    parser = argparse.ArgumentParser(
        prog="sigrok-wrapper",
        description="Run bounded project-local sigrok logic-analyzer tools.",
        epilog=(
            "Input protection is device-specific; qualify electrical limits "
            "before connecting an unknown clone."
        ),
    )
    parser.add_argument(
        "--version",
        action="version",
        version=f"sigrok-wrapper {WRAPPER_VERSION}",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser(
        "runtime-status",
        help="Verify the native runtime and report installed capabilities.",
    )
    discover_parser = subparsers.add_parser(
        "discover",
        help="List USB analyzers without uploading firmware.",
    )
    discover_parser.add_argument("--vid", help="Optional exact 4-hex USB VID.")
    discover_parser.add_argument("--pid", help="Optional exact 4-hex USB PID.")
    doctor_parser = subparsers.add_parser(
        "doctor",
        help="Correlate one expected IOKit/libusb identity without firmware upload.",
    )
    doctor_parser.add_argument("--vid", required=True, help="Exact 4-hex USB VID.")
    doctor_parser.add_argument("--pid", required=True, help="Exact 4-hex USB PID.")
    doctor_parser.add_argument(
        "--location",
        required=True,
        help="Exact 8-hex IOKit physical location.",
    )
    doctor_parser.add_argument(
        "--expected-bcd",
        required=True,
        help="Expected 4-hex USB bcdDevice revision.",
    )
    doctor_parser.add_argument(
        "--expected-state",
        choices=("original", "fx2lafw"),
        required=True,
        help="Expected current volatile firmware state.",
    )
    subparsers.add_parser(
        "list-supported",
        help="Print static libsigrok driver/device capability text.",
    )
    inspect_parser = subparsers.add_parser(
        "inspect",
        help="Validate and inspect one offline .sr session.",
    )
    inspect_parser.add_argument("input", type=Path, help="Existing .sr session.")
    probe_parser = subparsers.add_parser(
        "probe",
        help="Upload volatile fx2lafw and verify USB re-enumeration.",
        description=(
            "Uploads fx2lafw into analyzer RAM and re-enumerates USB; a power "
            "cycle restores the original firmware."
        ),
    )
    probe_parser.add_argument("--vid", required=True, help="Exact 4-hex USB VID.")
    probe_parser.add_argument("--pid", required=True, help="Exact 4-hex USB PID.")
    probe_parser.add_argument(
        "--location",
        required=True,
        help="Exact 8-hex IOKit physical location.",
    )
    probe_parser.add_argument(
        "--expected-bcd",
        required=True,
        help="Expected pre-upload 4-hex bcdDevice revision.",
    )
    probe_parser.add_argument(
        "--expected-state",
        choices=("original",),
        required=True,
        help="Probe starts from the expected original firmware state.",
    )
    capture_parser = subparsers.add_parser(
        "capture",
        help="Acquire one bounded session from an exact expected analyzer.",
        description=(
            "Acquires a bounded session. If the analyzer is in original state, "
            "libsigrok uploads fx2lafw into RAM and re-enumerates USB."
        ),
    )
    capture_parser.add_argument("--vid", required=True, help="Exact 4-hex USB VID.")
    capture_parser.add_argument("--pid", required=True, help="Exact 4-hex USB PID.")
    capture_parser.add_argument(
        "--location",
        required=True,
        help="Exact 8-hex IOKit physical location.",
    )
    capture_parser.add_argument(
        "--expected-bcd",
        required=True,
        help="Expected 4-hex bcdDevice for the current firmware state.",
    )
    capture_parser.add_argument(
        "--expected-state",
        choices=("original", "fx2lafw"),
        required=True,
        help="Fail closed unless current firmware state matches.",
    )
    capture_parser.add_argument(
        "--channels",
        required=True,
        help=(
            "Comma-separated D0..D7 selections, optionally D0=ALIAS; aliases "
            "cannot shadow physical channel names."
        ),
    )
    capture_parser.add_argument(
        "--samplerate",
        choices=tuple(SAMPLERATES),
        required=True,
        help="Exact digital sample rate; this is not signal bandwidth.",
    )
    capture_parser.add_argument(
        "--samples",
        type=bounded_samples,
        required=True,
        metavar="COUNT",
        help=(
            f"Sample count from 1 to {MAX_CAPTURE_SAMPLES}; count/rate must not "
            f"exceed {MAX_CAPTURE_SECONDS:.0f} seconds."
        ),
    )
    capture_parser.add_argument(
        "--trigger",
        help=(
            "Comma-separated physical conditions Dn=0|1|r|f|e "
            "(low, high, rising, falling, either)."
        ),
    )
    capture_parser.add_argument(
        "--output",
        type=Path,
        required=True,
        help="New non-symlink .sr output.",
    )
    convert_parser = subparsers.add_parser(
        "convert",
        help="Convert one validated offline .sr session.",
    )
    convert_parser.add_argument("input", type=Path, help="Existing .sr session.")
    convert_parser.add_argument(
        "--format",
        choices=("binary", "csv", "vcd"),
        required=True,
        help="Output format; binary loses samplerate and channel metadata.",
    )
    convert_parser.add_argument(
        "--output",
        type=Path,
        required=True,
        help="New non-symlink output path.",
    )
    return parser


def main() -> None:
    parser = build_parser()
    args = parser.parse_args()
    try:
        if args.command == "runtime-status":
            result: Any = runtime_status()
        elif args.command == "discover":
            if (args.vid is None) != (args.pid is None):
                raise ValueError("--vid and --pid must be supplied together")
            vid = parse_hex(args.vid, 16, "VID") if args.vid else None
            pid = parse_hex(args.pid, 16, "PID") if args.pid else None
            result = {
                "schema_version": 1,
                "operation": "discover",
                "devices": discover(vid, pid),
                "volatile_firmware_uploaded": False,
            }
        elif args.command == "list-supported":
            runtime_status()
            completed = run_sigrok(["--list-supported"], timeout=15.0)
            if completed.returncode != 0:
                raise RuntimeError("sigrok static capability listing failed")
            sys.stdout.write(completed.stdout)
            return
        elif args.command == "doctor":
            vid = parse_hex(args.vid, 16, "VID")
            pid = parse_hex(args.pid, 16, "PID")
            location = parse_hex(args.location, 32, "location")
            expected_bcd = parse_hex(args.expected_bcd, 16, "bcdDevice")
            runtime = runtime_status()
            device = require_device(vid, pid, location)
            result = {
                "schema_version": 1,
                "operation": "doctor",
                "ready": (
                    device["bcd_device"] == f"{expected_bcd:04x}"
                    and device["firmware_state"] == args.expected_state
                ),
                "volatile_firmware_uploaded": False,
                "runtime": runtime,
                "device": device,
            }
        elif args.command == "inspect":
            result = {
                "schema_version": 1,
                "operation": "inspect",
                "input": str(args.input),
                "session": validate_session(args.input),
            }
        elif args.command == "probe":
            args.vid = parse_hex(args.vid, 16, "VID")
            args.pid = parse_hex(args.pid, 16, "PID")
            args.location = parse_hex(args.location, 32, "location")
            expected_bcd = parse_hex(args.expected_bcd, 16, "bcdDevice")
            args.expected_bcd = f"{expected_bcd:04x}"
            result = probe(args)
        elif args.command == "capture":
            args.vid = parse_hex(args.vid, 16, "VID")
            args.pid = parse_hex(args.pid, 16, "PID")
            args.location = parse_hex(args.location, 32, "location")
            expected_bcd = parse_hex(args.expected_bcd, 16, "bcdDevice")
            args.expected_bcd = f"{expected_bcd:04x}"
            result = capture(args)
        else:
            result = convert(args)
        print(json.dumps(result, indent=2, sort_keys=True))
    except (
        OSError,
        RuntimeError,
        subprocess.SubprocessError,
        ValueError,
        zipfile.BadZipFile,
    ) as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
