import hashlib
import json
import re
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from bobe_hil.paths import SCENARIOS_ROOT


SCENARIO_SCHEMA = 2
CLAIMS = {"logical", "protocol", "physical"}
REQUIREMENTS = {
    "audio-fixture",
    "bodylink-stub",
    "fixed-camera-rig",
    "independent-observer",
    "lvgl-native",
    "openwrt",
    "pico-fixture",
    "wb12",
}
OPERATIONS = {
    "bodylink_stop",
    "bodylink_stub",
    "capture_audio",
    "capture_frame",
    "capture_serial",
    "fixture_chord",
    "fixture_press",
    "fixture_release_all",
    "fixture_reset",
    "network_profile",
    "ui_golden",
}


@dataclass(frozen=True)
class ScenarioStep:
    step_id: str
    operation: str
    parameters: dict[str, Any]


@dataclass(frozen=True)
class Scenario:
    name: str
    description: str
    claim: str
    requires: tuple[str, ...]
    steps: tuple[ScenarioStep, ...]
    cleanup: tuple[ScenarioStep, ...]
    path: Path


def scenario_paths() -> list[Path]:
    if not SCENARIOS_ROOT.is_dir():
        return []
    return sorted(SCENARIOS_ROOT.glob("*.toml"))


def load_scenario(path: Path) -> Scenario:
    if path.is_symlink() or not path.is_file() or path.parent != SCENARIOS_ROOT:
        raise ValueError("scenario must be a regular file in hil/scenarios")
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    if data.get("schema_version") != SCENARIO_SCHEMA:
        raise ValueError(f"{path.name}: unsupported scenario schema")
    name = data.get("name")
    description = data.get("description")
    claim = data.get("claim")
    requires = data.get("requires", [])
    raw_steps = data.get("steps", [])
    raw_cleanup = data.get("cleanup", [])
    if (
        not isinstance(name, str)
        or path.stem != name
        or not isinstance(description, str)
        or claim not in CLAIMS
        or not isinstance(requires, list)
        or not all(isinstance(item, str) for item in requires)
        or len(set(requires)) != len(requires)
        or not set(requires).issubset(REQUIREMENTS)
        or not isinstance(raw_steps, list)
        or not raw_steps
        or not isinstance(raw_cleanup, list)
    ):
        raise ValueError(f"{path.name}: malformed scenario metadata")
    steps = parse_steps(path, raw_steps, "step")
    cleanup = parse_steps(path, raw_cleanup, "cleanup")
    identifiers = [step.step_id for step in (*steps, *cleanup)]
    if len(set(identifiers)) != len(identifiers):
        raise ValueError(f"{path.name}: step identifiers must be unique")
    if claim == "physical" and not cleanup:
        raise ValueError(f"{path.name}: physical scenario requires cleanup")
    return Scenario(
        name,
        description,
        claim,
        tuple(requires),
        tuple(steps),
        tuple(cleanup),
        path,
    )


def exact_parameters(
    path: Path,
    index: int,
    operation: str,
    parameters: dict[str, Any],
    expected: set[str],
) -> None:
    if set(parameters) != expected:
        raise ValueError(
            f"{path.name}: {operation} step {index} parameters are invalid"
        )


def bounded_integer(value: Any, minimum: int, maximum: int) -> bool:
    return type(value) is int and minimum <= value <= maximum


def string_list(value: Any) -> bool:
    return (
        isinstance(value, list)
        and all(isinstance(item, str) and item for item in value)
        and len(value) <= 32
    )


def validate_parameters(
    path: Path,
    index: int,
    operation: str,
    parameters: dict[str, Any],
) -> None:
    if operation == "ui_golden":
        exact_parameters(path, index, operation, parameters, {"set"})
        valid = parameters["set"] == "all-expressions"
    elif operation in {
        "bodylink_stop",
        "fixture_reset",
        "fixture_release_all",
    }:
        exact_parameters(path, index, operation, parameters, set())
        valid = True
    elif operation == "fixture_press":
        exact_parameters(
            path,
            index,
            operation,
            parameters,
            {"control", "duration_ms"},
        )
        valid = (
            parameters["control"] in {"action", "volume-down", "volume-up"}
            and bounded_integer(parameters["duration_ms"], 30, 2000)
        )
    elif operation == "fixture_chord":
        exact_parameters(
            path,
            index,
            operation,
            parameters,
            {"chord", "duration_ms"},
        )
        valid = (
            parameters["chord"] in {"shutdown", "provision-recovery"}
            and bounded_integer(parameters["duration_ms"], 30, 2000)
        )
    elif operation == "capture_serial":
        exact_parameters(
            path,
            index,
            operation,
            parameters,
            {"duration_seconds", "expect", "reject"},
        )
        valid = (
            bounded_integer(parameters["duration_seconds"], 1, 600)
            and string_list(parameters["expect"])
            and string_list(parameters["reject"])
        )
    elif operation == "bodylink_stub":
        exact_parameters(
            path,
            index,
            operation,
            parameters,
            {"scenario", "timeout_seconds", "background"},
        )
        valid = (
            parameters["scenario"] in {"handshake", "voice-roundtrip"}
            and bounded_integer(parameters["timeout_seconds"], 30, 600)
            and type(parameters["background"]) is bool
        )
    elif operation == "capture_audio":
        exact_parameters(
            path,
            index,
            operation,
            parameters,
            {"direction", "duration_seconds"},
        )
        valid = (
            parameters["direction"] in {"microphone", "speaker"}
            and bounded_integer(parameters["duration_seconds"], 1, 10)
        )
    elif operation == "capture_frame":
        exact_parameters(path, index, operation, parameters, {"state"})
        valid = parameters["state"] in {
            "boot",
            "idle",
            "listening",
            "speaking",
        }
    elif operation == "network_profile":
        exact_parameters(
            path,
            index,
            operation,
            parameters,
            {"profile", "ttl_seconds"},
        )
        valid = (
            parameters["profile"]
            in {
                "clean",
                "delay-150",
                "loss-1",
                "loss-5",
                "jitter-loss",
                "rate-512",
                "blackhole",
            }
            and bounded_integer(parameters["ttl_seconds"], 5, 600)
        )
    else:
        valid = False
    if not valid:
        raise ValueError(
            f"{path.name}: {operation} step {index} values are invalid"
        )


def parse_steps(
    path: Path,
    raw_steps: list[Any],
    section: str,
) -> list[ScenarioStep]:
    steps: list[ScenarioStep] = []
    for index, raw_step in enumerate(raw_steps):
        if not isinstance(raw_step, dict):
            raise ValueError(f"{path.name}: {section} {index} is not a table")
        step_id = raw_step.get("id")
        operation = raw_step.get("operation")
        if operation not in OPERATIONS:
            raise ValueError(
                f"{path.name}: step {index} operation {operation!r} has no "
                f"implementation"
            )
        if (
            not isinstance(step_id, str)
            or re.fullmatch(r"[a-z][a-z0-9-]{0,47}", step_id) is None
        ):
            raise ValueError(f"{path.name}: step {index} ID is malformed")
        parameters = {
            key: value
            for key, value in raw_step.items()
            if key not in {"id", "operation"}
        }
        validate_parameters(path, index, operation, parameters)
        steps.append(ScenarioStep(step_id, operation, parameters))
    return steps


def load_named_scenario(name: str) -> Scenario:
    if not name or "/" in name or name.startswith("."):
        raise ValueError("scenario name is malformed")
    return load_scenario(SCENARIOS_ROOT / f"{name}.toml")


def list_scenarios() -> list[Scenario]:
    return [load_scenario(path) for path in scenario_paths()]


def scenario_contract(scenario: Scenario) -> dict[str, Any]:
    def render_steps(
        steps: tuple[ScenarioStep, ...],
    ) -> list[dict[str, Any]]:
        return [
            {
                "id": step.step_id,
                "operation": step.operation,
                "parameters": step.parameters,
            }
            for step in steps
        ]

    return {
        "schema_version": SCENARIO_SCHEMA,
        "name": scenario.name,
        "claim": scenario.claim,
        "requires": list(scenario.requires),
        "steps": render_steps(scenario.steps),
        "cleanup": render_steps(scenario.cleanup),
    }


def contract_sha256(contract: dict[str, Any]) -> str:
    return hashlib.sha256(
        json.dumps(
            contract,
            separators=(",", ":"),
            sort_keys=True,
        ).encode()
    ).hexdigest()


def scenario_contract_sha256(scenario: Scenario) -> str:
    """Identify a scenario contract in run evidence."""

    return contract_sha256(scenario_contract(scenario))
