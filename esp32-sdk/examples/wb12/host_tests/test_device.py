import sys
import subprocess
import tempfile
import threading
import unittest
from pathlib import Path
from typing import Any
from unittest import mock

from bobe_hil import device
from bobe_hil.paths import exclusive_device_access


IDENTITY = "aa:bb:cc:dd:ee:ff"
OTHER_IDENTITY = "00:11:22:33:44:55"
ELF = "a" * 64


class FakeMonitor:
    def __init__(self, chunks: list[str]) -> None:
        self.chunks = chunks
        self.index = 0
        self.disconnects = 0
        self.overflowed = False
        self.stopped = False

    def text(self) -> str:
        value = "".join(self.chunks[: self.index + 1])
        self.index = min(self.index + 1, len(self.chunks) - 1)
        return value

    def snapshot(self) -> bytes:
        return "".join(self.chunks[: self.index + 1]).encode()

    def stop(self) -> bytes:
        self.stopped = True
        return self.snapshot()


class ScriptedSerial:
    def __init__(self, events: list[bytes | Exception]) -> None:
        self.events = iter(events)
        self.writes: list[bytes] = []

    @property
    def in_waiting(self) -> int:
        return 1

    def __enter__(self) -> "ScriptedSerial":
        return self

    def __exit__(self, *_args: object) -> None:
        return None

    def read(self, _size: int) -> bytes:
        try:
            event = next(self.events)
        except StopIteration:
            return b""
        if isinstance(event, Exception):
            raise event
        return event

    def write(self, data: bytes) -> int:
        self.writes.append(data)
        return len(data)

    def flush(self) -> None:
        return None


def usb_port(
    path: str,
    serial_number: str | None,
    *,
    vid: int = device.ESPRESSIF_USB_VID,
    pid: int = device.ESP32S3_USB_SERIAL_PID,
) -> mock.Mock:
    return mock.Mock(
        device=path,
        description="USB JTAG/serial debug unit",
        vid=vid,
        pid=pid,
        serial_number=serial_number,
    )


def board(path: str = "/dev/cu.wb12", mac: str = IDENTITY) -> dict[str, object]:
    return {
        "device": path,
        "description": "USB JTAG/serial debug unit",
        "vid": device.ESPRESSIF_USB_VID,
        "pid": device.ESP32S3_USB_SERIAL_PID,
        "serial_number": mac.upper(),
        "mac": mac,
    }


def connected(*ports: mock.Mock) -> Any:
    return mock.patch.object(device.list_ports, "comports", return_value=list(ports))


class DeviceIdentityTests(unittest.TestCase):
    def test_matching_ports_require_exact_vid_pid_and_serial(self) -> None:
        ports = [
            mock.Mock(
                device="/dev/cu.wb12",
                description="ESP32-S3",
                vid=0x303A,
                pid=0x1001,
                serial_number="AABBCCDDEEFF",
            ),
            mock.Mock(
                device="/dev/cu.other",
                description="ESP32-S3",
                vid=0x303A,
                pid=0x1001,
                serial_number="001122334455",
            ),
        ]
        with mock.patch.object(device.list_ports, "comports", return_value=ports):
            matches = device.matching_ports(IDENTITY)
        self.assertEqual([match["device"] for match in matches], ["/dev/cu.wb12"])

    def test_serial_numbers_with_and_without_colons_give_one_mac(self) -> None:
        with connected(
            usb_port("/dev/cu.colons", "AA:BB:CC:DD:EE:FF"),
            usb_port("/dev/cu.digits", "aabbccddeeff"),
            usb_port("/dev/cu.mixed", "AaBbCcDdEeFf"),
        ):
            boards = device.connected_boards()
        self.assertEqual([item["mac"] for item in boards], [IDENTITY] * 3)
        self.assertEqual(
            [item["serial_number"] for item in boards],
            ["AA:BB:CC:DD:EE:FF", "aabbccddeeff", "AaBbCcDdEeFf"],
        )

    def test_connected_boards_need_the_usb_serial_jtag_ids_and_a_mac(
        self,
    ) -> None:
        with connected(
            usb_port("/dev/cu.wb12", "AA:BB:CC:DD:EE:FF"),
            usb_port("/dev/cu.uart", "001122334455", vid=0x1A86, pid=0x7523),
            usb_port("/dev/cu.no-serial", None),
            usb_port("/dev/cu.short", "AABBCCDDEE"),
            usb_port("/dev/cu.text", "not-a-mac-address"),
        ):
            boards = device.connected_boards()
        self.assertEqual(
            boards,
            [
                {
                    "device": "/dev/cu.wb12",
                    "description": "USB JTAG/serial debug unit",
                    "vid": 0x303A,
                    "pid": 0x1001,
                    "serial_number": "AA:BB:CC:DD:EE:FF",
                    "mac": IDENTITY,
                }
            ],
        )

    def test_one_connected_board_is_discovered(self) -> None:
        with connected(
            usb_port("/dev/cu.wb12", "AABBCCDDEEFF"),
            usb_port("/dev/cu.uart", "001122334455", vid=0x1A86, pid=0x7523),
        ):
            self.assertEqual(device.resolve_port(), (IDENTITY, "/dev/cu.wb12"))

    def test_no_connected_board_is_an_error(self) -> None:
        with connected():
            with self.assertRaisesRegex(
                RuntimeError,
                "^no WB-12 is connected over USB$",
            ):
                device.resolve_port()

    def test_several_connected_boards_require_a_port(self) -> None:
        with connected(
            usb_port("/dev/cu.first", "AABBCCDDEEFF"),
            usb_port("/dev/cu.second", "001122334455"),
        ):
            with self.assertRaises(RuntimeError) as error:
                device.resolve_port()
        self.assertEqual(
            str(error.exception),
            "2 WB-12 boards are connected; pass --port",
        )

    def test_explicit_port_takes_the_identity_from_its_serial_number(
        self,
    ) -> None:
        with connected(
            usb_port("/dev/cu.first", "AABBCCDDEEFF"),
            usb_port("/dev/cu.second", "00:11:22:33:44:55"),
        ):
            self.assertEqual(
                device.resolve_port("/dev/cu.second"),
                (OTHER_IDENTITY, "/dev/cu.second"),
            )

    def test_explicit_port_that_is_not_a_wb12_is_rejected(self) -> None:
        with connected(
            usb_port("/dev/cu.wb12", "AABBCCDDEEFF"),
            usb_port("/dev/cu.uart", "001122334455", vid=0x1A86, pid=0x7523),
        ):
            with self.assertRaises(RuntimeError) as error:
                device.resolve_port("/dev/cu.uart")
        self.assertEqual(
            str(error.exception),
            "/dev/cu.uart is not a connected WB-12 USB Serial/JTAG port",
        )

    def test_explicit_identity_selects_its_board(self) -> None:
        with connected(
            usb_port("/dev/cu.first", "AABBCCDDEEFF"),
            usb_port("/dev/cu.second", "001122334455"),
        ):
            self.assertEqual(
                device.resolve_port(identity=OTHER_IDENTITY),
                (OTHER_IDENTITY, "/dev/cu.second"),
            )
            self.assertEqual(
                device.resolve_port("/dev/cu.chosen", identity=IDENTITY),
                (IDENTITY, "/dev/cu.chosen"),
            )
            with self.assertRaisesRegex(RuntimeError, "found 0"):
                device.resolve_port(identity="11:22:33:44:55:66")

    def test_inspect_reports_absence_as_fact(self) -> None:
        with connected():
            report = device.inspect_device()
        self.assertEqual(
            report,
            {
                "available": False,
                "blocker": "no WB-12 is connected over USB",
                "boards": [],
            },
        )

    def test_inspect_reports_the_connected_boards(self) -> None:
        with connected(usb_port("/dev/cu.wb12", "AABBCCDDEEFF")):
            single = device.inspect_device()
        with connected(
            usb_port("/dev/cu.first", "AABBCCDDEEFF"),
            usb_port("/dev/cu.second", "001122334455"),
        ):
            several = device.inspect_device()
        self.assertTrue(single["available"])
        self.assertIsNone(single["blocker"])
        self.assertEqual([item["mac"] for item in single["boards"]], [IDENTITY])
        self.assertFalse(several["available"])
        self.assertIn("--port", several["blocker"])
        self.assertEqual(len(several["boards"]), 2)

    def test_identify_without_port_reads_the_usb_descriptor(self) -> None:
        with (
            connected(usb_port("/dev/cu.wb12", "AA:BB:CC:DD:EE:FF")),
            mock.patch.object(device, "run_esptool") as esptool,
        ):
            result = device.identify()
        self.assertEqual(result["operation"], "identify")
        self.assertEqual(result["port"]["device"], "/dev/cu.wb12")
        self.assertEqual(result["mac"], IDENTITY)
        self.assertEqual(result["identity_basis"], "USB descriptor")
        esptool.assert_not_called()

    def test_explicit_identify_reports_the_rom_mac(self) -> None:
        completed = subprocess.CompletedProcess(
            ["esptool"],
            0,
            "MAC: 11:22:33:44:55:66\n",
            "",
        )
        with (
            mock.patch.object(
                device,
                "exclusive_device_access",
            ) as exclusive_access,
            mock.patch.object(
                device,
                "run_esptool",
                return_value=completed,
            ) as esptool,
        ):
            result = device.identify(port="/dev/cu.explicit")
        self.assertEqual(
            result,
            {
                "operation": "identify",
                "port": "/dev/cu.explicit",
                "mac": "11:22:33:44:55:66",
                "identity_basis": "ESP32-S3 ROM read-mac",
            },
        )
        exclusive_access.assert_called_once_with("device")
        self.assertIn("read-mac", esptool.call_args.args[1])

    def test_boot_observe_reports_the_mac_of_the_connected_board(self) -> None:
        completed = subprocess.CompletedProcess(
            ["esptool"],
            0,
            f"MAC: {IDENTITY}\n",
            "",
        )
        with (
            connected(usb_port("/dev/cu.wb12", "AABBCCDDEEFF")),
            mock.patch.object(device, "exclusive_device_access"),
            mock.patch.object(device, "run_esptool", return_value=completed),
            mock.patch.object(
                device,
                "observe",
                return_value={"operation": "observe", "passed": True},
            ) as observed,
        ):
            result = device.boot_observe(seconds=1.0)
        self.assertEqual(result["operation"], "boot-observe")
        self.assertEqual(result["mac"], IDENTITY)
        self.assertEqual(observed.call_args.kwargs["port"], "/dev/cu.wb12")
        self.assertEqual(observed.call_args.kwargs["identity"], IDENTITY)

    def test_another_board_on_the_port_stops_the_operation(self) -> None:
        with connected(usb_port("/dev/cu.wb12", "001122334455")):
            with self.assertRaisesRegex(
                RuntimeError,
                "WB-12 USB identity changed during the operation",
            ):
                device.assert_same_usb_identity("/dev/cu.wb12", IDENTITY)


class BootMarkerTests(unittest.TestCase):
    def boot_log(self, elf: str = ELF) -> str:
        return (
            f"ELF file SHA256:  {elf[:9]}\n"
            "Starting BoBe WB-12 body\n"
            "device=11111111-2222-3333-4444-555555555555 "
            f"boot={'b' * 32} firmware=1.2.3 project=bobe_wb12 elf={elf}\n"
            "BoBe body startup completed on CPU1\n"
            "health uptime=10s\n"
        )

    def test_ordered_markers_require_the_reviewed_sequence(self) -> None:
        self.assertTrue(
            device.ordered_boot_markers(
                self.boot_log(),
                expected_elf=ELF,
            )
        )
        shuffled = "\n".join(reversed(self.boot_log().splitlines()))
        self.assertFalse(
            device.ordered_boot_markers(
                shuffled,
                expected_elf=ELF,
            )
        )

    def test_reboot_after_start_is_detected(self) -> None:
        self.assertTrue(
            device.reboot_marker_after_start(
                "Starting BoBe WB-12 body\nrst:0x3 (RTC_SW_SYS_RST)\n"
            )
        )
        self.assertFalse(device.reboot_marker_after_start(self.boot_log()))

    def test_ordered_markers_require_the_bootloader_elf_digest(self) -> None:
        without_bootloader_marker = "\n".join(
            line
            for line in self.boot_log().splitlines()
            if not line.startswith("ELF file SHA256:")
        )
        self.assertFalse(
            device.ordered_boot_markers(
                without_bootloader_marker,
                expected_elf=ELF,
            )
        )
        self.assertFalse(
            device.ordered_boot_markers(
                self.boot_log(),
                expected_elf="f" * 64,
            )
        )


class ObserveTests(unittest.TestCase):
    def test_observation_succeeds_early_on_expected_markers(self) -> None:
        monitor = FakeMonitor(["boot\n", "boot\nhealth uptime=10s\n"])
        result = device.observe(
            seconds=30,
            expect=("health uptime=10s",),
            reject=("Guru Meditation",),
            monitor=monitor,
            retain=False,
        )
        self.assertTrue(result["passed"])
        self.assertFalse(monitor.stopped)

    def test_rejected_marker_fails_the_observation(self) -> None:
        monitor = FakeMonitor(["Guru Meditation\n"])
        result = device.observe(
            seconds=1,
            expect=("health uptime=10s",),
            reject=("Guru Meditation",),
            monitor=monitor,
            retain=False,
        )
        self.assertFalse(result["passed"])
        self.assertEqual(result["rejected"], ["Guru Meditation"])

    def test_absent_markers_time_out_without_passing(self) -> None:
        monitor = FakeMonitor(["nothing useful\n"])
        result = device.observe(
            seconds=0.2,
            expect=("health uptime=10s",),
            reject=(),
            monitor=monitor,
            retain=False,
        )
        self.assertFalse(result["passed"])
        self.assertEqual(result["expected"], {"health uptime=10s": False})

    def test_observation_duration_is_bounded(self) -> None:
        with self.assertRaises(ValueError):
            device.observe(seconds=0, monitor=FakeMonitor([""]), retain=False)


class SerialMonitorTests(unittest.TestCase):
    def test_monitor_reconnects_and_counts_disconnects(self) -> None:
        payloads = [
            ScriptedSerial([b"first ", OSError("device reset")]),
            ScriptedSerial([b"second"]),
        ]
        monitor = device.SerialMonitor(IDENTITY, port="/dev/cu.wb12")
        with mock.patch.object(device.serial, "Serial", side_effect=payloads):
            monitor.start()
            deadline = threading.Event()
            deadline.wait(0.5)
            payload = monitor.stop()
        self.assertIn(b"first", payload)
        self.assertIn(b"second", payload)
        self.assertGreaterEqual(monitor.disconnects, 1)


class FlashTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.image = self.root / "bobe_wb12.bin"
        self.image.write_bytes(b"image-bytes")
        self.runs = self.root / "runs"
        self.runs.mkdir()
        patch_runs = mock.patch.object(device, "RUNS_ROOT", self.runs)
        patch_runs.start()
        self.addCleanup(patch_runs.stop)
        patch_boards = mock.patch.object(
            device,
            "connected_boards",
            return_value=[board()],
        )
        patch_boards.start()
        self.addCleanup(patch_boards.stop)
        patch_lock = mock.patch.object(
            device,
            "exclusive_device_access",
            lambda name: exclusive_device_access(name, root=self.root),
        )
        patch_lock.start()
        self.addCleanup(patch_lock.stop)

    def esptool(self, port: str, arguments: list[str], **_kwargs: object):
        self.calls.append(list(arguments))
        stdout = ""
        if "read-mac" in arguments:
            stdout = f"MAC: {IDENTITY}\n"
        elif "get-security-info" in arguments:
            stdout = "Secure Boot: Disabled\nFlash Encryption: Disabled\n"
        elif "read-flash" in arguments:
            Path(arguments[-1]).write_bytes(b"\x00" * 0x2000)
        return subprocess.CompletedProcess(["esptool"], 0, stdout, "")

    def test_offsets_must_be_flash_aligned(self) -> None:
        with self.assertRaises(ValueError):
            device.write_image(
                "/dev/cu.wb12",
                IDENTITY,
                self.image,
                self.root,
                offset=0x20001,
            )

    def test_flash_writes_the_caller_offset_without_overstating_boot(self) -> None:
        self.calls: list[list[str]] = []
        observation = {"passed": True, "required": {}, "rejected": []}
        with (
            mock.patch.object(device, "run_esptool", side_effect=self.esptool),
            mock.patch.object(
                device,
                "parse_otadata",
                return_value={"selected_slot": 0, "selected_state": "valid"},
            ),
            mock.patch.object(
                device,
                "observe_started_app",
                return_value=observation,
            ) as observed,
            mock.patch.object(
                device.fixture_control,
                "inspect_fixture",
                return_value={"available": False},
            ),
        ):
            result = device.flash(
                self.image,
                offset=0x30000,
                observe_seconds=1.0,
            )
        self.assertTrue(result["passed"])
        self.assertEqual(result["mac"], IDENTITY)
        self.assertEqual(result["written"]["offset"], "0x30000")
        self.assertFalse(result["rolled_back"])
        self.assertFalse(result["selected_for_boot"])
        self.assertIsNone(result["booted_written_image"])
        write = [call for call in self.calls if "write-flash" in call][0]
        verify = [call for call in self.calls if "verify-flash" in call][0]
        self.assertIn("0x30000", write)
        self.assertIn("0x30000", verify)
        self.assertEqual(
            verify[verify.index("--after") + 1],
            "no-reset",
        )
        self.assertLess(self.calls.index(write), self.calls.index(verify))
        reset = self.calls[self.calls.index(verify) + 1]
        self.assertIn("read-mac", reset)
        self.assertEqual(reset[reset.index("--before") + 1], "usb-reset")
        self.assertEqual(reset[reset.index("--after") + 1], "hard-reset")
        self.assertEqual(observed.call_count, 1)

    def test_failed_boot_restores_the_caller_rollback_image(self) -> None:
        self.calls = []
        rollback = self.root / "lkg.bin"
        rollback.write_bytes(b"rollback-bytes")
        observations = [
            {"passed": False, "required": {}, "rejected": ["Guru Meditation"]},
            {"passed": True, "required": {}, "rejected": []},
        ]
        with (
            mock.patch.object(device, "run_esptool", side_effect=self.esptool),
            mock.patch.object(
                device,
                "parse_otadata",
                return_value={"selected_slot": 0, "selected_state": "valid"},
            ),
            mock.patch.object(
                device,
                "observe_started_app",
                side_effect=observations,
            ),
            mock.patch.object(
                device.fixture_control,
                "inspect_fixture",
                return_value={"available": False},
            ),
        ):
            with self.assertRaises(RuntimeError) as error:
                device.flash(
                    self.image,
                    rollback_image=rollback,
                    observe_seconds=1.0,
                )
        self.assertIn("rollback image was written", str(error.exception))
        writes = [call for call in self.calls if "write-flash" in call]
        self.assertEqual(len(writes), 2)

    def test_absent_rollback_image_reports_the_original_failure(self) -> None:
        self.calls = []
        with (
            mock.patch.object(device, "run_esptool", side_effect=self.esptool),
            mock.patch.object(
                device,
                "parse_otadata",
                return_value={"selected_slot": 0, "selected_state": "valid"},
            ),
            mock.patch.object(
                device,
                "observe_started_app",
                return_value={"passed": False, "required": {}, "rejected": []},
            ),
            mock.patch.object(
                device.fixture_control,
                "inspect_fixture",
                return_value={"available": False},
            ),
        ):
            with self.assertRaises(RuntimeError) as error:
                device.flash(
                    self.image,
                    observe_seconds=1.0,
                )
        self.assertIn("boot observation failed", str(error.exception))

    def test_flash_reports_when_another_ota_slot_is_selected(self) -> None:
        self.calls = []
        with (
            mock.patch.object(device, "run_esptool", side_effect=self.esptool),
            mock.patch.object(
                device,
                "parse_otadata",
                return_value={"selected_slot": 1, "selected_state": "valid"},
            ),
            mock.patch.object(
                device,
                "observe_started_app",
                return_value={"passed": True, "required": {}, "rejected": []},
            ),
            mock.patch.object(
                device.fixture_control,
                "inspect_fixture",
                return_value={"available": False},
            ),
        ):
            result = device.flash(
                self.image,
                observe_seconds=1.0,
            )
        self.assertTrue(result["passed"])
        self.assertFalse(result["selected_for_boot"])
        self.assertFalse(result["booted_written_image"])

    def test_manifest_binds_boot_identity_for_selected_slot(self) -> None:
        self.calls = []
        manifest = {
            "app_elf_sha256": ELF,
            "image_sha256": device.sha256_file(self.image),
            "profile": "edge",
        }
        observation = {"passed": True, "required": {}, "rejected": []}
        with (
            mock.patch.object(device, "run_esptool", side_effect=self.esptool),
            mock.patch.object(
                device,
                "parse_otadata",
                return_value={"selected_slot": 0, "selected_state": "valid"},
            ),
            mock.patch.object(device, "image_manifest", return_value=manifest),
            mock.patch.object(
                device,
                "observe_started_app",
                return_value=observation,
            ) as observed,
            mock.patch.object(
                device.fixture_control,
                "inspect_fixture",
                return_value={"available": False},
            ),
        ):
            result = device.flash(
                self.image,
                observe_seconds=1.0,
            )
        self.assertTrue(result["selected_for_boot"])
        self.assertTrue(result["booted_written_image"])
        self.assertEqual(observed.call_args.kwargs["expected_elf"], ELF)

    def test_explicit_port_flash_stops_when_the_rom_mac_differs(self) -> None:
        self.calls = []

        def other_rom(port: str, arguments: list[str], **_kwargs: object):
            if "read-mac" in arguments:
                self.calls.append(list(arguments))
                return subprocess.CompletedProcess(
                    ["esptool"],
                    0,
                    f"MAC: {OTHER_IDENTITY}\n",
                    "",
                )
            return self.esptool(port, arguments)

        with (
            mock.patch.object(device, "run_esptool", side_effect=other_rom),
            mock.patch.object(
                device.fixture_control,
                "inspect_fixture",
                return_value={"available": False},
            ),
        ):
            with self.assertRaisesRegex(
                RuntimeError,
                "ROM loader identity verification failed",
            ):
                device.flash(
                    self.image,
                    port="/dev/cu.wb12",
                    observe_seconds=1.0,
                )
        self.assertEqual(len(self.calls), 1)
        self.assertFalse(any("write-flash" in call for call in self.calls))

    def test_loader_entry_warns_but_does_not_refuse_on_secure_boot(self) -> None:
        self.calls = []

        def secure(port: str, arguments: list[str], **_kwargs: object):
            if "get-security-info" in arguments:
                return subprocess.CompletedProcess(
                    ["esptool"],
                    0,
                    "Secure Boot: Enabled\nFlash Encryption: Disabled\n",
                    "",
                )
            return self.esptool(port, arguments)

        with (
            mock.patch.object(device, "run_esptool", side_effect=secure),
            mock.patch.object(
                device,
                "parse_otadata",
                return_value={"selected_slot": 0, "selected_state": "valid"},
            ),
            mock.patch.object(
                device.fixture_control,
                "inspect_fixture",
                return_value={"available": False},
            ),
        ):
            loader = device.enter_verified_loader(
                "/dev/cu.wb12",
                IDENTITY,
                self.root,
            )
        self.assertIn(
            "secure boot is enabled; a plain write can brick this device",
            loader["warnings"],
        )

    def test_fixture_loader_entry_waits_for_re_enumeration(self) -> None:
        self.calls = []
        with (
            mock.patch.object(device, "run_esptool", side_effect=self.esptool),
            mock.patch.object(
                device,
                "parse_otadata",
                return_value={"selected_slot": 0, "selected_state": "valid"},
            ),
            mock.patch.object(
                device.fixture_control,
                "inspect_fixture",
                return_value={"available": True},
            ),
            mock.patch.object(
                device.fixture_control,
                "enter_loader",
                return_value={"operation": "fixture-enter-loader"},
            ) as entered,
        ):
            loader = device.enter_verified_loader(
                "/dev/cu.wb12",
                IDENTITY,
                self.root,
            )
        self.assertEqual(entered.call_count, 1)
        self.assertEqual(loader["entered_with"], "no-reset")
        first = self.calls[0]
        self.assertIn("--before", first)
        self.assertEqual(first[first.index("--before") + 1], "no-reset")


class ExclusiveAccessTests(unittest.TestCase):
    def test_second_holder_is_refused_while_the_first_holds_the_device(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with exclusive_device_access("device", root=root):
                script = (
                    "import sys;"
                    "sys.path.insert(0, %r);"
                    "from bobe_hil.paths import exclusive_device_access;"
                    "from pathlib import Path;"
                    "exclusive_device_access('device', root=Path(%r)).__enter__()"
                    % (str(Path(__file__).resolve().parent.parent), str(root))
                )
                result = subprocess.run(
                    [sys.executable, "-c", script],
                    capture_output=True,
                    text=True,
                    check=False,
                )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("in use by another process", result.stderr)


if __name__ == "__main__":
    unittest.main()
