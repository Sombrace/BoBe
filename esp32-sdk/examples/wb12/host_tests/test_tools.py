import asyncio
import binascii
import json
import re
import struct
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import (
    Ed25519PrivateKey,
)

ROOT = Path(__file__).resolve().parent.parent
TOOLS = ROOT / "tools"
sys.path.insert(0, str(TOOLS))

from bobe_hil import image as bobe_image
from bobe_hil import ui as bobe_ui
import generate_provisioning_credentials as provisioning_credentials
import provision_bobe as provision_tool
import configure_bobe_usb as usb_config_tool


def run_tool(name: str, *arguments: Path | str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(TOOLS / name), *(str(value) for value in arguments)],
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
    )


def ota_entry(sequence: int, state: int = 2, *, valid_crc: bool = True) -> bytes:
    checksum = (
        binascii.crc32(struct.pack("<I", sequence), 0xFFFFFFFF) & 0xFFFFFFFF
    )
    if not valid_crc:
        checksum ^= 0xFFFFFFFF
    return struct.pack("<I20xII", sequence, state, checksum)


class BoBeImageToolTests(unittest.TestCase):
    def test_parses_selected_ota_slot(self) -> None:
        data = bytearray(b"\xff" * 0x2000)
        data[0 : bobe_image.OTA_ENTRY.size] = ota_entry(1)
        parsed = bobe_image.parse_otadata(bytes(data))
        self.assertEqual(parsed["selected_slot"], 0)
        self.assertEqual(parsed["selected_state"], "valid")

        data[0x1000 : 0x1000 + bobe_image.OTA_ENTRY.size] = ota_entry(2)
        parsed = bobe_image.parse_otadata(bytes(data))
        self.assertEqual(parsed["selected_slot"], 1)
        self.assertEqual(parsed["selected_copy"], 1)

    def test_rejects_invalid_or_unsafe_otadata(self) -> None:
        with self.assertRaisesRegex(ValueError, "truncated"):
            bobe_image.parse_otadata(b"\xff" * 32)

        invalid = bytearray(b"\xff" * 0x2000)
        invalid[: bobe_image.OTA_ENTRY.size] = ota_entry(1, valid_crc=False)
        with self.assertRaisesRegex(ValueError, "no valid selection"):
            bobe_image.parse_otadata(bytes(invalid))

        aborted = bytearray(b"\xff" * 0x2000)
        aborted[: bobe_image.OTA_ENTRY.size] = ota_entry(1, state=2)
        aborted[
            0x1000 : 0x1000 + bobe_image.OTA_ENTRY.size
        ] = ota_entry(2, state=4)
        selected = bobe_image.parse_otadata(bytes(aborted))
        self.assertEqual(selected["selected_slot"], 0)

    def test_manifest_must_match_inspected_image(self) -> None:
        inspected = {
            "schema_version": 1,
            "target": "esp32s3",
            "project_name": "bobe_wb12",
            "app_version": "1",
            "idf_version": "v6.0.2",
            "app_elf_sha256": "a" * 64,
            "image_size": 64,
            "image_sha256": "b" * 64,
        }
        manifest = {
            **inspected,
            "profile": "edge",
            "elf_file_sha256": "c" * 64,
            "source_commit": "d" * 40,
            "source_dirty": False,
            "created_at": "2026-07-21T00:00:00+00:00",
        }
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            image = root / "app.bin"
            image.write_bytes(b"x")
            manifest_path = root / "app.bin.manifest.json"
            manifest_path.write_text(json.dumps(manifest))
            with mock.patch.object(
                bobe_image,
                "inspect_application",
                return_value=inspected,
            ):
                loaded = bobe_image.load_manifest(manifest_path, image)
                self.assertEqual(loaded["profile"], "edge")

                manifest["image_sha256"] = "e" * 64
                manifest_path.write_text(json.dumps(manifest))
                with self.assertRaisesRegex(ValueError, "does not match"):
                    bobe_image.load_manifest(manifest_path, image)

                manifest["image_sha256"] = "b" * 64
                manifest["build_number"] = 3
                manifest_path.write_text(json.dumps(manifest))
                self.assertEqual(
                    bobe_image.load_manifest(manifest_path, image)["build_number"],
                    3,
                )
                for malformed in ("3", 0, True):
                    manifest["build_number"] = malformed
                    manifest_path.write_text(json.dumps(manifest))
                    with self.assertRaisesRegex(ValueError, "build number"):
                        bobe_image.load_manifest(manifest_path, image)

    def test_manifest_records_the_built_firmware_build_number(self) -> None:
        inspected = {
            "schema_version": 1,
            "target": "esp32s3",
            "project_name": "bobe_wb12",
            "app_version": "1",
            "idf_version": "v6.0.2",
            "app_elf_sha256": "a" * 64,
            "image_size": 64,
            "image_sha256": "b" * 64,
        }
        with tempfile.TemporaryDirectory() as directory:
            build = Path(directory)
            image = build / "bobe_wb12.bin"
            image.write_bytes(b"x")
            elf = build / "bobe_wb12.elf"
            elf.write_bytes(b"elf")
            configuration = build / "config" / "sdkconfig.json"
            configuration.parent.mkdir()
            configuration.write_text(json.dumps({"BOBE_BUILD_NUMBER": 5}))
            with (
                mock.patch.object(
                    bobe_image,
                    "inspect_application",
                    return_value=dict(inspected),
                ),
                mock.patch.object(
                    bobe_image,
                    "git_metadata",
                    return_value=("d" * 40, False),
                ),
            ):
                manifest = bobe_image.create_manifest(build, image, elf, "edge")
                self.assertEqual(manifest["build_number"], 5)
                configuration.unlink()
                with self.assertRaisesRegex(ValueError, "build configuration"):
                    bobe_image.create_manifest(build, image, elf, "edge")

    def test_manifest_signature_covers_provenance(self) -> None:
        manifest = {
            "schema_version": 1,
            "target": "esp32s3",
            "project_name": "bobe_wb12",
            "app_version": "1",
            "idf_version": "v6.0.2",
            "app_elf_sha256": "a" * 64,
            "image_size": 64,
            "image_sha256": "b" * 64,
            "profile": "edge",
            "elf_file_sha256": "c" * 64,
            "source_commit": "d" * 40,
            "source_dirty": False,
            "created_at": "2026-07-21T00:00:00+00:00",
        }
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            private_key = root / "private.pem"
            public_key = root / "public.pem"
            key = Ed25519PrivateKey.generate()
            private_key.write_bytes(
                key.private_bytes(
                    serialization.Encoding.PEM,
                    serialization.PrivateFormat.PKCS8,
                    serialization.NoEncryption(),
                )
            )
            public_key.write_bytes(
                key.public_key().public_bytes(
                    serialization.Encoding.PEM,
                    serialization.PublicFormat.SubjectPublicKeyInfo,
                )
            )
            signed = bobe_image.sign_manifest(manifest, private_key)
            bobe_image.verify_manifest_signature(signed, public_key)
            signed["source_commit"] = "e" * 40
            with self.assertRaisesRegex(ValueError, "signature is invalid"):
                bobe_image.verify_manifest_signature(signed, public_key)


class UsbConfigurationToolTests(unittest.TestCase):
    def test_base64url_omits_padding(self) -> None:
        self.assertEqual(
            usb_config_tool.base64url(b"WrongNetwork"),
            "V3JvbmdOZXR3b3Jr",
        )

    def test_loads_ignored_env_without_exposing_values(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "wifi.env"
            path.write_text(
                "BOBE_WIFI_SSID='network'\n"
                "BOBE_WIFI_PASSWORD='password-123'\n"
            )
            self.assertEqual(
                usb_config_tool.load_env(path),
                ("network", "password-123"),
            )


class ToolAuthorityTests(unittest.TestCase):
    def test_host_ui_dimensions_match_firmware_header(self) -> None:
        source = (
            ROOT / "components/bobe_device_config/include/bobe_device_config.h"
        ).read_text()
        width = re.search(
            r"^#define BOBE_DISPLAY_WIDTH ([0-9]+)U$",
            source,
            re.MULTILINE,
        )
        height = re.search(
            r"^#define BOBE_DISPLAY_HEIGHT ([0-9]+)U$",
            source,
            re.MULTILINE,
        )
        self.assertIsNotNone(width)
        self.assertIsNotNone(height)
        self.assertEqual(bobe_ui.UI_WIDTH, int(width.group(1)))
        self.assertEqual(bobe_ui.UI_HEIGHT, int(height.group(1)))

    def test_lab_help_owns_media_and_hardware_operation_descriptions(self) -> None:
        for arguments, expected in (
            (
                ("media", "--help"),
                (
                    "Generate deterministic mono PCM16",
                    "List exact CoreAudio input/output names",
                    "Convert explicit-dimension big-endian RGB565",
                    "may prompt macOS TCC",
                    "produces audible sound",
                ),
            ),
            (
                ("scenario", "run", "--help"),
                (
                    "status is planned with exit 0",
                    "simulation, never physical",
                    "blocked and skipped in JUnit",
                ),
            ),
            (
                ("device", "--help"),
                (
                    "Report the WB-12 boards connected over USB",
                    "caller-selected",
                    "Write and read-back verify",
                ),
            ),
            (
                ("device", "flash", "--help"),
                (
                    "--image",
                    "--offset",
                    "--rollback-image",
                    "--observe-seconds",
                ),
            ),
            (
                ("fixture", "--help"),
                (
                    "Close one contact for a bounded duration",
                    "wired multi-contact chord",
                    "Release every fixture contact",
                ),
            ),
            (
                ("net", "apply", "--help"),
                (
                    "--profile",
                    "--ttl",
                    "clean clears any active lease",
                ),
            ),
            (
                ("observer", "--help"),
                (
                    "pinned observer TLS identity",
                    "bounded mTLS status",
                ),
            ),
        ):
            with self.subTest(arguments=arguments):
                result = subprocess.run(
                    [str(TOOLS / "bobe_hil.sh"), *arguments],
                    cwd=ROOT,
                    check=False,
                    capture_output=True,
                    text=True,
                )
                self.assertEqual(result.returncode, 0)
                for text in expected:
                    self.assertIn(text, result.stdout)

    def test_camera_and_sigrok_help_expose_current_interfaces(self) -> None:
        camera = subprocess.run(
            [str(TOOLS / "bobe_capture.sh"), "--help"],
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(camera.returncode, 0)
        self.assertIn("Invoke through tools/bobe_capture.sh", camera.stdout)
        self.assertIn("owns macOS TCC permission", camera.stdout)
        self.assertIn("All commands emit one JSON object", camera.stdout)
        self.assertIn("preflight", camera.stdout)
        self.assertIn("rectify", camera.stdout)
        list_help = subprocess.run(
            [str(TOOLS / "bobe_capture.sh"), "list", "--help"],
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(list_help.returncode, 0)
        self.assertIn("--summary", list_help.stdout)

        capture = subprocess.run(
            [str(TOOLS / "sigrok.sh"), "capture", "--help"],
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(capture.returncode, 0)
        self.assertIn("D0=ALIAS", capture.stdout)
        self.assertIn("Dn=0|1|r|f|e", capture.stdout)
        self.assertIn("exceed 60 seconds", capture.stdout)
        self.assertIn("uploads fx2lafw into RAM", capture.stdout)

    def test_check_wrapper_help_names_gates_and_boundaries(self) -> None:
        result = subprocess.run(
            [str(TOOLS / "check.sh"), "--help"],
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0)
        for gate in (
            "all",
            "core-policy",
            "ui",
            "ui-regenerate",
            "fixture",
            "capture",
        ):
            self.assertIn(gate, result.stdout)
        self.assertIn("No gate flashes or actuates hardware", result.stdout)
        self.assertIn("ui-regenerate mutates references", result.stdout)

    def test_check_wrapper_rejects_unknown_gate_without_retention_claim(self) -> None:
        result = subprocess.run(
            [str(TOOLS / "check.sh"), "not-a-gate"],
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 2)
        self.assertNotIn("artifacts retained", result.stderr)

    def test_idf_wrapper_exposes_project_local_idf_cli(self) -> None:
        result = subprocess.run(
            [str(TOOLS / "idf.sh"), "--help"],
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0)
        self.assertIn("flash", result.stdout)
        self.assertIn("app-flash", result.stdout)
        self.assertIn("monitor", result.stdout)


class ProvisioningCredentialBundleTests(unittest.TestCase):
    @staticmethod
    def staged_bundle(root: Path, generation: str) -> Path:
        staged = root / f"staged-{generation}"
        staged.mkdir(mode=0o700)
        (staged / "bobe_provisioning_credentials_private.c").write_text(
            generation,
            encoding="utf-8",
        )
        (staged / "security2-private.json").write_text(
            json.dumps({"generation_id": generation}),
            encoding="utf-8",
        )
        return staged

    def test_create_refuses_replacement_and_rotate_archives_whole_bundle(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "provisioning"
            first_generation = "a" * 32
            first = self.staged_bundle(root, first_generation)
            self.assertIsNone(
                provisioning_credentials.publish_bundle(
                    first,
                    output,
                    rotate=False,
                )
            )
            self.assertEqual(
                (output / "bobe_provisioning_credentials_private.c").read_text(),
                first_generation,
            )

            duplicate = self.staged_bundle(root, "b" * 32)
            with self.assertRaisesRegex(ValueError, "--create"):
                provisioning_credentials.publish_bundle(
                    duplicate,
                    output,
                    rotate=False,
                )

            replacement_generation = "c" * 32
            replacement = self.staged_bundle(
                root,
                replacement_generation,
            )
            archived = provisioning_credentials.publish_bundle(
                replacement,
                output,
                rotate=True,
            )
            self.assertEqual(
                archived,
                root / "provisioning-history" / first_generation,
            )
            self.assertEqual(
                (archived / "bobe_provisioning_credentials_private.c").read_text(),
                first_generation,
            )
            self.assertEqual(
                (output / "bobe_provisioning_credentials_private.c").read_text(),
                replacement_generation,
            )


class BoBeProvisioningToolTests(unittest.TestCase):
    @staticmethod
    def write_inputs(root: Path, claim_device_id: str = "device-1") -> tuple[Path, Path]:
        credentials = root / "credentials.json"
        claim = root / "claim.json"
        credentials.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "generation_id": "0" * 32,
                    "device_id": "device-1",
                    "username": "bobe-device-1",
                    "password": "srp-password",
                    "softap_password": "softap-password-123",
                }
            )
        )
        claim.write_text(
            json.dumps(
                {
                    "endpoint_uri": "wss://bobe.local:7443/body/v1",
                    "trust_kid": "development",
                    "device_id": claim_device_id,
                    "controller_epoch": 1,
                    "server_ca_pem": "ca",
                    "client_cert_pem": "cert",
                    "client_key_pem": "key",
                }
            )
        )
        credentials.chmod(0o600)
        claim.chmod(0o600)
        return credentials, claim

    def test_validates_private_inputs_without_disclosing_secrets(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            credentials, claim = self.write_inputs(Path(directory))
            result = run_tool(
                "provision_bobe.py",
                "--ssid",
                "test-network",
                "--credentials",
                credentials,
                "--claim",
                claim,
                "--dry-run",
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            receipt = json.loads(result.stdout)
            self.assertEqual(receipt["status"], "blocked")
            self.assertEqual(
                receipt["reason"],
                "dry-run-input-validation-only",
            )
            self.assertIsNone(receipt["receipt"])
            self.assertNotIn("srp-password", result.stdout + result.stderr)
            self.assertNotIn("softap-password", result.stdout + result.stderr)

    def test_accepts_utf8_ssid_and_rejects_symlinked_private_input(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            credentials, claim = self.write_inputs(root)
            unicode_ssid = run_tool(
                "provision_bobe.py",
                "--ssid",
                "Renée’s Hotspot",
                "--passphrase",
                "synthetic-passphrase",
                "--credentials",
                credentials,
                "--claim",
                claim,
                "--dry-run",
            )
            self.assertEqual(unicode_ssid.returncode, 0, unicode_ssid.stderr)

            credentials_link = root / "credentials-link.json"
            credentials_link.symlink_to(credentials)
            symlinked = run_tool(
                "provision_bobe.py",
                "--ssid",
                "test-network",
                "--credentials",
                credentials_link,
                "--claim",
                claim,
                "--dry-run",
            )
            self.assertNotEqual(symlinked.returncode, 0)
            self.assertIn("refusing symbolic link", symlinked.stderr)

    def test_rejects_identity_mismatch_and_broad_permissions(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            credentials, claim = self.write_inputs(root, "device-2")
            mismatch = run_tool(
                "provision_bobe.py",
                "--ssid",
                "test-network",
                "--credentials",
                credentials,
                "--claim",
                claim,
                "--dry-run",
            )
            self.assertNotEqual(mismatch.returncode, 0)
            self.assertIn("identities differ", mismatch.stderr)

            _, matching_claim = self.write_inputs(root)
            matching_claim.chmod(0o644)
            broad = run_tool(
                "provision_bobe.py",
                "--ssid",
                "test-network",
                "--credentials",
                credentials,
                "--claim",
                matching_claim,
                "--dry-run",
            )
            self.assertNotEqual(broad.returncode, 0)
            self.assertIn("permissions are too broad", broad.stderr)

    def test_rejects_credentials_without_a_generation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            credentials, claim = self.write_inputs(Path(directory))
            value = json.loads(credentials.read_text())
            del value["generation_id"]
            credentials.write_text(json.dumps(value))
            missing = run_tool(
                "provision_bobe.py",
                "--ssid",
                "test-network",
                "--credentials",
                credentials,
                "--claim",
                claim,
                "--dry-run",
            )
            self.assertNotEqual(missing.returncode, 0)
            self.assertIn("missing generation_id", missing.stderr)

    def test_reports_claim_commit_before_wifi_apply_failure(self) -> None:
        class FakeConnection:
            def close(self) -> None:
                return None

        class FakeTransport:
            def __init__(self) -> None:
                self.conn = FakeConnection()

            async def send_data(
                self,
                endpoint: str,
                _payload: object,
            ) -> object:
                if endpoint == "proto-ver":
                    return '{"prov":{"sec_patch_ver":1}}'
                if endpoint == "prov-session":
                    return b"session-response"
                if endpoint == "bobe-claim":
                    return "claim-response"
                return b"config-response"

        class FakeSecurityContext:
            def security_session(self, response: object) -> bytes | None:
                return b"session-request" if response is None else None

            def encrypt_data(self, _value: bytes) -> bytes:
                return b"encrypted-claim"

            def decrypt_data(self, _value: bytes) -> bytes:
                return b'{"status":"ok"}'

        class FakeSecurityModule:
            @staticmethod
            def Security2(*_args: object) -> FakeSecurityContext:
                return FakeSecurityContext()

        class FakeProvisioningModule:
            @staticmethod
            def config_set_config_request(*_args: object) -> bytes:
                return b"set"

            @staticmethod
            def config_set_config_response(*_args: object) -> int:
                return 0

            @staticmethod
            def config_apply_config_request(*_args: object) -> bytes:
                return b"apply"

            @staticmethod
            def config_apply_config_response(*_args: object) -> int:
                return 1

        transport = FakeTransport()
        phases: list[str] = []
        with mock.patch.object(
            provision_tool,
            "load_esp_prov",
            return_value=(
                FakeProvisioningModule(),
                FakeSecurityModule(),
                mock.Mock(Transport_HTTP=lambda _host: transport),
                lambda _value: b"claim",
            ),
        ):
            with self.assertRaisesRegex(RuntimeError, "could not apply"):
                asyncio.run(
                    provision_tool.provision(
                        "192.168.4.1:80",
                        "test-network",
                        "test-password",
                        {
                            "username": "user",
                            "password": "password",
                        },
                        {"device_id": "device-1"},
                        phases.append,
                    )
                )
        self.assertEqual(
            phases,
            [
                "device-preflight",
                "claim-committed",
                "wifi-configured",
            ],
        )


if __name__ == "__main__":
    unittest.main()
