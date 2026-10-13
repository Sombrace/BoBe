import json
import platform
import socket
import subprocess
import sys
from typing import Any

from serial.tools import list_ports

from bobe_hil import __version__
from bobe_hil.calibration import inspect_calibrations
from bobe_hil.paths import HIL_PRIVATE_ROOT, REPOSITORY_ROOT, git_metadata
from bobe_hil.readiness import PHYSICAL_REQUIREMENTS, probe_requirements


def command_version(command: list[str]) -> str:
    result = subprocess.run(
        command,
        cwd=REPOSITORY_ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    output = (result.stdout or result.stderr).strip().splitlines()
    if not output:
        raise RuntimeError(f"{command[0]} returned no version")
    return output[0]


def audio_devices() -> list[dict[str, Any]]:
    try:
        import sounddevice
    except ImportError:
        return []
    try:
        return [
            {
                "index": index,
                "name": device["name"],
                "inputs": device["max_input_channels"],
                "outputs": device["max_output_channels"],
                "sample_rate": device["default_samplerate"],
            }
            for index, device in enumerate(sounddevice.query_devices())
        ]
    except sounddevice.PortAudioError:
        return []


def report() -> dict[str, Any]:
    git_commit, dirty = git_metadata(REPOSITORY_ROOT)
    ports = [
        {
            "device": port.device,
            "description": port.description,
            "vid": port.vid,
            "pid": port.pid,
            "serial_number": port.serial_number,
        }
        for port in list_ports.comports()
    ]
    requirement_status = probe_requirements()
    unavailable = [
        f"{name}:{requirement_status[name]['reason']}"
        for name in PHYSICAL_REQUIREMENTS
        if requirement_status[name]["state"] != "available"
    ]
    devices = audio_devices()
    if not devices:
        unavailable.append("coreaudio:no-devices")
    return {
        "schema_version": 3,
        "bobe_hil_version": __version__,
        "host": {
            "platform": platform.platform(),
            "python": sys.version.split()[0],
            "hostname": socket.gethostname(),
        },
        "source": {
            "commit": git_commit,
            "dirty": dirty,
        },
        "tools": {
            "git": command_version(["git", "--version"]),
            "uv": command_version(["uv", "--version"]),
        },
        "private_config_root": str(HIL_PRIVATE_ROOT),
        "serial_ports": ports,
        "audio_devices": devices,
        "calibrations": inspect_calibrations(),
        "physical_hardware_complete": not unavailable,
        "unavailable": unavailable,
        "requirements": requirement_status,
    }


def render_json() -> str:
    return json.dumps(report(), indent=2, sort_keys=True)
