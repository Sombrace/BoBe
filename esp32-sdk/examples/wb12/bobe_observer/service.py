import argparse
import asyncio
import base64
import hashlib
import hmac
import json
import os
import ssl
import stat
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from cryptography import x509
from cryptography.hazmat.primitives import serialization

from bobe_observer import __version__
from bobe_observer.protocol import (
    ARTIFACT_MAX_BYTES,
    ARTIFACT_NAMES,
    MAX_ARTIFACT_CHUNK_BYTES,
    MAX_REQUEST_BYTES,
    MAX_TOTAL_ARTIFACT_BYTES,
    ObserverRequest,
    parse_request,
    response,
)


CONFIG_ROOT = Path("/etc/bobe-observer")
STATE_ROOT = Path("/var/lib/bobe-observer")
CREDENTIALS_ROOT = Path(
    "/run/credentials/bobe-observer.service"
)
CONFIG_SCHEMA = 1
PORT = 9443
MAX_REQUESTS_PER_CONNECTION = 2048


class ObserverBlockedError(RuntimeError):
    pass


class ObserverUnavailableError(RuntimeError):
    pass


@dataclass(frozen=True)
class ServiceConfig:
    observer_id: str
    deployment_sha256: str
    client_spki_sha256: str


def checked_root_file(
    path: Path,
    description: str,
    *,
    private: bool,
    maximum: int = 1024 * 1024,
) -> Path:
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"{description} is unavailable or unsafe")
    metadata = path.stat()
    forbidden = 0o077 if private else 0o022
    if (
        metadata.st_uid not in {0, os.geteuid()}
        or stat.S_IMODE(metadata.st_mode) & forbidden
        or metadata.st_size <= 0
        or metadata.st_size > maximum
    ):
        raise ValueError(f"{description} ownership, mode or size is invalid")
    return path


def checked_credential_file(
    path: Path,
    description: str,
    *,
    credentials_root: Path,
) -> Path:
    if (
        path.parent != credentials_root
        or path.is_symlink()
        or not path.is_file()
    ):
        raise ValueError(f"{description} is unavailable or unsafe")
    metadata = path.stat()
    if (
        metadata.st_uid not in {0, os.geteuid()}
        or stat.S_IMODE(metadata.st_mode) & 0o077
        or metadata.st_size <= 0
        or metadata.st_size > 64 * 1024
    ):
        raise ValueError(f"{description} owner, mode or size is invalid")
    return path


def checked_state_file(path: Path, description: str) -> Path:
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"{description} is unavailable or unsafe")
    metadata = path.stat()
    if (
        metadata.st_uid != os.geteuid()
        or stat.S_IMODE(metadata.st_mode) & 0o077
        or metadata.st_size <= 0
        or metadata.st_size > 1024 * 1024
    ):
        raise ValueError(f"{description} owner, mode or size is invalid")
    return path


def load_service_config(root: Path = CONFIG_ROOT) -> ServiceConfig:
    path = checked_root_file(
        root / "observer.json",
        "observer service configuration",
        private=False,
    )
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError("observer service configuration is invalid JSON") from error
    if not isinstance(value, dict) or set(value) != {
        "client_spki_sha256",
        "deployment_sha256",
        "observer_id",
        "schema_version",
        "target",
    }:
        raise ValueError("observer service configuration fields are invalid")
    if (
        type(value["schema_version"]) is not int
        or value["schema_version"] != CONFIG_SCHEMA
        or value["target"] != "bobe-independent-observer-service"
        or not isinstance(value["observer_id"], str)
        or len(value["observer_id"]) != 32
        or any(character not in "0123456789abcdef" for character in value["observer_id"])
        or not isinstance(value["deployment_sha256"], str)
        or len(value["deployment_sha256"]) != 64
        or any(
            character not in "0123456789abcdef"
            for character in value["deployment_sha256"]
        )
        or not isinstance(value["client_spki_sha256"], str)
        or len(value["client_spki_sha256"]) != 64
        or any(
            character not in "0123456789abcdef"
            for character in value["client_spki_sha256"]
        )
    ):
        raise ValueError("observer service configuration values are invalid")
    return ServiceConfig(
        str(value["observer_id"]),
        str(value["deployment_sha256"]),
        str(value["client_spki_sha256"]),
    )


def load_counter(root: Path = STATE_ROOT) -> int:
    path = root / "counter.json"
    if not path.exists() and not path.is_symlink():
        return 0
    path = checked_state_file(
        path,
        "observer service counter",
    )
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError("observer service counter is invalid JSON") from error
    if (
        not isinstance(value, dict)
        or set(value) != {"last_counter", "schema_version"}
        or type(value["schema_version"]) is not int
        or value["schema_version"] != 1
        or type(value["last_counter"]) is not int
        or not 0 <= value["last_counter"] < 2**63
    ):
        raise ValueError("observer service counter values are invalid")
    return int(value["last_counter"])


def checked_state_directory(path: Path, description: str) -> Path:
    if path.is_symlink() or not path.is_dir():
        raise ValueError(f"{description} is unavailable or unsafe")
    metadata = path.stat()
    if (
        metadata.st_uid != os.geteuid()
        or stat.S_IMODE(metadata.st_mode) & 0o077
    ):
        raise ValueError(f"{description} owner or mode is invalid")
    return path


def run_directory(root: Path, run_counter: int) -> Path:
    return root / "runs" / f"{run_counter:020d}"


def load_transfer_index(
    root: Path,
    run_counter: int,
    *,
    required: bool,
) -> dict[str, Any] | None:
    directory = run_directory(root, run_counter)
    if not directory.exists() and not directory.is_symlink():
        if required:
            raise ObserverUnavailableError(
                "observer artifact run is unavailable"
            )
        return None
    checked_state_directory(root / "runs", "observer runs directory")
    checked_state_directory(directory, "observer artifact run directory")
    path = directory / "transfer.json"
    if not path.exists() and not path.is_symlink() and not required:
        return None
    path = checked_state_file(path, "observer transfer index")
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError("observer transfer index is invalid JSON") from error
    if (
        not isinstance(value, dict)
        or set(value) != {
            "artifacts",
            "run_counter",
            "schema_version",
            "transfer_nonce",
        }
        or type(value["schema_version"]) is not int
        or value["schema_version"] != 1
        or type(value["run_counter"]) is not int
        or value["run_counter"] != run_counter
        or not isinstance(value["transfer_nonce"], str)
        or len(value["transfer_nonce"]) != 64
        or any(
            character not in "0123456789abcdef"
            for character in value["transfer_nonce"]
        )
        or not isinstance(value["artifacts"], list)
        or len(value["artifacts"]) != len(ARTIFACT_NAMES)
    ):
        raise ValueError("observer transfer index values are invalid")
    indexed: dict[str, dict[str, Any]] = {}
    total_size = 0
    for artifact in value["artifacts"]:
        if (
            not isinstance(artifact, dict)
            or set(artifact) != {"name", "sha256", "size"}
        ):
            raise ValueError("observer transfer artifact fields are invalid")
        name = artifact["name"]
        if (
            not isinstance(name, str)
            or name not in ARTIFACT_NAMES
            or name in indexed
            or type(artifact["size"]) is not int
            or not 0
            < artifact["size"]
            <= ARTIFACT_MAX_BYTES[name]
            or not isinstance(artifact["sha256"], str)
            or len(artifact["sha256"]) != 64
            or any(
                character not in "0123456789abcdef"
                for character in artifact["sha256"]
            )
        ):
            raise ValueError("observer transfer artifact values are invalid")
        total_size += artifact["size"]
        indexed[name] = artifact
    if (
        set(indexed) != ARTIFACT_NAMES
        or total_size > MAX_TOTAL_ARTIFACT_BYTES
    ):
        raise ValueError("observer transfer artifact set is invalid")
    return {**value, "indexed": indexed}


def read_artifact_chunk(
    root: Path,
    binding: dict[str, Any],
) -> dict[str, Any]:
    run_counter = int(binding["run_counter"])
    if run_counter != load_counter(root):
        raise ObserverUnavailableError(
            "observer artifact run is not current"
        )
    index = load_transfer_index(root, run_counter, required=True)
    if index is None:
        raise ObserverUnavailableError(
            "observer artifact run is unavailable"
        )
    if binding["transfer_nonce"] != index["transfer_nonce"]:
        raise ObserverUnavailableError(
            "observer artifact transfer nonce does not match"
        )
    artifact = index["indexed"][binding["artifact_name"]]
    offset = int(binding["offset"])
    requested = int(binding["length"])
    size = int(artifact["size"])
    if offset >= size:
        raise ValueError("observer artifact offset exceeds content")
    length = min(requested, size - offset)
    path = run_directory(root, run_counter) / str(artifact["name"])
    try:
        descriptor = os.open(
            path,
            os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
        )
    except OSError as error:
        raise ValueError("observer artifact is missing or unsafe") from error
    try:
        metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != os.geteuid()
            or stat.S_IMODE(metadata.st_mode) & 0o077
            or metadata.st_size != size
        ):
            raise ValueError("observer artifact owner, mode or size is invalid")
        os.lseek(descriptor, offset, os.SEEK_SET)
        chunk = bytearray()
        while len(chunk) < length:
            part = os.read(descriptor, length - len(chunk))
            if not part:
                raise ValueError("observer artifact ended before chunk")
            chunk.extend(part)
        if os.fstat(descriptor).st_size != size:
            raise ValueError("observer artifact changed during read")
    finally:
        os.close(descriptor)
    return {
        "artifact_name": artifact["name"],
        "artifact_sha256": artifact["sha256"],
        "chunk_sha256": hashlib.sha256(chunk).hexdigest(),
        "data_base64": base64.b64encode(chunk).decode("ascii"),
        "eof": offset + len(chunk) == size,
        "length": len(chunk),
        "offset": offset,
        "run_counter": run_counter,
        "size": size,
        "transfer_nonce": index["transfer_nonce"],
    }


def peer_spki_sha256(ssl_object: Any) -> str:
    peer_der = ssl_object.getpeercert(binary_form=True)
    if not isinstance(peer_der, bytes) or not peer_der:
        raise ValueError("observer peer certificate is unavailable")
    try:
        certificate = x509.load_der_x509_certificate(peer_der)
    except ValueError as error:
        raise ValueError("observer peer certificate is invalid") from error
    return hashlib.sha256(
        certificate.public_key().public_bytes(
            serialization.Encoding.DER,
            serialization.PublicFormat.SubjectPublicKeyInfo,
        )
    ).hexdigest()


def peer_is_pinned(
    ssl_object: object | None,
    expected_spki_sha256: str,
) -> bool:
    if ssl_object is None:
        return False
    try:
        actual = peer_spki_sha256(ssl_object)
    except (AttributeError, ValueError):
        return False
    return hmac.compare_digest(actual, expected_spki_sha256)


class StatusOnlyObserverBackend:
    def __init__(
        self,
        config: ServiceConfig,
        *,
        state_root: Path = STATE_ROOT,
    ) -> None:
        self.config = config
        self.state_root = state_root

    def status(self) -> dict[str, Any]:
        counter = load_counter(self.state_root)
        transfer = (
            load_transfer_index(
                self.state_root,
                counter,
                required=False,
            )
            if counter > 0
            else None
        )
        return {
            "version": __version__,
            "observer_id": self.config.observer_id,
            "deployment_sha256": self.config.deployment_sha256,
            "ready": False,
            "last_counter": counter,
            "operations": (
                ["status", "artifact-read"]
                if transfer is not None
                else ["status"]
            ),
            "blocker": "physical audio runner is not implemented",
        }

    def execute(self, request: ObserverRequest) -> dict[str, Any]:
        if request.operation == "status":
            return self.status()
        if request.operation == "artifact-read":
            return read_artifact_chunk(
                self.state_root,
                request.binding,
            )
        raise ObserverBlockedError(
            f"{request.operation} is not implemented"
        )


async def handle_client(
    reader: asyncio.StreamReader,
    writer: asyncio.StreamWriter,
    backend: StatusOnlyObserverBackend,
) -> None:
    for _ in range(MAX_REQUESTS_PER_CONNECTION):
        request_id = "0" * 32
        continue_allowed = True
        try:
            data = await asyncio.wait_for(reader.readline(), timeout=5.0)
            if not data:
                break
            if len(data) > MAX_REQUEST_BYTES:
                raise ValueError("observer request exceeds bound")
            request = parse_request(data)
            request_id = request.request_id
            result = await asyncio.to_thread(backend.execute, request)
            rendered = response(request_id, ok=True, result=result)
        except ObserverBlockedError as error:
            rendered = response(
                request_id,
                ok=False,
                error_code="NOT_IMPLEMENTED",
                message=str(error),
            )
        except ObserverUnavailableError as error:
            rendered = response(
                request_id,
                ok=False,
                error_code="NOT_AVAILABLE",
                message=str(error),
            )
        except (OSError, RuntimeError, TimeoutError, ValueError) as error:
            rendered = response(
                request_id,
                ok=False,
                error_code="REQUEST_FAILED",
                message=str(error),
            )
            continue_allowed = False
        writer.write(rendered)
        await writer.drain()
        if not continue_allowed:
            break
    writer.close()
    await writer.wait_closed()


def server_ssl_context(
    root: Path = CONFIG_ROOT,
    credentials: Path = CREDENTIALS_ROOT,
) -> ssl.SSLContext:
    ca = checked_root_file(
        root / "mac-client-ca.pem",
        "observer client CA",
        private=False,
        maximum=64 * 1024,
    )
    certificate = checked_root_file(
        root / "server-cert.pem",
        "observer server certificate",
        private=False,
        maximum=64 * 1024,
    )
    key = checked_credential_file(
        credentials / "tls-key.pem",
        "observer server TLS credential",
        credentials_root=credentials,
    )
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_3
    context.maximum_version = ssl.TLSVersion.TLSv1_3
    context.load_cert_chain(certificate, key)
    context.load_verify_locations(ca)
    context.verify_mode = ssl.CERT_REQUIRED
    return context


async def run_server() -> None:
    config = load_service_config()
    backend = StatusOnlyObserverBackend(config)
    context = server_ssl_context()

    async def authenticated_handler(
        reader: asyncio.StreamReader,
        writer: asyncio.StreamWriter,
    ) -> None:
        ssl_object = writer.get_extra_info("ssl_object")
        if not peer_is_pinned(ssl_object, config.client_spki_sha256):
            writer.close()
            await writer.wait_closed()
            return
        await handle_client(reader, writer, backend)

    server = await asyncio.start_server(
        authenticated_handler,
        host="0.0.0.0",
        port=PORT,
        ssl=context,
        limit=MAX_REQUEST_BYTES + 1,
    )
    async with server:
        await server.serve_forever()


def main() -> None:
    parser = argparse.ArgumentParser(
        prog="bobe-observer",
        description="Run the bounded independent BoBe observer service.",
    )
    parser.add_argument(
        "--version",
        action="version",
        version=f"bobe-observer {__version__}",
    )
    parser.add_argument("action", choices=("check-config", "serve"))
    args = parser.parse_args()
    if args.action == "check-config":
        config = load_service_config()
        status = StatusOnlyObserverBackend(config).status()
        print(json.dumps(status, indent=2, sort_keys=True))
    else:
        asyncio.run(run_server())
