import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from bobe_hil import fixture


BOARD_ID = "0123456789abcdef"
PORT = {
    "device": "/dev/cu.fixture",
    "description": "Pico",
    "vid": 0x2E8A,
    "pid": 0x0009,
    "serial_number": BOARD_ID,
}


class FakeFixtureTransport:
    def __init__(self) -> None:
        self.nonce = ""
        self.lines: list[str] = []

    def __enter__(self) -> "FakeFixtureTransport":
        return self

    def __exit__(self, *_args: object) -> None:
        return None

    def exchange(self, line: str, timeout_seconds: float) -> str:
        if not 0 < timeout_seconds <= 3:
            raise AssertionError("fixture timeout is not bounded")
        self.lines.append(line)
        fields = line.split()
        if fields[0] == "HELLO":
            self.nonce = fields[2]
            return f"OK HELLO 1 {self.nonce} 0.3.0 {BOARD_ID}"
        if fields[0] == "STATUS":
            return (
                f"OK STATUS {fields[1]} {fields[2]} "
                "mask=0 reset=0 loader=0 action_lockout=0"
            )
        if fields[0] == "RESET":
            return (
                f"OK RESET {fields[1]} {fields[2]} {fields[3]} state=scheduled"
            )
        return f"OK {fields[0]} {fields[1]} {fields[2]}"


class FixtureConfigTests(unittest.TestCase):
    def write_config(self, root: Path, **overrides: object) -> Path:
        path = root / "fixture.json"
        value = {
            "schema_version": 1,
            "board_id": BOARD_ID,
            "firmware_version": "0.3.0",
            **overrides,
        }
        path.write_text(json.dumps(value), encoding="utf-8")
        return path

    def test_configuration_needs_no_ownership_or_qualification(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = self.write_config(root)
            path.chmod(0o666)
            config = fixture.load_fixture_config(root)
            self.assertEqual(config.board_id, BOARD_ID)
            self.assertEqual(config.firmware_version, "0.3.0")

    def test_malformed_board_identity_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_config(root, board_id="nope")
            with self.assertRaises(ValueError):
                fixture.load_fixture_config(root)

    def test_usb_identity_must_match_exactly_once(self) -> None:
        port = mock.Mock(
            device="/dev/cu.fixture",
            description="Pico",
            vid=0x2E8A,
            pid=0x0009,
            serial_number=BOARD_ID.upper(),
        )
        other = mock.Mock(
            device="/dev/cu.other",
            description="Pico",
            vid=0x2E8A,
            pid=0x0009,
            serial_number="ffffffffffffffff",
        )
        with mock.patch.object(
            fixture.list_ports,
            "comports",
            return_value=[port, other],
        ):
            matches = fixture.matching_fixture_ports(BOARD_ID)
        self.assertEqual([match["device"] for match in matches], ["/dev/cu.fixture"])

    def test_inspect_reports_absence_as_fact_not_permission(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_config(root)
            with mock.patch.object(
                fixture,
                "matching_fixture_ports",
                return_value=[],
            ):
                report = fixture.inspect_fixture(root)
            self.assertTrue(report["configured"])
            self.assertFalse(report["available"])
            self.assertIn("found 0", report["blocker"])
            self.assertNotIn("qualified", json.dumps(report))


class FixtureOperationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        (self.root / "fixture.json").write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "board_id": BOARD_ID,
                    "firmware_version": "0.3.0",
                }
            ),
            encoding="utf-8",
        )
        self.transport = FakeFixtureTransport()
        patch_ports = mock.patch.object(
            fixture,
            "matching_fixture_ports",
            return_value=[PORT],
        )
        patch_transport = mock.patch.object(
            fixture,
            "SerialFixtureTransport",
            return_value=self.transport,
        )
        patch_ports.start()
        patch_transport.start()
        self.addCleanup(patch_ports.stop)
        self.addCleanup(patch_transport.stop)

    def commands(self) -> list[str]:
        return [line.split()[0] for line in self.transport.lines]

    def test_press_uses_the_caller_selected_contact_and_duration(self) -> None:
        result = fixture.press("volume-up", 75, root=self.root)
        self.assertEqual(result["contact"], "volume-up")
        self.assertEqual(result["duration_ms"], 75)
        press = [line for line in self.transport.lines if line.startswith("PRESS")]
        self.assertEqual(len(press), 1)
        self.assertTrue(press[0].endswith("VOLUME_UP 75"))
        self.assertEqual(result["fixture"]["board_id"], BOARD_ID)

    def test_chord_and_reset_accept_caller_durations(self) -> None:
        fixture.chord("shutdown", 1500, root=self.root)
        fixture.reset(180, root=self.root)
        chord = [line for line in self.transport.lines if line.startswith("CHORD")]
        reset = [line for line in self.transport.lines if line.startswith("RESET")]
        self.assertTrue(chord[0].endswith("SHUTDOWN 1500"))
        self.assertTrue(reset[0].endswith("180"))

    def test_out_of_range_durations_are_rejected_before_actuation(self) -> None:
        with self.assertRaises(ValueError):
            fixture.press("action", 5000, root=self.root)
        with self.assertRaises(ValueError):
            fixture.reset(5000, root=self.root)
        self.assertEqual(self.transport.lines, [])

    def test_unknown_contact_names_are_rejected(self) -> None:
        with self.assertRaises(ValueError):
            fixture.press("power", 100, root=self.root)

    def test_loader_entry_and_release_need_no_qualification(self) -> None:
        loader = fixture.enter_loader(self.root)
        release = fixture.release_all(self.root)
        self.assertEqual(loader["operation"], "fixture-enter-loader")
        self.assertTrue(release["released"])
        self.assertIn("ENTER_LOADER", self.commands())
        self.assertIn("RELEASE_ALL", self.commands())

    def test_missing_fixture_reports_absence(self) -> None:
        with mock.patch.object(
            fixture,
            "matching_fixture_ports",
            return_value=[],
        ):
            with self.assertRaises(RuntimeError) as error:
                fixture.status(self.root)
        self.assertIn("found 0", str(error.exception))


if __name__ == "__main__":
    unittest.main()
