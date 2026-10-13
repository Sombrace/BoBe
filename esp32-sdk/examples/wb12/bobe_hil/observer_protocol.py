import json
import re
from dataclasses import dataclass
from typing import Any


PROTOCOL_SCHEMA = 1
MAX_REQUEST_BYTES = 8192
MAX_RESPONSE_BYTES = 16384
MAX_ARTIFACT_CHUNK_BYTES = 4096
OPERATIONS = {
    "artifact-read",
    "physical-bodylink-audio-loop",
    "status",
}
ARTIFACT_NAMES = {
    "bodylink-microphone.pcm",
    "bodylink-speaker.pcm",
    "bodylink-transcript.jsonl",
    "fixture-transcript.jsonl",
    "observer-inputs.pcm",
    "uplink-stimulus.pcm",
}
ARTIFACT_MAX_BYTES = {
    "bodylink-microphone.pcm": 500 * 320 * 2,
    "bodylink-speaker.pcm": 150 * 480 * 2,
    "bodylink-transcript.jsonl": 1024 * 1024,
    "fixture-transcript.jsonl": 1024 * 1024,
    "observer-inputs.pcm": 384000 * 2 * 2,
    "uplink-stimulus.pcm": 48000 * 2,
}
MAX_ARTIFACT_BYTES = max(ARTIFACT_MAX_BYTES.values())
MAX_TOTAL_ARTIFACT_BYTES = sum(ARTIFACT_MAX_BYTES.values())
REQUEST_ID_PATTERN = re.compile(r"^[0-9a-f]{32}$")
HEX_64_PATTERN = re.compile(r"^[0-9a-f]{64}$")
PHYSICAL_BINDING_FIELDS = {
    "client_spki_sha256",
    "contract_sha256",
    "device_boot_id",
    "device_id",
    "firmware_elf_sha256",
    "fixture_board_id",
    "host_nonce",
    "minimum_run_counter",
    "usb_serial_number",
}
ARTIFACT_BINDING_FIELDS = {
    "artifact_name",
    "length",
    "offset",
    "run_counter",
    "transfer_nonce",
}


@dataclass(frozen=True)
class ObserverRequest:
    request_id: str
    operation: str
    binding: dict[str, Any]


def parse_request(data: bytes) -> ObserverRequest:
    if (
        not data
        or len(data) > MAX_REQUEST_BYTES
        or b"\n" in data.rstrip(b"\n")
    ):
        raise ValueError("observer request framing is invalid")
    try:
        value = json.loads(data)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError("observer request is not valid JSON") from error
    if not isinstance(value, dict) or set(value) != {
        "binding",
        "operation",
        "request_id",
        "schema_version",
    }:
        raise ValueError("observer request fields are invalid")
    request_id = value["request_id"]
    operation = value["operation"]
    binding = value["binding"]
    if (
        type(value["schema_version"]) is not int
        or value["schema_version"] != PROTOCOL_SCHEMA
    ):
        raise ValueError("observer request schema is unsupported")
    if (
        not isinstance(request_id, str)
        or REQUEST_ID_PATTERN.fullmatch(request_id) is None
    ):
        raise ValueError("observer request ID is malformed")
    if not isinstance(operation, str) or operation not in OPERATIONS:
        raise ValueError("observer operation is unsupported")
    if not isinstance(binding, dict):
        raise ValueError("observer request binding is invalid")
    if operation == "status":
        if binding:
            raise ValueError("observer status accepts no binding")
    elif operation == "physical-bodylink-audio-loop":
        validate_physical_binding(binding)
    else:
        validate_artifact_binding(binding)
    return ObserverRequest(request_id, operation, binding)


def validate_physical_binding(binding: dict[str, Any]) -> None:
    if set(binding) != PHYSICAL_BINDING_FIELDS:
        raise ValueError("observer physical binding fields are invalid")
    for field in (
        "client_spki_sha256",
        "contract_sha256",
        "firmware_elf_sha256",
        "host_nonce",
    ):
        if (
            not isinstance(binding[field], str)
            or HEX_64_PATTERN.fullmatch(binding[field]) is None
        ):
            raise ValueError("observer physical binding value is invalid")
    if (
        not isinstance(binding["device_boot_id"], str)
        or re.fullmatch(r"[0-9a-f]{32}", binding["device_boot_id"]) is None
        or not isinstance(binding["fixture_board_id"], str)
        or re.fullmatch(r"[0-9a-f]{16}", binding["fixture_board_id"]) is None
        or not isinstance(binding["usb_serial_number"], str)
        or re.fullmatch(r"[0-9a-f]{12}", binding["usb_serial_number"])
        is None
        or not isinstance(binding["device_id"], str)
        or not 1 <= len(binding["device_id"]) <= 64
        or not binding["device_id"].isascii()
        or not all(character.isprintable() for character in binding["device_id"])
        or type(binding["minimum_run_counter"]) is not int
        or not 0 <= binding["minimum_run_counter"] < 2**63
    ):
        raise ValueError("observer physical binding value is invalid")


def validate_artifact_binding(binding: dict[str, Any]) -> None:
    if set(binding) != ARTIFACT_BINDING_FIELDS:
        raise ValueError("observer artifact binding fields are invalid")
    if (
        not isinstance(binding["artifact_name"], str)
        or binding["artifact_name"] not in ARTIFACT_NAMES
        or type(binding["run_counter"]) is not int
        or not 1 <= binding["run_counter"] < 2**63
        or type(binding["offset"]) is not int
        or not 0
        <= binding["offset"]
        < ARTIFACT_MAX_BYTES[binding["artifact_name"]]
        or type(binding["length"]) is not int
        or not 1 <= binding["length"] <= MAX_ARTIFACT_CHUNK_BYTES
        or binding["offset"] + binding["length"]
        > ARTIFACT_MAX_BYTES[binding["artifact_name"]]
        or not isinstance(binding["transfer_nonce"], str)
        or HEX_64_PATTERN.fullmatch(binding["transfer_nonce"]) is None
    ):
        raise ValueError("observer artifact binding value is invalid")


def encode_request(
    request_id: str,
    operation: str,
    binding: dict[str, Any],
) -> bytes:
    rendered = (
        json.dumps(
            {
                "schema_version": PROTOCOL_SCHEMA,
                "request_id": request_id,
                "operation": operation,
                "binding": binding,
            },
            separators=(",", ":"),
            sort_keys=True,
        )
        + "\n"
    ).encode()
    parse_request(rendered)
    return rendered


def response(
    request_id: str,
    *,
    ok: bool,
    result: dict[str, Any] | None = None,
    error_code: str | None = None,
    message: str | None = None,
) -> bytes:
    value: dict[str, Any] = {
        "schema_version": PROTOCOL_SCHEMA,
        "request_id": request_id,
        "ok": ok,
    }
    if ok:
        value["result"] = result or {}
    else:
        value["error"] = {
            "code": error_code or "INTERNAL_ERROR",
            "message": message or "observer operation failed",
        }
    rendered = (
        json.dumps(value, separators=(",", ":"), sort_keys=True) + "\n"
    ).encode()
    if len(rendered) > MAX_RESPONSE_BYTES:
        raise ValueError("observer response exceeds bound")
    return rendered


def parse_response(data: bytes, expected_request_id: str) -> dict[str, Any]:
    if (
        not isinstance(expected_request_id, str)
        or REQUEST_ID_PATTERN.fullmatch(expected_request_id) is None
    ):
        raise ValueError("observer expected request ID is malformed")
    if (
        not data
        or len(data) > MAX_RESPONSE_BYTES
        or b"\n" in data.rstrip(b"\n")
    ):
        raise ValueError("observer response framing is invalid")
    try:
        value = json.loads(data)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError("observer response is not valid JSON") from error
    if (
        not isinstance(value, dict)
        or type(value.get("schema_version")) is not int
        or value.get("schema_version") != PROTOCOL_SCHEMA
        or value.get("request_id") != expected_request_id
        or type(value.get("ok")) is not bool
    ):
        raise ValueError("observer response binding is invalid")
    if value["ok"]:
        if set(value) != {"ok", "request_id", "result", "schema_version"}:
            raise ValueError("observer success response fields are invalid")
        result = value["result"]
        if not isinstance(result, dict):
            raise ValueError("observer success result is invalid")
        return result
    if set(value) != {"error", "ok", "request_id", "schema_version"}:
        raise ValueError("observer error response fields are invalid")
    error = value["error"]
    if (
        not isinstance(error, dict)
        or set(error) != {"code", "message"}
        or not isinstance(error["code"], str)
        or not isinstance(error["message"], str)
        or not 1 <= len(error["code"]) <= 64
        or not 1 <= len(error["message"]) <= 1024
    ):
        raise ValueError("observer error response is invalid")
    raise RuntimeError(f"observer {error['code']}: {error['message']}")
