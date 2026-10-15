import hashlib
import ipaddress
import json
import re
import secrets
import select
import socket
import ssl
import time
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from cryptography import x509
from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import (
    Ed25519PrivateKey,
    Ed25519PublicKey,
)
from cryptography.x509.oid import ExtendedKeyUsageOID

from bobe_hil.observer_protocol import (
    MAX_RESPONSE_BYTES,
    encode_request,
    parse_response,
)
from bobe_hil.paths import private_key_file, private_root, regular_file


OBSERVER_CONFIG_SCHEMA = 1
OBSERVER_COUNTER_SCHEMA = 1
OBSERVER_PORT = 9443
OBSERVER_REQUEST_SECONDS = 12.0
RFC1918_NETWORKS = tuple(
    ipaddress.ip_network(value)
    for value in ("10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16")
)


@dataclass(frozen=True)
class ObserverConfig:
    host: str
    observer_id: str
    deployment_sha256: str
    server_spki_sha256: str
    client_spki_sha256: str


def observer_root(root: Path | None = None) -> Path:
    return private_root(root) / "observer"


def valid_hex(value: Any, length: int) -> bool:
    return (
        isinstance(value, str)
        and re.fullmatch(rf"[0-9a-f]{{{length}}}", value) is not None
    )


def load_json_file(path: Path, description: str) -> dict[str, Any]:
    regular_file(path, description)
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"{description} is invalid JSON") from error
    if not isinstance(value, dict):
        raise ValueError(f"{description} must be a JSON object")
    return value


def load_observer_config(root: Path | None = None) -> ObserverConfig:
    value = load_json_file(
        observer_root(root) / "observer.json",
        "observer configuration",
    )
    host = value.get("host")
    if not isinstance(host, str):
        raise ValueError("observer host must be a fixed IP address")
    try:
        address = ipaddress.ip_address(host)
    except ValueError as error:
        raise ValueError("observer host must be a fixed IP address") from error
    if (
        value.get("schema_version") != OBSERVER_CONFIG_SCHEMA
        or value.get("target") != "bobe-independent-observer"
        or address.version != 4
        or not any(address in network for network in RFC1918_NETWORKS)
        or address.is_loopback
        or address.is_link_local
        or address.is_multicast
        or address.is_unspecified
        or value.get("port") != OBSERVER_PORT
        or not valid_hex(value.get("observer_id"), 32)
        or not valid_hex(value.get("deployment_sha256"), 64)
        or not valid_hex(value.get("server_spki_sha256"), 64)
        or not valid_hex(value.get("client_spki_sha256"), 64)
    ):
        raise ValueError("observer configuration values are invalid")
    return ObserverConfig(
        str(address),
        str(value["observer_id"]),
        str(value["deployment_sha256"]),
        str(value["server_spki_sha256"]),
        str(value["client_spki_sha256"]),
    )


def certificate_spki_sha256(certificate: x509.Certificate) -> str:
    return hashlib.sha256(
        certificate.public_key().public_bytes(
            serialization.Encoding.DER,
            serialization.PublicFormat.SubjectPublicKeyInfo,
        )
    ).hexdigest()


def load_x509_certificate(path: Path, description: str) -> x509.Certificate:
    regular_file(path, description)
    try:
        certificate = x509.load_pem_x509_certificate(path.read_bytes())
    except ValueError as error:
        raise ValueError(f"{description} is invalid") from error
    if not isinstance(certificate.public_key(), Ed25519PublicKey):
        raise ValueError(f"{description} key is not Ed25519")
    return certificate


def certificate_valid_now(
    certificate: x509.Certificate,
    now: datetime,
    description: str,
) -> None:
    if not certificate.not_valid_before_utc <= now <= certificate.not_valid_after_utc:
        raise ValueError(f"{description} is not currently valid")


def verify_ed25519_certificate(
    certificate: x509.Certificate,
    issuer: x509.Certificate,
    description: str,
) -> None:
    if certificate.issuer != issuer.subject:
        raise ValueError(f"{description} issuer is invalid")
    issuer_key = issuer.public_key()
    if not isinstance(issuer_key, Ed25519PublicKey):
        raise ValueError(f"{description} issuer key is not Ed25519")
    try:
        issuer_key.verify(
            certificate.signature,
            certificate.tbs_certificate_bytes,
        )
    except InvalidSignature as error:
        raise ValueError(f"{description} signature is invalid") from error


def validate_tls_identity(
    root: Path,
    config: ObserverConfig,
    *,
    now: datetime,
) -> dict[str, str]:
    ca = load_x509_certificate(root / "observer-ca.pem", "observer TLS CA")
    server = load_x509_certificate(
        root / "observer-server-cert.pem",
        "observer TLS server certificate",
    )
    client = load_x509_certificate(
        root / "mac-client-cert.pem",
        "observer TLS client certificate",
    )
    private_path = private_key_file(
        root / "mac-client-key.pem",
        "observer TLS client key",
    )
    try:
        private_key = serialization.load_pem_private_key(
            private_path.read_bytes(),
            password=None,
        )
    except (TypeError, ValueError) as error:
        raise ValueError("observer TLS client key is invalid") from error
    if not isinstance(private_key, Ed25519PrivateKey):
        raise ValueError("observer TLS client key is not Ed25519")
    for certificate, description in (
        (ca, "observer TLS CA"),
        (server, "observer TLS server certificate"),
        (client, "observer TLS client certificate"),
    ):
        certificate_valid_now(certificate, now, description)
    try:
        constraints = ca.extensions.get_extension_for_class(
            x509.BasicConstraints
        ).value
    except x509.ExtensionNotFound as error:
        raise ValueError("observer TLS CA constraints are missing") from error
    if not constraints.ca:
        raise ValueError("observer TLS CA is not a CA")
    verify_ed25519_certificate(ca, ca, "observer TLS CA")
    verify_ed25519_certificate(server, ca, "observer TLS server certificate")
    verify_ed25519_certificate(client, ca, "observer TLS client certificate")
    try:
        server_usage = server.extensions.get_extension_for_class(
            x509.ExtendedKeyUsage
        ).value
        client_usage = client.extensions.get_extension_for_class(
            x509.ExtendedKeyUsage
        ).value
        server_san = server.extensions.get_extension_for_class(
            x509.SubjectAlternativeName
        ).value
    except x509.ExtensionNotFound as error:
        raise ValueError("observer TLS certificate extensions are missing") from error
    if (
        ExtendedKeyUsageOID.SERVER_AUTH not in server_usage
        or ExtendedKeyUsageOID.CLIENT_AUTH not in client_usage
        or ipaddress.ip_address(config.host)
        not in server_san.get_values_for_type(x509.IPAddress)
    ):
        raise ValueError("observer TLS certificate purpose or host is invalid")
    server_spki = certificate_spki_sha256(server)
    client_spki = certificate_spki_sha256(client)
    private_spki = hashlib.sha256(
        private_key.public_key().public_bytes(
            serialization.Encoding.DER,
            serialization.PublicFormat.SubjectPublicKeyInfo,
        )
    ).hexdigest()
    if (
        server_spki != config.server_spki_sha256
        or client_spki != config.client_spki_sha256
        or private_spki != client_spki
    ):
        raise ValueError("observer TLS SPKI binding is invalid")
    return {
        "server_spki_sha256": server_spki,
        "client_spki_sha256": client_spki,
        "ca_spki_sha256": certificate_spki_sha256(ca),
    }


def load_counter(root: Path, observer_id: str) -> int:
    path = root / "counter.json"
    if not path.exists() and not path.is_symlink():
        return 0
    value = load_json_file(path, "observer counter state")
    if (
        set(value) != {"last_counter", "observer_id", "schema_version"}
        or value["schema_version"] != OBSERVER_COUNTER_SCHEMA
        or value["observer_id"] != observer_id
        or type(value["last_counter"]) is not int
        or not 0 <= value["last_counter"] < 2**63
    ):
        raise ValueError("observer counter state is invalid")
    return int(value["last_counter"])


def observer_ssl_context(root: Path) -> ssl.SSLContext:
    ca = regular_file(root / "observer-ca.pem", "observer TLS CA")
    client = regular_file(
        root / "mac-client-cert.pem",
        "observer TLS client certificate",
    )
    key = private_key_file(
        root / "mac-client-key.pem",
        "observer TLS client key",
    )
    context = ssl.create_default_context(
        ssl.Purpose.SERVER_AUTH,
        cafile=str(ca),
    )
    context.minimum_version = ssl.TLSVersion.TLSv1_3
    context.maximum_version = ssl.TLSVersion.TLSv1_3
    context.check_hostname = True
    context.load_cert_chain(client, key)
    return context


class ObserverTransport:
    def __init__(self, root: Path, config: ObserverConfig) -> None:
        self.root = root
        self.config = config
        self.connection: ssl.SSLSocket | None = None

    @staticmethod
    def remaining(deadline: float) -> float:
        value = deadline - time.monotonic()
        if value <= 0:
            raise TimeoutError("observer request exceeded total deadline")
        return value

    def wait_for_socket(self, deadline: float, *, write: bool) -> None:
        if self.connection is None:
            raise RuntimeError("observer TLS connection is unavailable")
        readers = [] if write else [self.connection]
        writers = [self.connection] if write else []
        ready_read, ready_write, _ = select.select(
            readers,
            writers,
            [],
            self.remaining(deadline),
        )
        if not ready_read and not ready_write:
            raise TimeoutError("observer TLS operation timed out")

    def authenticate(self, deadline: float) -> None:
        if self.connection is None:
            raise RuntimeError("observer TLS connection is unavailable")
        self.connection.setblocking(False)
        while True:
            try:
                self.connection.do_handshake()
                break
            except ssl.SSLWantReadError:
                self.wait_for_socket(deadline, write=False)
            except ssl.SSLWantWriteError:
                self.wait_for_socket(deadline, write=True)
        peer_der = self.connection.getpeercert(binary_form=True)
        if not isinstance(peer_der, bytes) or not peer_der:
            raise RuntimeError("observer TLS server certificate is unavailable")
        try:
            peer = x509.load_der_x509_certificate(peer_der)
        except ValueError as error:
            raise RuntimeError("observer TLS server certificate is invalid") from error
        if certificate_spki_sha256(peer) != self.config.server_spki_sha256:
            raise RuntimeError("observer TLS server SPKI changed")

    def __enter__(self) -> "ObserverTransport":
        deadline = time.monotonic() + OBSERVER_REQUEST_SECONDS
        context = observer_ssl_context(self.root)
        raw = socket.create_connection(
            (self.config.host, OBSERVER_PORT),
            timeout=min(4.0, self.remaining(deadline)),
        )
        try:
            raw.setblocking(False)
            connection = context.wrap_socket(
                raw,
                server_hostname=self.config.host,
                do_handshake_on_connect=False,
            )
        except (OSError, ssl.SSLError, ValueError):
            raw.close()
            raise
        self.connection = connection
        try:
            self.authenticate(deadline)
        except (
            OSError,
            RuntimeError,
            TimeoutError,
            ValueError,
            ssl.SSLError,
        ):
            self.close()
            raise
        return self

    def close(self) -> None:
        if self.connection is not None:
            self.connection.close()
            self.connection = None

    def __exit__(self, *_args: object) -> None:
        self.close()

    def request(
        self,
        operation: str,
        binding: dict[str, Any],
        *,
        deadline: float | None = None,
    ) -> dict[str, Any]:
        if self.connection is None:
            raise RuntimeError("observer TLS connection is unavailable")
        request_deadline = time.monotonic() + OBSERVER_REQUEST_SECONDS
        deadline = (
            request_deadline
            if deadline is None
            else min(deadline, request_deadline)
        )
        request_id = secrets.token_hex(16)
        pending = memoryview(encode_request(request_id, operation, binding))
        while pending:
            self.remaining(deadline)
            try:
                written = self.connection.send(pending)
            except ssl.SSLWantReadError:
                self.wait_for_socket(deadline, write=False)
                continue
            except ssl.SSLWantWriteError:
                self.wait_for_socket(deadline, write=True)
                continue
            if written <= 0:
                raise RuntimeError("observer request write was incomplete")
            pending = pending[written:]
        received = bytearray()
        while b"\n" not in received:
            self.remaining(deadline)
            try:
                chunk = self.connection.recv(4096)
            except ssl.SSLWantReadError:
                self.wait_for_socket(deadline, write=False)
                continue
            except ssl.SSLWantWriteError:
                self.wait_for_socket(deadline, write=True)
                continue
            if not chunk:
                raise RuntimeError("observer closed without a response")
            received.extend(chunk)
            if len(received) > MAX_RESPONSE_BYTES:
                raise RuntimeError("observer response exceeds bound")
        return parse_response(bytes(received), request_id)


def request_observer_status(
    root: Path,
    config: ObserverConfig,
) -> dict[str, Any]:
    with ObserverTransport(root, config) as transport:
        return transport.request("status", {})


def validate_remote_status(
    value: dict[str, Any],
    config: ObserverConfig,
    last_counter: int,
) -> dict[str, Any]:
    if (
        value.get("observer_id") != config.observer_id
        or value.get("deployment_sha256") != config.deployment_sha256
        or type(value.get("ready")) is not bool
        or type(value.get("last_counter")) is not int
        or value["last_counter"] != last_counter
        or not isinstance(value.get("operations"), list)
        or not isinstance(value.get("version"), str)
        or re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", value["version"]) is None
    ):
        raise ValueError("observer remote status values are invalid")
    return value


def inspect_observer(
    root: Path | None = None,
    *,
    live_probe: bool = False,
) -> dict[str, Any]:
    location = observer_root(root)
    if location.is_symlink() or not location.is_dir():
        return {
            "configured": False,
            "identity_ready": False,
            "available": False,
            "blocker": "observer configuration root is unavailable",
        }
    try:
        config = load_observer_config(root)
        tls = validate_tls_identity(location, config, now=datetime.now(UTC))
        counter = load_counter(location, config.observer_id)
    except (OSError, ValueError) as error:
        return {
            "configured": False,
            "identity_ready": False,
            "available": False,
            "blocker": str(error),
        }
    result: dict[str, Any] = {
        "configured": True,
        "identity_ready": True,
        "available": False,
        "blocker": "observer live probe has not run",
        "host": config.host,
        "port": OBSERVER_PORT,
        "observer_id": config.observer_id,
        "deployment_sha256": config.deployment_sha256,
        "last_counter": counter,
        "tls": tls,
    }
    if not live_probe:
        return result
    try:
        remote = validate_remote_status(
            request_observer_status(location, config),
            config,
            counter,
        )
    except (
        OSError,
        RuntimeError,
        TimeoutError,
        ValueError,
        ssl.SSLError,
    ) as error:
        return {
            **result,
            "available": False,
            "blocker": f"observer live probe failed: {error}",
        }
    return {
        **result,
        "available": True,
        "blocker": None if remote["ready"] else str(remote.get("blocker")),
        "remote": remote,
    }
