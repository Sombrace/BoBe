import argparse
import importlib.util
import json
import os
import plistlib
import subprocess
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location(
    "sigrok_cli_tool",
    ROOT / "tools" / "sigrok_cli.py",
)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("could not load sigrok CLI wrapper")
sigrok = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(sigrok)


def write_session(
    path: Path,
    *,
    samples: int = 100,
    samplerate: int = 1_000_000,
    unit_size: int = 1,
    labels: tuple[str, ...] = ("D0", "D1"),
) -> None:
    metadata = (
        "[global]\n"
        "sigrok version = 0.5.2\n"
        "\n"
        "[device 1]\n"
        "capturefile = logic-1\n"
        f"samplerate = {samplerate // 1_000_000} MHz\n"
        f"unitsize = {unit_size}\n"
        "total analog = 0\n"
        f"total probes = {len(labels)}\n"
        + "".join(
            f"probe{index} = {label}\n"
            for index, label in enumerate(labels, start=1)
        )
    )
    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("version", "2\n")
        archive.writestr("metadata", metadata)
        archive.writestr("logic-1", bytes(samples * unit_size))


class SigrokCliTests(unittest.TestCase):
    @staticmethod
    def write_runtime_manifest(runtime: Path) -> Path:
        manifest = runtime / "MANIFEST.sha256"
        lines = []
        for path in sorted(
            (
                path
                for path in runtime.rglob("*")
                if path.is_file() and path != manifest
            ),
            key=lambda path: str(path.relative_to(runtime)),
        ):
            lines.append(
                f"{sigrok.sha256_file(path)}  ./{path.relative_to(runtime)}"
            )
        manifest.write_text("\n".join(lines) + "\n", encoding="ascii")
        return manifest

    def test_discovery_reports_only_matching_usb_identity(self) -> None:
        inventory = [
            {
                "idVendor": 0x0925,
                "idProduct": 0x3881,
                "bcdDevice": 0x0001,
                "locationID": 0x00144300,
                "USB Address": 26,
                "USBSpeed": 3,
            },
            {
                "idVendor": 0x1050,
                "idProduct": 0x0407,
                "locationID": 0x02000000,
                "USB Serial Number": "private-unrelated-serial",
            },
        ]
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout=plistlib.dumps(inventory),
            stderr=b"",
        )
        with mock.patch.object(
            sigrok.subprocess,
            "run",
            return_value=completed,
        ):
            matches = sigrok.discover(0x0925, 0x3881)
            candidates = sigrok.discover(None, None)
        self.assertEqual(matches, candidates)
        self.assertEqual(len(matches), 1)
        self.assertEqual(matches[0]["profile"], "saleae-logic")
        self.assertEqual(matches[0]["speed"], "high-speed")
        self.assertEqual(matches[0]["location_id"], "00144300")
        self.assertNotIn("private-unrelated-serial", json.dumps(matches))

    def test_bounded_capture_is_validated_and_atomically_published(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "capture.sr"
            args = argparse.Namespace(
                vid=0x0925,
                pid=0x3881,
                location=0x00144300,
                expected_bcd="0001",
                expected_state="original",
                channels="D0=SCL,D1=SDA",
                samplerate="1m",
                samples=100,
                trigger="D0=r",
                output=output,
                confirm_volatile_firmware_upload=True,
            )
            device = {
                "bcd_device": "0001",
                "firmware_state": "original",
                "location_id": "00144300",
                "speed": "high-speed",
                "connection": "0.26",
            }
            loaded = {
                **device,
                "firmware_state": "fx2lafw",
                "connection": "0.27",
            }
            commands: list[list[str]] = []

            def run(
                arguments: list[str],
                *,
                timeout: float,
                output_limit: int | None = None,
            ) -> subprocess.CompletedProcess[str]:
                del timeout, output_limit
                commands.append(arguments)
                if "--output-file" in arguments:
                    temporary = Path(
                        arguments[arguments.index("--output-file") + 1]
                    )
                    write_session(
                        temporary,
                        samples=100,
                        labels=("SCL", "SDA"),
                    )
                return subprocess.CompletedProcess(
                    args=arguments,
                    returncode=0,
                    stdout="device 1: 100 samples\n",
                    stderr="",
                )

            with (
                mock.patch.object(sigrok, "runtime_status"),
                mock.patch.object(
                    sigrok,
                    "require_device",
                    side_effect=(device, loaded),
                ) as identity,
                mock.patch.object(sigrok, "run_sigrok", side_effect=run),
            ):
                result = sigrok.capture(args)
            self.assertTrue(output.is_file())
            self.assertTrue(result["volatile_firmware_uploaded"])
            self.assertFalse(result["firmware_already_loaded"])
            self.assertEqual(result["session"]["sample_count"], 100)
            self.assertEqual(identity.call_count, 2)
            capture_command = commands[0]
            self.assertIn("fx2lafw:conn=0.26", capture_command)
            self.assertIn("samplerate=1m", capture_command)
            self.assertIn("D0=SCL,D1=SDA", capture_command)
            self.assertIn("SCL=r", capture_command)
            with self.assertRaisesRegex(ValueError, "new non-symlink"):
                with (
                    mock.patch.object(sigrok, "runtime_status"),
                    mock.patch.object(
                        sigrok,
                        "require_device",
                        return_value=device,
                    ),
                ):
                    sigrok.capture(args)

    def test_session_validation_rejects_traversal_and_bad_alignment(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            valid = root / "valid.sr"
            write_session(valid)
            with mock.patch.object(
                sigrok,
                "run_sigrok",
                return_value=subprocess.CompletedProcess(
                    args=[],
                    returncode=0,
                    stdout="ok\n",
                    stderr="",
                ),
            ):
                self.assertEqual(
                    sigrok.validate_session(valid)["sample_count"],
                    100,
                )
            unsafe = root / "unsafe.sr"
            with zipfile.ZipFile(unsafe, "w") as archive:
                archive.writestr("version", "2\n")
                archive.writestr("metadata", "[device 1]\nunitsize=1\n")
                archive.writestr("../logic-1", b"x")
            with self.assertRaisesRegex(ValueError, "member set"):
                sigrok.validate_session(unsafe)

            misaligned = root / "misaligned.sr"
            with zipfile.ZipFile(misaligned, "w") as archive:
                archive.writestr("version", "2\n")
                archive.writestr(
                    "metadata",
                    "[global]\nsigrok version=0.5.2\n"
                    "[device 1]\ncapturefile=logic-1\n"
                    "samplerate=1 MHz\ntotal analog=0\n"
                    "total probes=1\nunitsize=2\nprobe1=D0\n",
                )
                archive.writestr("logic-1-1", b"xx")
                archive.writestr("logic-1-2", b"x")
            with self.assertRaisesRegex(ValueError, "misaligned"):
                sigrok.validate_session(misaligned)

            gaps = root / "gaps.sr"
            with zipfile.ZipFile(gaps, "w") as archive:
                archive.writestr("version", "2\n")
                archive.writestr(
                    "metadata",
                    "[global]\nsigrok version=0.5.2\n"
                    "[device 1]\ncapturefile=logic-1\n"
                    "samplerate=1 MHz\ntotal analog=0\n"
                    "total probes=1\nunitsize=1\nprobe1=D0\n",
                )
                archive.writestr("logic-1-1", b"x")
                archive.writestr("logic-1-3", b"x")
            with self.assertRaisesRegex(ValueError, "not contiguous"):
                sigrok.validate_session(gaps)

            zero_padded = root / "zero-padded.sr"
            with zipfile.ZipFile(zero_padded, "w") as archive:
                archive.writestr("version", "2\n")
                archive.writestr(
                    "metadata",
                    "[global]\nsigrok version=0.5.2\n"
                    "[device 1]\ncapturefile=logic-1\n"
                    "samplerate=1 MHz\ntotal analog=0\n"
                    "total probes=1\nunitsize=1\nprobe1=D0\n",
                )
                archive.writestr("logic-1-01", b"x")
            with self.assertRaisesRegex(ValueError, "not contiguous"):
                sigrok.validate_session(zero_padded)

            impossible_probe = root / "impossible-probe.sr"
            with zipfile.ZipFile(impossible_probe, "w") as archive:
                archive.writestr("version", "2\n")
                archive.writestr(
                    "metadata",
                    "[global]\nsigrok version=0.5.2\n"
                    "[device 1]\ncapturefile=logic-1\n"
                    "samplerate=1 MHz\ntotal analog=0\n"
                    "total probes=9\nunitsize=1\nprobe9=CLK\n",
                )
                archive.writestr("logic-1-1", b"x")
            with self.assertRaisesRegex(ValueError, "exceeds its unit size"):
                sigrok.validate_session(impossible_probe)

    def test_offline_conversion_is_bounded_and_no_clobber(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "capture.sr"
            output = root / "capture.bin"
            write_session(source)

            def run(
                arguments: list[str],
                *,
                timeout: float,
                output_limit: int | None = None,
            ) -> subprocess.CompletedProcess[str]:
                del timeout
                self.assertEqual(output_limit, sigrok.MAX_CONVERT_BYTES)
                temporary = Path(
                    arguments[arguments.index("--output-file") + 1]
                )
                temporary.write_bytes(bytes(range(100)))
                return subprocess.CompletedProcess(
                    args=arguments,
                    returncode=0,
                    stdout="",
                    stderr="",
                )

            args = argparse.Namespace(
                input=source,
                format="binary",
                output=output,
            )
            with (
                mock.patch.object(sigrok, "runtime_status"),
                mock.patch.object(
                    sigrok,
                    "validate_session",
                    return_value={"sha256": "a" * 64},
                ),
                mock.patch.object(sigrok, "run_sigrok", side_effect=run),
            ):
                result = sigrok.convert(args)
            self.assertEqual(output.stat().st_size, 100)
            self.assertEqual(result["output_size"], 100)
            with self.assertRaisesRegex(ValueError, "new non-symlink"):
                with (
                    mock.patch.object(sigrok, "runtime_status"),
                    mock.patch.object(
                        sigrok,
                        "validate_session",
                        return_value={"sha256": "a" * 64},
                    ),
                ):
                    sigrok.convert(args)

    def test_identity_requires_iokit_and_libusb_correlation(self) -> None:
        iokit = {
            "vid": "0925",
            "pid": "3881",
            "bcd_device": "0001",
            "location_id": "00144300",
            "usb_address": 26,
            "speed": "high-speed",
        }
        libusb = {
            "vid": "0925",
            "pid": "3881",
            "bcd_device": "0001",
            "bus": 0,
            "address": 26,
            "ports": [4, 3],
            "speed": 3,
        }
        with (
            mock.patch.object(sigrok, "discover", return_value=[iokit]),
            mock.patch.object(
                sigrok,
                "libusb_devices",
                return_value=[libusb],
            ),
        ):
            result = sigrok.require_device(
                0x0925,
                0x3881,
                0x00144300,
            )
        self.assertEqual(result["connection"], "0.26")
        with (
            mock.patch.object(sigrok, "discover", return_value=[iokit]),
            mock.patch.object(
                sigrok,
                "libusb_devices",
                return_value=[{**libusb, "address": 27}],
            ),
        ):
            with self.assertRaisesRegex(RuntimeError, "same unique analyzer"):
                sigrok.require_device(
                    0x0925,
                    0x3881,
                    0x00144300,
                )

    def test_capture_bounds_and_alias_aware_trigger(self) -> None:
        channels, aliases = sigrok.normalize_channels(
            "D0=SCL,D1=SDA"
        )
        self.assertEqual(channels, "D0=SCL,D1=SDA")
        self.assertEqual(
            sigrok.normalize_trigger("D0=r", aliases),
            "SCL=r",
        )
        with self.assertRaisesRegex(ValueError, "disabled channel"):
            sigrok.normalize_trigger("D2=r", aliases)
        with self.assertRaisesRegex(ValueError, "shadows"):
            sigrok.normalize_channels("D0=D1,D1=X")
        with self.assertRaisesRegex(ValueError, "shadows"):
            sigrok.normalize_channels("D0=D1")
        parser = sigrok.build_parser()
        with self.assertRaises(SystemExit):
            parser.parse_args(
                [
                    "capture",
                    "--vid",
                    "0925",
                    "--pid",
                    "3881",
                    "--location",
                    "00144300",
                    "--expected-bcd",
                    "0001",
                    "--expected-state",
                    "original",
                    "--channels",
                    "D0",
                    "--samplerate",
                    "1m",
                    "--samples",
                    "0",
                    "--output",
                    "capture.sr",
                ]
            )
        args = argparse.Namespace(
            vid=0x0925,
            pid=0x3881,
            location=0x00144300,
            expected_bcd="0001",
            expected_state="original",
            channels="D0",
            samplerate="20k",
            samples=1_200_001,
            trigger=None,
            output=Path("capture.sr"),
            confirm_volatile_firmware_upload=True,
        )
        device = {
            "bcd_device": "0001",
            "firmware_state": "original",
            "location_id": "00144300",
            "speed": "high-speed",
            "connection": "0.26",
        }
        with (
            mock.patch.object(sigrok, "runtime_status") as runtime,
            mock.patch.object(
                sigrok,
                "require_device",
                return_value=device,
            ) as identity,
            mock.patch.object(sigrok, "run_sigrok") as runner,
        ):
            with self.assertRaisesRegex(ValueError, "60 second"):
                sigrok.capture(args)
        runner.assert_not_called()
        identity.assert_not_called()
        runtime.assert_not_called()

    def test_probe_reports_volatile_firmware_operation(self) -> None:
        args = argparse.Namespace(
            vid=0x0925,
            pid=0x3881,
            location=0x00144300,
            expected_bcd="0001",
            expected_state="original",
        )
        before = {
            "bcd_device": "0001",
            "firmware_state": "original",
            "location_id": "00144300",
            "speed": "high-speed",
            "connection": "0.26",
        }
        after = {
            **before,
            "firmware_state": "fx2lafw",
            "connection": "0.27",
        }
        with (
            mock.patch.object(sigrok, "runtime_status"),
            mock.patch.object(
                sigrok,
                "require_device",
                side_effect=(before, after),
            ),
            mock.patch.object(
                sigrok,
                "run_sigrok",
                return_value=subprocess.CompletedProcess(
                    args=[],
                    returncode=0,
                    stdout="fx2lafw:conn=0.27\n",
                    stderr="",
                ),
            ) as runner,
        ):
            result = sigrok.probe(args)
        self.assertTrue(result["volatile_firmware_uploaded"])
        self.assertTrue(result["power_cycle_restores_original_firmware"])
        self.assertEqual(
            runner.call_args.args[0],
            [
                "--driver",
                "fx2lafw:conn=0.26",
                "--scan",
            ],
        )

    def test_loaded_firmware_capture_reports_no_new_upload(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "loaded.sr"
            loaded = {
                "bcd_device": "0001",
                "firmware_state": "fx2lafw",
                "location_id": "00144300",
                "speed": "high-speed",
                "connection": "0.27",
            }
            args = argparse.Namespace(
                vid=0x0925,
                pid=0x3881,
                location=0x00144300,
                expected_bcd="0001",
                expected_state="fx2lafw",
                channels="D0=CLK",
                samplerate="1m",
                samples=100,
                trigger=None,
                output=output,
                confirm_volatile_firmware_upload=True,
            )

            def run(
                arguments: list[str],
                *,
                timeout: float,
                output_limit: int | None = None,
            ) -> subprocess.CompletedProcess[str]:
                del timeout, output_limit
                if "--output-file" in arguments:
                    write_session(
                        Path(arguments[arguments.index("--output-file") + 1]),
                        samples=100,
                        labels=("CLK",),
                    )
                return subprocess.CompletedProcess(
                    args=arguments,
                    returncode=0,
                    stdout="ok\n",
                    stderr="",
                )

            with (
                mock.patch.object(sigrok, "runtime_status"),
                mock.patch.object(
                    sigrok,
                    "require_device",
                    side_effect=(loaded, loaded),
                ),
                mock.patch.object(sigrok, "run_sigrok", side_effect=run),
            ):
                result = sigrok.capture(args)
        self.assertTrue(result["firmware_already_loaded"])
        self.assertFalse(result["volatile_firmware_uploaded"])

    def test_attempt_failure_reports_unknown_firmware_state(self) -> None:
        args = argparse.Namespace(
            vid=0x0925,
            pid=0x3881,
            location=0x00144300,
            expected_bcd="0001",
            expected_state="original",
        )
        before = {
            "bcd_device": "0001",
            "firmware_state": "original",
            "location_id": "00144300",
            "speed": "high-speed",
            "connection": "0.26",
        }
        with (
            mock.patch.object(sigrok, "runtime_status"),
            mock.patch.object(sigrok, "require_device", return_value=before),
            mock.patch.object(
                sigrok,
                "run_sigrok",
                side_effect=subprocess.TimeoutExpired("sigrok", 30),
            ),
        ):
            with self.assertRaisesRegex(
                RuntimeError,
                "firmware state is unknown",
            ) as raised:
                sigrok.probe(args)
        self.assertNotIn("retry", str(raised.exception))

    def test_runtime_manifest_rejects_extra_files_and_stale_lock(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            runtime = root / "runtime"
            runtime.mkdir()
            payload = runtime / "payload"
            payload.write_text("fixed", encoding="ascii")
            manifest = self.write_runtime_manifest(runtime)
            with (
                mock.patch.object(sigrok, "RUNTIME", runtime),
                mock.patch.object(sigrok, "MANIFEST", manifest),
            ):
                self.assertEqual(len(sigrok.parse_manifest()), 1)
                (runtime / "extra").write_text("unexpected", encoding="ascii")
                with self.assertRaisesRegex(RuntimeError, "file set"):
                    sigrok.parse_manifest()
                (runtime / "extra").unlink()
                (runtime / "dangling").symlink_to("missing")
                with self.assertRaisesRegex(RuntimeError, "symbolic link"):
                    sigrok.parse_manifest()

            target = root / "runtime-target"
            target.mkdir()
            (target / "payload").write_text("fixed", encoding="ascii")
            target_manifest = self.write_runtime_manifest(target)
            linked = root / "runtime-link"
            linked.symlink_to(target, target_is_directory=True)
            with (
                mock.patch.object(sigrok, "RUNTIME", linked),
                mock.patch.object(
                    sigrok,
                    "MANIFEST",
                    linked / target_manifest.name,
                ),
            ):
                with self.assertRaisesRegex(RuntimeError, "unsafe"):
                    sigrok.parse_manifest()

            runtime = root / "stale-runtime"
            (runtime / "bin").mkdir(parents=True)
            (runtime / "share" / "sigrok-firmware").mkdir(parents=True)
            binary = runtime / "bin" / "sigrok-cli"
            helper = runtime / "bin" / "sigrok-usb-identity"
            binary.write_text("binary", encoding="ascii")
            helper.write_text("helper", encoding="ascii")
            binary.chmod(0o700)
            helper.chmod(0o700)
            installed_lock = runtime / "sigrok-cli.lock"
            installed_lock.write_text("SIGROK_LOCK_SCHEMA=0\n", encoding="ascii")
            firmware_root = runtime / "share" / "sigrok-firmware"
            (firmware_root / "fx2lafw-saleae-logic.fw").write_bytes(b"firmware")
            manifest = self.write_runtime_manifest(runtime)
            current_lock = root / "sigrok-cli.lock"
            current_lock.write_text("SIGROK_LOCK_SCHEMA=1\n", encoding="ascii")
            with (
                mock.patch.object(sigrok, "RUNTIME", runtime),
                mock.patch.object(sigrok, "MANIFEST", manifest),
                mock.patch.object(sigrok, "BINARY", binary),
                mock.patch.object(sigrok, "USB_IDENTITY", helper),
                mock.patch.object(sigrok, "FIRMWARE_ROOT", firmware_root),
                mock.patch.object(sigrok, "LOCK", current_lock),
            ):
                with self.assertRaisesRegex(RuntimeError, "build inputs"):
                    sigrok.runtime_status()

    def test_bootstrap_fast_path_rejects_symlinks(self) -> None:
        source = (
            ROOT / "tools" / "bootstrap_sigrok_cli.sh"
        ).read_text(encoding="utf-8")
        for path in (
            "$DESTINATION",
            "$DESTINATION/bin/sigrok-cli",
            "$DESTINATION/bin/sigrok-usb-identity",
            "$DESTINATION/MANIFEST.sha256",
            "$DESTINATION/sigrok-cli.lock",
        ):
            self.assertIn(f'! -L "{path}"', source)


if __name__ == "__main__":
    unittest.main()
