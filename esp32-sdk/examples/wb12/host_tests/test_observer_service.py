import asyncio
import base64
import hashlib
import ipaddress
import json
import os
import ssl
import tempfile
import unittest
from datetime import UTC, datetime, timedelta
from pathlib import Path
from unittest import mock

from cryptography import x509
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.x509.oid import NameOID

from bobe_observer import protocol, service
from bobe_hil.observer_evidence import ARTIFACT_CONTRACT


ROOT = Path(__file__).resolve().parent.parent


class FakeWriter:
    def __init__(self) -> None:
        self.data = bytearray()
        self.closed = False

    def write(self, data: bytes) -> None:
        self.data.extend(data)

    async def drain(self) -> None:
        return None

    def close(self) -> None:
        self.closed = True

    async def wait_closed(self) -> None:
        return None


async def invoke(
    payload: bytes,
    backend: service.StatusOnlyObserverBackend,
) -> dict[str, object]:
    reader = asyncio.StreamReader()
    reader.feed_data(payload)
    reader.feed_eof()
    writer = FakeWriter()
    await service.handle_client(reader, writer, backend)
    return json.loads(writer.data)


async def invoke_many(
    payloads: list[bytes],
    backend: service.StatusOnlyObserverBackend,
) -> list[dict[str, object]]:
    reader = asyncio.StreamReader()
    for payload in payloads:
        reader.feed_data(payload)
    reader.feed_eof()
    writer = FakeWriter()
    await service.handle_client(reader, writer, backend)
    return [
        json.loads(line)
        for line in writer.data.decode().splitlines()
    ]


class ObserverServiceTests(unittest.TestCase):
    @staticmethod
    def certificate(
        subject_key: Ed25519PrivateKey,
        subject_name: x509.Name,
        issuer_key: Ed25519PrivateKey,
        issuer_name: x509.Name,
        *,
        ca: bool,
        server_ip: str | None = None,
    ) -> x509.Certificate:
        now = datetime.now(UTC)
        builder = (
            x509.CertificateBuilder()
            .subject_name(subject_name)
            .issuer_name(issuer_name)
            .public_key(subject_key.public_key())
            .serial_number(x509.random_serial_number())
            .not_valid_before(now - timedelta(days=1))
            .not_valid_after(now + timedelta(days=30))
            .add_extension(
                x509.BasicConstraints(ca=ca, path_length=None),
                critical=True,
            )
        )
        if server_ip is not None:
            builder = builder.add_extension(
                x509.SubjectAlternativeName(
                    [x509.IPAddress(ipaddress.ip_address(server_ip))]
                ),
                critical=False,
            )
        return builder.sign(issuer_key, algorithm=None)

    @staticmethod
    def private_bytes(key: Ed25519PrivateKey) -> bytes:
        return key.private_bytes(
            serialization.Encoding.PEM,
            serialization.PrivateFormat.PKCS8,
            serialization.NoEncryption(),
        )

    @staticmethod
    def request(
        operation: str,
        binding: dict[str, object],
    ) -> bytes:
        return (
            json.dumps(
                {
                    "schema_version": 1,
                    "request_id": "1" * 32,
                    "operation": operation,
                    "binding": binding,
                },
                separators=(",", ":"),
                sort_keys=True,
            )
            + "\n"
        ).encode()

    @staticmethod
    def physical_binding() -> dict[str, object]:
        return {
            "client_spki_sha256": "2" * 64,
            "contract_sha256": "3" * 64,
            "device_boot_id": "4" * 32,
            "device_id": "11111111-2222-4333-8444-555555555555",
            "firmware_elf_sha256": "5" * 64,
            "fixture_board_id": "6" * 16,
            "host_nonce": "7" * 64,
            "minimum_run_counter": 8,
            "usb_serial_number": "9" * 12,
        }

    def test_protocol_accepts_only_fixed_status_and_physical_bindings(
        self,
    ) -> None:
        status = protocol.parse_request(self.request("status", {}))
        self.assertEqual(status.operation, "status")
        physical = protocol.parse_request(
            self.request(
                "physical-bodylink-audio-loop",
                self.physical_binding(),
            )
        )
        self.assertEqual(
            physical.binding["minimum_run_counter"],
            8,
        )
        self.assertEqual(
            protocol.ARTIFACT_NAMES,
            set(ARTIFACT_CONTRACT),
        )
        artifact = protocol.parse_request(
            self.request(
                "artifact-read",
                {
                    "artifact_name": "observer-inputs.pcm",
                    "length": 4096,
                    "offset": 0,
                    "run_counter": 8,
                    "transfer_nonce": "a" * 64,
                },
            )
        )
        self.assertEqual(artifact.operation, "artifact-read")
        boundary = dict(artifact.binding)
        boundary["offset"] = (
            protocol.ARTIFACT_MAX_BYTES["observer-inputs.pcm"] - 4096
        )
        protocol.parse_request(self.request("artifact-read", boundary))
        out_of_bounds = dict(artifact.binding)
        out_of_bounds["offset"] = protocol.ARTIFACT_MAX_BYTES[
            "observer-inputs.pcm"
        ]
        out_of_bounds["length"] = 1
        with self.assertRaisesRegex(ValueError, "binding value"):
            protocol.parse_request(
                self.request("artifact-read", out_of_bounds)
            )
        malformed_artifact = dict(artifact.binding)
        malformed_artifact["artifact_name"] = []
        with self.assertRaisesRegex(ValueError, "binding value"):
            protocol.parse_request(
                self.request("artifact-read", malformed_artifact)
            )
        malformed = self.physical_binding()
        malformed["device_boot_id"] = "a" * 64
        with self.assertRaisesRegex(ValueError, "binding value"):
            protocol.parse_request(
                self.request(
                    "physical-bodylink-audio-loop",
                    malformed,
                )
            )
        with self.assertRaisesRegex(ValueError, "unsupported"):
            protocol.parse_request(self.request("shell", {}))
        for malformed_value in ([], {}, True, 1):
            value = json.loads(self.request("status", {}))
            value["operation"] = malformed_value
            with self.subTest(operation=malformed_value):
                with self.assertRaisesRegex(ValueError, "unsupported"):
                    protocol.parse_request(json.dumps(value).encode())
        for schema in (True, 1.0):
            value = json.loads(self.request("status", {}))
            value["schema_version"] = schema
            with self.subTest(schema=schema):
                with self.assertRaisesRegex(ValueError, "unsupported"):
                    protocol.parse_request(json.dumps(value).encode())

        malformed_response = asyncio.run(
            invoke(
                json.dumps(
                    {
                        "schema_version": 1,
                        "request_id": "1" * 32,
                        "operation": [],
                        "binding": {},
                    }
                ).encode(),
                service.StatusOnlyObserverBackend(
                    service.ServiceConfig(
                        "a" * 32,
                        "b" * 64,
                        "c" * 64,
                    )
                ),
            )
        )
        self.assertFalse(malformed_response["ok"])
        self.assertEqual(
            malformed_response["error"]["code"],
            "REQUEST_FAILED",
        )

    def test_status_only_backend_reports_status_and_blocks_physical(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory)
            counter = state / "counter.json"
            counter.write_text(
                json.dumps({"schema_version": 1, "last_counter": 9}),
                encoding="utf-8",
            )
            counter.chmod(0o600)
            backend = service.StatusOnlyObserverBackend(
                service.ServiceConfig(
                    "a" * 32,
                    "b" * 64,
                    "c" * 64,
                ),
                state_root=state,
            )
            status_response = asyncio.run(
                invoke(self.request("status", {}), backend)
            )
            self.assertTrue(status_response["ok"])
            self.assertFalse(status_response["result"]["ready"])
            self.assertEqual(status_response["result"]["operations"], ["status"])
            self.assertEqual(status_response["result"]["last_counter"], 9)
            physical_response = asyncio.run(
                invoke(
                    self.request(
                        "physical-bodylink-audio-loop",
                        self.physical_binding(),
                    ),
                    backend,
                )
            )
            self.assertFalse(physical_response["ok"])
            self.assertEqual(
                physical_response["error"]["code"],
                "NOT_IMPLEMENTED",
            )
            artifact_response = asyncio.run(
                invoke(
                    self.request(
                        "artifact-read",
                        {
                            "artifact_name": "observer-inputs.pcm",
                            "length": 4096,
                            "offset": 0,
                            "run_counter": 8,
                            "transfer_nonce": "a" * 64,
                        },
                    ),
                    backend,
                )
            )
            self.assertFalse(artifact_response["ok"])
            self.assertEqual(
                artifact_response["error"]["code"],
                "NOT_AVAILABLE",
            )
            self.assertEqual(json.loads(counter.read_text())["last_counter"], 9)

    def test_service_config_is_exact_and_private(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            config = root / "observer.json"
            config.write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "target": "bobe-independent-observer-service",
                        "observer_id": "a" * 32,
                        "deployment_sha256": "b" * 64,
                        "client_spki_sha256": "c" * 64,
                    }
                ),
                encoding="utf-8",
            )
            config.chmod(0o600)
            loaded = service.load_service_config(root)
            self.assertEqual(loaded.observer_id, "a" * 32)
            config.chmod(0o660)
            with self.assertRaisesRegex(ValueError, "ownership"):
                service.load_service_config(root)
            invalid = json.loads(config.read_text())
            invalid["schema_version"] = True
            config.write_text(json.dumps(invalid), encoding="utf-8")
            config.chmod(0o600)
            with self.assertRaisesRegex(ValueError, "values are invalid"):
                service.load_service_config(root)

    def test_artifact_transfer_is_current_nonce_bound_and_chunked(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory)
            counter = state / "counter.json"
            counter.write_text(
                json.dumps({"schema_version": 1, "last_counter": 9}),
                encoding="utf-8",
            )
            counter.chmod(0o600)
            runs = state / "runs"
            runs.mkdir(mode=0o700)
            run = runs / f"{9:020d}"
            run.mkdir(mode=0o700)
            artifacts = []
            for name in sorted(protocol.ARTIFACT_NAMES):
                size = 5000 if name == "observer-inputs.pcm" else 128
                content = hashlib.sha256(name.encode()).digest()
                content = (content * ((size // len(content)) + 1))[:size]
                path = run / name
                path.write_bytes(content)
                path.chmod(0o600)
                artifacts.append(
                    {
                        "name": name,
                        "size": size,
                        "sha256": hashlib.sha256(content).hexdigest(),
                    }
                )
            transfer_nonce = "a" * 64
            index = run / "transfer.json"
            index.write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "run_counter": 9,
                        "transfer_nonce": transfer_nonce,
                        "artifacts": artifacts,
                    }
                ),
                encoding="utf-8",
            )
            index.chmod(0o600)
            backend = service.StatusOnlyObserverBackend(
                service.ServiceConfig(
                    "a" * 32,
                    "b" * 64,
                    "c" * 64,
                ),
                state_root=state,
            )
            status = asyncio.run(
                invoke(self.request("status", {}), backend)
            )
            self.assertEqual(
                status["result"]["operations"],
                ["status", "artifact-read"],
            )
            binding = {
                "artifact_name": "observer-inputs.pcm",
                "length": 4096,
                "offset": 0,
                "run_counter": 9,
                "transfer_nonce": transfer_nonce,
            }
            first = asyncio.run(
                invoke(self.request("artifact-read", binding), backend)
            )
            self.assertTrue(first["ok"])
            self.assertEqual(first["result"]["length"], 4096)
            self.assertFalse(first["result"]["eof"])
            first_bytes = base64.b64decode(
                first["result"]["data_base64"],
                validate=True,
            )
            self.assertEqual(
                hashlib.sha256(first_bytes).hexdigest(),
                first["result"]["chunk_sha256"],
            )
            second = asyncio.run(
                invoke(
                    self.request(
                        "artifact-read",
                        {**binding, "offset": 4096},
                    ),
                    backend,
                )
            )
            self.assertTrue(second["result"]["eof"])
            self.assertEqual(second["result"]["length"], 904)
            persistent = asyncio.run(
                invoke_many(
                    [
                        self.request("status", {}),
                        self.request("artifact-read", binding),
                        self.request(
                            "artifact-read",
                            {**binding, "offset": 4096},
                        ),
                    ],
                    backend,
                )
            )
            self.assertEqual(len(persistent), 3)
            self.assertTrue(all(item["ok"] for item in persistent))
            self.assertTrue(persistent[-1]["result"]["eof"])

            wrong_nonce = asyncio.run(
                invoke(
                    self.request(
                        "artifact-read",
                        {**binding, "transfer_nonce": "b" * 64},
                    ),
                    backend,
                )
            )
            self.assertEqual(
                wrong_nonce["error"]["code"],
                "NOT_AVAILABLE",
            )
            artifact = run / "observer-inputs.pcm"
            target = run / "observer-inputs-target.pcm"
            artifact.replace(target)
            artifact.symlink_to(target)
            unsafe = asyncio.run(
                invoke(self.request("artifact-read", binding), backend)
            )
            self.assertEqual(
                unsafe["error"]["code"],
                "REQUEST_FAILED",
            )
            self.assertEqual(
                json.loads(counter.read_text())["last_counter"],
                9,
            )

    def test_tls_context_and_client_spki_are_exact(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "config"
            credentials = Path(directory) / "credentials"
            root.mkdir()
            credentials.mkdir()
            ca_key = Ed25519PrivateKey.generate()
            server_key = Ed25519PrivateKey.generate()
            client_key = Ed25519PrivateKey.generate()
            ca_name = x509.Name(
                [x509.NameAttribute(NameOID.COMMON_NAME, "Observer Client CA")]
            )
            server_name = x509.Name(
                [x509.NameAttribute(NameOID.COMMON_NAME, "Observer Server")]
            )
            client_name = x509.Name(
                [x509.NameAttribute(NameOID.COMMON_NAME, "hardwared")]
            )
            ca = self.certificate(
                ca_key,
                ca_name,
                ca_key,
                ca_name,
                ca=True,
            )
            server_certificate = self.certificate(
                server_key,
                server_name,
                ca_key,
                ca_name,
                ca=False,
                server_ip="192.168.50.2",
            )
            client_certificate = self.certificate(
                client_key,
                client_name,
                ca_key,
                ca_name,
                ca=False,
            )
            (root / "mac-client-ca.pem").write_bytes(
                ca.public_bytes(serialization.Encoding.PEM)
            )
            (root / "server-cert.pem").write_bytes(
                server_certificate.public_bytes(serialization.Encoding.PEM)
            )
            (credentials / "tls-key.pem").write_bytes(
                self.private_bytes(server_key)
            )
            for path in (*root.iterdir(), *credentials.iterdir()):
                path.chmod(0o600)
            context = service.server_ssl_context(root, credentials)
            self.assertEqual(
                context.minimum_version,
                ssl.TLSVersion.TLSv1_3,
            )
            self.assertEqual(
                context.maximum_version,
                ssl.TLSVersion.TLSv1_3,
            )
            self.assertEqual(context.verify_mode, ssl.CERT_REQUIRED)

            class Peer:
                def __init__(self, certificate: x509.Certificate) -> None:
                    self.der = certificate.public_bytes(
                        serialization.Encoding.DER
                    )

                def getpeercert(self, *, binary_form: bool = False):
                    return self.der if binary_form else {}

            peer = Peer(client_certificate)
            expected_spki = service.peer_spki_sha256(peer)
            self.assertTrue(service.peer_is_pinned(peer, expected_spki))
            self.assertFalse(service.peer_is_pinned(peer, "0" * 64))
            self.assertFalse(service.peer_is_pinned(None, expected_spki))

    def test_systemd_unit_exposes_no_hardware_or_mock_authority(self) -> None:
        unit = (
            ROOT / "hil" / "observer" / "systemd" / "bobe-observer.service"
        ).read_text(encoding="utf-8")
        self.assertIn("DevicePolicy=closed", unit)
        self.assertIn("StateDirectory=bobe-observer", unit)
        self.assertIn(
            "WorkingDirectory=/var/lib/bobe-observer",
            unit,
        )
        self.assertIn("PrivateDevices=true", unit)
        self.assertIn("NoNewPrivileges=true", unit)
        self.assertIn("ProtectSystem=strict", unit)
        self.assertIn("LoadCredential=observer-key.pem:", unit)
        self.assertNotIn("DeviceAllow=", unit)
        self.assertNotIn("--mock", unit)
        self.assertNotIn("/bin/sh", unit)


if __name__ == "__main__":
    unittest.main()
