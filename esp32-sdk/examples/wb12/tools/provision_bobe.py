#!/usr/bin/env python3

import argparse
import asyncio
import hashlib
import json
import os
import secrets
import stat
import sys
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Callable


ROOT = Path(__file__).resolve().parent.parent
ESP_PROV = (
    ROOT
    / "managed_components/espressif__network_provisioning/tool/esp_prov"
)
IDF_VERSION = os.environ.get("BOBE_IDF_VERSION", "6.0.2")
IDF_PATH = Path(os.environ.get("IDF_PATH", str(ROOT.parents[1] / f".deps/esp-idf-v{IDF_VERSION}")))
CLAIM_FIELDS = {
    "endpoint_uri",
    "trust_kid",
    "device_id",
    "controller_epoch",
    "server_ca_pem",
    "client_cert_pem",
    "client_key_pem",
}
RECEIPT_SCHEMA = 1


def private_json(path: Path, maximum_size: int) -> dict[str, Any]:
    if path.is_symlink():
        raise ValueError(f"refusing symbolic link: {path}")
    metadata = path.stat()
    if not stat.S_ISREG(metadata.st_mode):
        raise ValueError(f"not a regular file: {path}")
    if stat.S_IMODE(metadata.st_mode) & 0o077:
        raise ValueError(f"private file permissions are too broad: {path}")
    if metadata.st_size <= 0 or metadata.st_size > maximum_size:
        raise ValueError(f"private file has an invalid size: {path}")
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"expected a JSON object: {path}")
    return value


def atomic_receipt(path: Path, value: dict[str, Any]) -> None:
    if path.is_symlink():
        raise ValueError("provisioning receipt must not be a symbolic link")
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    path.parent.chmod(0o700)
    temporary = path.with_name(f".{path.name}.{secrets.token_hex(8)}.tmp")
    descriptor = os.open(
        temporary,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL,
        0o600,
    )
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as output:
            json.dump(value, output, indent=2, sort_keys=True)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def default_receipt_path(run_id: str) -> Path:
    return ROOT / "private" / "provisioning-receipts" / f"{run_id}.json"


def validated_receipt_path(path: Path) -> Path:
    private_root = (ROOT / "private").resolve()
    resolved = path.resolve()
    if resolved == private_root or private_root not in resolved.parents:
        raise ValueError("provisioning receipt must stay under private/")
    return resolved


def validated_inputs(
    credentials_path: Path,
    claim_path: Path,
) -> tuple[dict[str, str], dict[str, Any]]:
    credentials = private_json(credentials_path, 16384)
    claim = private_json(claim_path, 32768)
    if credentials.get("schema_version") != 1:
        raise ValueError("credentials use an unsupported schema")
    required_credentials = {
        "generation_id",
        "device_id",
        "username",
        "password",
        "softap_password",
    }
    for name in required_credentials:
        if not isinstance(credentials.get(name), str) or not credentials[name]:
            raise ValueError(f"credentials are missing {name}")
    if len(credentials["softap_password"]) < 16 or len(
        credentials["softap_password"]
    ) > 63:
        raise ValueError("SoftAP password must contain 16 to 63 characters")
    if set(claim) != CLAIM_FIELDS:
        raise ValueError("claim fields do not match the BodyLink V1 schema")
    if claim.get("device_id") != credentials["device_id"]:
        raise ValueError("claim and provisioning device identities differ")
    if not isinstance(claim.get("controller_epoch"), int) or claim[
        "controller_epoch"
    ] < 1:
        raise ValueError("claim controller_epoch must be a positive integer")
    encoded_claim = json.dumps(
        claim,
        separators=(",", ":"),
        ensure_ascii=True,
    ).encode()
    if len(encoded_claim) > 16384:
        raise ValueError("claim exceeds the device endpoint limit")
    return credentials, claim


def wifi_wire_values(ssid: str, passphrase: str) -> tuple[str, str]:
    ssid_bytes = ssid.encode("utf-8")
    passphrase_bytes = passphrase.encode("utf-8")
    if not 1 <= len(ssid_bytes) <= 32 or b"\0" in ssid_bytes:
        raise ValueError("SSID must contain 1 to 32 UTF-8 bytes without NUL")
    if (
        len(passphrase_bytes) not in range(8, 64)
        and len(passphrase_bytes) != 0
    ) or b"\0" in passphrase_bytes:
        raise ValueError(
            "Wi-Fi passphrase must be empty or contain 8 to 63 UTF-8 bytes "
            "without NUL"
        )
    return (
        ssid_bytes.decode("latin-1"),
        passphrase_bytes.decode("latin-1"),
    )


def load_esp_prov() -> tuple[Any, Any, Any, Any]:
    if not ESP_PROV.is_dir() or not IDF_PATH.is_dir():
        raise RuntimeError("run tools/bootstrap_idf.sh and reconfigure first")
    os.environ["IDF_PATH"] = str(IDF_PATH)
    sys.path.insert(0, str(ESP_PROV))
    import prov
    import security
    import transport
    from utils import str_to_bytes

    return prov, security, transport, str_to_bytes


async def establish_session(transport: Any, security: Any) -> None:
    response = None
    while True:
        request = security.security_session(response)
        if request is None:
            return
        response = await transport.send_data("prov-session", request)
        if response is None:
            raise RuntimeError("device ended the Security-2 handshake")


async def provision(
    host: str,
    ssid: str,
    passphrase: str,
    credentials: dict[str, str],
    claim: dict[str, Any],
    phase_committed: Callable[[str], None],
) -> None:
    prov, security, transport_module, str_to_bytes = load_esp_prov()
    wire_ssid, wire_passphrase = wifi_wire_values(ssid, passphrase)
    transport = transport_module.Transport_HTTP(host)
    try:
        version_response = await transport.send_data("proto-ver", "---")
        version = json.loads(version_response)
        patch_version = int(version.get("prov", {}).get("sec_patch_ver", 0))
        phase_committed("device-preflight")
        security_context = security.Security2(
            patch_version,
            credentials["username"],
            credentials["password"],
            False,
        )
        await establish_session(transport, security_context)

        claim_data = json.dumps(
            claim,
            separators=(",", ":"),
            ensure_ascii=True,
        ).encode()
        encrypted_claim = security_context.encrypt_data(claim_data)
        claim_response = await transport.send_data(
            "bobe-claim",
            encrypted_claim.decode("latin-1"),
        )
        decrypted_claim = security_context.decrypt_data(
            str_to_bytes(claim_response)
        )
        claim_result = json.loads(decrypted_claim)
        if claim_result.get("status") != "ok":
            raise RuntimeError("device rejected the BodyLink claim")
        phase_committed("claim-committed")

        config_request = prov.config_set_config_request(
            "wifi",
            security_context,
            wire_ssid,
            wire_passphrase,
        )
        config_response = await transport.send_data(
            "prov-config",
            config_request,
        )
        if prov.config_set_config_response(
            security_context,
            config_response,
        ) != 0:
            raise RuntimeError("device rejected the Wi-Fi credentials")
        phase_committed("wifi-configured")

        apply_request = prov.config_apply_config_request(
            "wifi",
            security_context,
        )
        apply_response = await transport.send_data(
            "prov-config",
            apply_request,
        )
        if prov.config_apply_config_response(
            security_context,
            apply_response,
        ) != 0:
            raise RuntimeError("device could not apply the Wi-Fi credentials")
        phase_committed("wifi-applied")
    finally:
        transport.conn.close()


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Provision and claim a BoBe body over its secure SoftAP."
    )
    parser.add_argument("--ssid", required=True, help="Target Wi-Fi SSID.")
    parser.add_argument(
        "--passphrase",
        default="",
        help="Target Wi-Fi passphrase; do not print or retain it.",
    )
    parser.add_argument(
        "--host",
        default="192.168.4.1:80",
        help="Secure SoftAP provisioning endpoint (default: 192.168.4.1:80).",
    )
    parser.add_argument(
        "--credentials",
        type=Path,
        default=ROOT / "private/provisioning/security2-private.json",
        help="Private Security-2 credential manifest.",
    )
    parser.add_argument(
        "--claim",
        type=Path,
        default=ROOT / "private/bodylink-pki/claim.json",
        help="Private BodyLink claim matching the credential device ID.",
    )
    parser.add_argument(
        "--receipt",
        type=Path,
        help="New receipt path under private/.",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Validate inputs without SoftAP traffic; writes only an explicit receipt.",
    )
    args = parser.parse_args()

    wifi_wire_values(args.ssid, args.passphrase)
    credentials, claim = validated_inputs(
        Path(os.path.abspath(args.credentials)),
        Path(os.path.abspath(args.claim)),
    )
    run_id = (
        datetime.now(UTC).strftime("%Y%m%dT%H%M%S.%fZ")
        + "-"
        + secrets.token_hex(4)
    )
    receipt_path = validated_receipt_path(
        Path(os.path.abspath(args.receipt))
        if args.receipt is not None
        else default_receipt_path(run_id)
    )
    phases = {
        "device-preflight": False,
        "claim-committed": False,
        "wifi-configured": False,
        "wifi-applied": False,
    }
    receipt: dict[str, Any] = {
        "schema_version": RECEIPT_SCHEMA,
        "run_id": run_id,
        "created_at": datetime.now(UTC).isoformat(timespec="milliseconds"),
        "status": "running",
        "device_id": credentials["device_id"],
        "credential_generation": credentials["generation_id"],
        "controller_epoch": claim["controller_epoch"],
        "host": args.host,
        "ssid_sha256": hashlib.sha256(args.ssid.encode("utf-8")).hexdigest(),
        "phases": phases,
    }

    def phase_committed(name: str) -> None:
        if name not in phases:
            raise ValueError("provisioning phase is unknown")
        phases[name] = True
        atomic_receipt(receipt_path, receipt)

    if args.dry_run:
        receipt["status"] = "blocked"
        receipt["reason"] = "dry-run-input-validation-only"
        if args.receipt is not None:
            atomic_receipt(receipt_path, receipt)
        print(
            json.dumps(
                {
                    **receipt,
                    "receipt": (
                        str(receipt_path) if args.receipt is not None else None
                    ),
                },
                indent=2,
                sort_keys=True,
            )
        )
        return

    atomic_receipt(receipt_path, receipt)
    try:
        asyncio.run(
            provision(
                args.host,
                args.ssid,
                args.passphrase,
                credentials,
                claim,
                phase_committed,
            )
        )
    except (OSError, RuntimeError, TimeoutError, ValueError) as error:
        receipt["status"] = "failed"
        receipt["finished_at"] = datetime.now(UTC).isoformat(
            timespec="milliseconds"
        )
        receipt["error"] = str(error)
        receipt["recovery"] = (
            "retry-wifi-with-same-claim"
            if phases["claim-committed"]
            else "retry-full-provisioning"
        )
        atomic_receipt(receipt_path, receipt)
        print(
            json.dumps(
                {**receipt, "receipt": str(receipt_path)},
                indent=2,
                sort_keys=True,
            )
        )
        raise SystemExit(1) from error
    receipt["status"] = "passed"
    receipt["finished_at"] = datetime.now(UTC).isoformat(
        timespec="milliseconds"
    )
    atomic_receipt(receipt_path, receipt)
    print(
        json.dumps(
            {**receipt, "receipt": str(receipt_path)},
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
