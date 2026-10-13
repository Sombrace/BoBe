import json
import subprocess
from pathlib import Path
from typing import Any

from bobe_hil import device, fixture, network, observer
from bobe_hil.bodylink import (
    CA_CERT_PATH,
    CLAIM_PATH,
    SERVER_CERT_PATH,
    SERVER_KEY_PATH,
)
from bobe_hil.calibration import (
    load_audio_calibration,
    load_camera_calibration,
)
from bobe_hil.media import audio_device_list
from bobe_hil.paths import REPOSITORY_ROOT
from bobe_hil.scenario import REQUIREMENTS
from bobe_hil.ui import reference_case_count


PHYSICAL_REQUIREMENTS = (
    "wb12",
    "pico-fixture",
    "openwrt",
    "fixed-camera-rig",
    "audio-fixture",
    "independent-observer",
)
STATES = {"available", "unavailable", "error"}


def status(state: str, reason: str, **evidence: Any) -> dict[str, Any]:
    if state not in STATES:
        raise ValueError("requirement state is invalid")
    return {"state": state, "reason": reason, "evidence": evidence}


def fixed_files_ready(paths: tuple[Path, ...]) -> bool:
    return all(path.is_file() and not path.is_symlink() for path in paths)


def probe_wb12() -> dict[str, Any]:
    report = device.inspect_device()
    if report["available"]:
        reason = "one-wb12-connected"
    elif len(report["boards"]) > 1:
        reason = "several-wb12-connected"
    else:
        reason = "wb12-not-connected"
    return status(
        "available" if report["available"] else "unavailable",
        reason,
        **report,
    )


def probe_fixture() -> dict[str, Any]:
    report = fixture.inspect_fixture()
    if not report["available"]:
        return status("unavailable", "pico-fixture-not-enumerated", **report)
    try:
        probe = fixture.status()
    except (OSError, RuntimeError, TimeoutError, ValueError) as error:
        return status(
            "error",
            "pico-fixture-probe-failed",
            error=str(error),
            **report,
        )
    return status(
        "available",
        "pico-fixture-identity-and-protocol-verified",
        probe=probe,
        **report,
    )


def probe_openwrt() -> dict[str, Any]:
    report = network.inspect_network()
    if not report["available"]:
        return status("unavailable", "openwrt-not-configured", **report)
    try:
        probe = network.status()
    except (OSError, RuntimeError, TimeoutError, ValueError) as error:
        return status(
            "error",
            "openwrt-probe-failed",
            error=str(error),
            **report,
        )
    return status("available", "openwrt-netem-reachable", probe=probe, **report)


def camera_inventory() -> dict[str, Any]:
    completed = subprocess.run(
        [
            str(REPOSITORY_ROOT / "tools" / "bobe_capture.sh"),
            "list",
            "--summary",
        ],
        cwd=REPOSITORY_ROOT,
        check=False,
        capture_output=True,
        text=True,
        timeout=120.0,
    )
    stream = completed.stdout if completed.returncode == 0 else completed.stderr
    value = json.loads(stream)
    if not isinstance(value, dict):
        raise ValueError("camera inventory JSON is not an object")
    if completed.returncode != 0:
        raise RuntimeError(
            str(value.get("error", {}).get("message", "camera inventory failed"))
        )
    return value


def probe_camera() -> dict[str, Any]:
    try:
        inventory = camera_inventory()
    except (
        json.JSONDecodeError,
        OSError,
        RuntimeError,
        subprocess.TimeoutExpired,
        ValueError,
    ) as error:
        return status("error", "camera-inventory-failed", error=str(error))
    try:
        calibration = load_camera_calibration()
    except (OSError, ValueError) as error:
        return status(
            "unavailable",
            "camera-calibration-unavailable",
            error=str(error),
            inventory=inventory,
        )
    try:
        preflight = subprocess.run(
            [
                str(REPOSITORY_ROOT / "tools" / "bobe_capture.sh"),
                "preflight",
                "--camera-id",
                str(calibration["camera_id"]),
            ],
            cwd=REPOSITORY_ROOT,
            check=False,
            capture_output=True,
            text=True,
            timeout=120.0,
        )
        value = json.loads(preflight.stdout)
        if not isinstance(value, dict):
            raise ValueError("camera preflight JSON is not an object")
    except (
        json.JSONDecodeError,
        OSError,
        subprocess.TimeoutExpired,
        ValueError,
    ) as error:
        return status(
            "error",
            "camera-preflight-failed",
            error=str(error),
            calibration=calibration,
            inventory=inventory,
        )
    available = preflight.returncode == 0 and value.get("status") == "READY"
    return status(
        "available" if available else "unavailable",
        "camera-preflight-ready" if available else "camera-preflight-not-ready",
        calibration=calibration,
        inventory=inventory,
        preflight=value,
        exit_code=preflight.returncode,
    )


def probe_audio() -> dict[str, Any]:
    try:
        calibration = load_audio_calibration()
    except (OSError, ValueError) as error:
        return status("unavailable", "audio-calibration-unavailable", error=str(error))
    try:
        devices = audio_device_list()
    except (OSError, RuntimeError, ValueError) as error:
        return status(
            "error",
            "coreaudio-discovery-failed",
            error=str(error),
            calibration=calibration,
        )
    input_matches = [
        item
        for item in devices
        if item["name"] == calibration["input_device_name"]
        and int(item["input_channels"]) > 0
    ]
    output_matches = [
        item
        for item in devices
        if item["name"] == calibration["output_device_name"]
        and int(item["output_channels"]) > 0
    ]
    available = len(input_matches) == 1 and len(output_matches) == 1
    return status(
        "available" if available else "unavailable",
        "calibrated-audio-devices-present"
        if available
        else "calibrated-audio-devices-absent",
        calibration=calibration,
        input_matches=input_matches,
        output_matches=output_matches,
    )


def probe_observer() -> dict[str, Any]:
    report = observer.inspect_observer(live_probe=True)
    if not report["configured"]:
        return status("unavailable", "observer-not-configured", **report)
    return status(
        "available" if report["available"] else "unavailable",
        "observer-identity-and-protocol-verified"
        if report["available"]
        else "observer-not-reachable",
        **report,
    )


def probe_lvgl() -> dict[str, Any]:
    reference_count = reference_case_count()
    available = reference_count > 0
    return status(
        "available" if available else "unavailable",
        "canonical-ui-references-present"
        if available
        else "canonical-ui-references-incomplete",
        reference_count=reference_count,
    )


def probe_bodylink() -> dict[str, Any]:
    files = (CLAIM_PATH, SERVER_CERT_PATH, SERVER_KEY_PATH, CA_CERT_PATH)
    available = fixed_files_ready(files)
    return status(
        "available" if available else "unavailable",
        "bodylink-private-config-present"
        if available
        else "bodylink-private-config-absent",
        files=[str(path) for path in files],
    )


PROBES = {
    "audio-fixture": probe_audio,
    "bodylink-stub": probe_bodylink,
    "fixed-camera-rig": probe_camera,
    "independent-observer": probe_observer,
    "lvgl-native": probe_lvgl,
    "openwrt": probe_openwrt,
    "pico-fixture": probe_fixture,
    "wb12": probe_wb12,
}


def probe_requirements(
    requirements: tuple[str, ...] | None = None,
) -> dict[str, dict[str, Any]]:
    requested = sorted(set(REQUIREMENTS if requirements is None else requirements))
    unsupported = set(requested) - set(PROBES)
    if unsupported:
        raise ValueError(
            f"readiness requirement is unsupported: {', '.join(sorted(unsupported))}"
        )
    return {name: PROBES[name]() for name in requested}


def readiness_for(
    requirements: tuple[str, ...],
) -> dict[str, dict[str, Any]]:
    return probe_requirements(requirements)
