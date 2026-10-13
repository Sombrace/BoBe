import unittest
from unittest import mock

from bobe_hil import readiness


ONE_BOARD = {
    "available": True,
    "blocker": None,
    "boards": [{"device": "/dev/cu.wb12", "mac": "aa:bb:cc:dd:ee:ff"}],
}


class ReadinessTests(unittest.TestCase):
    def test_present_hardware_is_reported_available_without_qualification(
        self,
    ) -> None:
        with (
            mock.patch.object(
                readiness.device,
                "inspect_device",
                return_value=ONE_BOARD,
            ),
            mock.patch.object(
                readiness.fixture,
                "inspect_fixture",
                return_value={
                    "configured": True,
                    "identified": True,
                    "available": True,
                    "blocker": None,
                    "matching_ports": [{"device": "/dev/cu.fixture"}],
                },
            ),
            mock.patch.object(
                readiness.fixture,
                "status",
                return_value={"operation": "fixture-status"},
            ),
        ):
            status = readiness.probe_requirements(("wb12", "pico-fixture"))
        self.assertEqual(status["wb12"]["state"], "available")
        self.assertEqual(status["wb12"]["reason"], "one-wb12-connected")
        self.assertEqual(status["pico-fixture"]["state"], "available")
        self.assertNotIn("qualified", str(status))

    def test_absent_hardware_is_unavailable_not_denied(self) -> None:
        with mock.patch.object(
            readiness.device,
            "inspect_device",
            return_value={
                "available": False,
                "blocker": "no WB-12 is connected over USB",
                "boards": [],
            },
        ):
            status = readiness.probe_requirements(("wb12",))
        self.assertEqual(status["wb12"]["state"], "unavailable")
        self.assertEqual(status["wb12"]["reason"], "wb12-not-connected")

    def test_several_boards_make_the_wb12_unavailable(self) -> None:
        with mock.patch.object(
            readiness.device,
            "inspect_device",
            return_value={
                "available": False,
                "blocker": "2 WB-12 boards are connected; pass --port",
                "boards": [
                    {"device": "/dev/cu.first", "mac": "aa:bb:cc:dd:ee:ff"},
                    {"device": "/dev/cu.second", "mac": "00:11:22:33:44:55"},
                ],
            },
        ):
            status = readiness.probe_requirements(("wb12",))
        self.assertEqual(status["wb12"]["state"], "unavailable")
        self.assertEqual(status["wb12"]["reason"], "several-wb12-connected")
        self.assertEqual(len(status["wb12"]["evidence"]["boards"]), 2)

    def test_probe_failure_is_an_error_state(self) -> None:
        with (
            mock.patch.object(
                readiness.network,
                "inspect_network",
                return_value={
                    "configured": True,
                    "available": True,
                    "blocker": None,
                    "host": "192.168.50.1",
                },
            ),
            mock.patch.object(
                readiness.network,
                "status",
                side_effect=RuntimeError("ssh refused"),
            ),
        ):
            status = readiness.probe_requirements(("openwrt",))
        self.assertEqual(status["openwrt"]["state"], "error")
        self.assertEqual(status["openwrt"]["reason"], "openwrt-probe-failed")

    def test_only_requested_requirements_are_probed(self) -> None:
        with (
            mock.patch.object(readiness, "probe_camera") as camera,
            mock.patch.object(readiness, "probe_audio") as audio,
            mock.patch.object(
                readiness.device,
                "inspect_device",
                return_value=ONE_BOARD,
            ),
        ):
            status = readiness.probe_requirements(("wb12",))
        self.assertEqual(set(status), {"wb12"})
        camera.assert_not_called()
        audio.assert_not_called()

    def test_calibration_absence_is_reported_as_missing_data(self) -> None:
        with mock.patch.object(
            readiness,
            "load_audio_calibration",
            side_effect=ValueError("audio calibration is unavailable"),
        ):
            status = readiness.probe_requirements(("audio-fixture",))
        self.assertEqual(status["audio-fixture"]["state"], "unavailable")
        self.assertEqual(
            status["audio-fixture"]["reason"],
            "audio-calibration-unavailable",
        )

    def test_every_scenario_requirement_has_a_probe(self) -> None:
        from bobe_hil.scenario import REQUIREMENTS

        self.assertEqual(set(readiness.PROBES), REQUIREMENTS)

    def test_unsupported_requirement_names_are_rejected(self) -> None:
        with self.assertRaises(ValueError):
            readiness.probe_requirements(("hardwared",))


if __name__ == "__main__":
    unittest.main()
