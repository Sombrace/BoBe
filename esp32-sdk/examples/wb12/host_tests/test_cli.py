import unittest
from pathlib import Path
from unittest import mock

from bobe_hil import cli


class ParserTests(unittest.TestCase):
    def setUp(self) -> None:
        self.parser = cli.build_parser()

    def test_device_flash_accepts_caller_image_offset_and_rollback(self) -> None:
        args = self.parser.parse_args(
            [
                "device",
                "flash",
                "--image",
                "build-bobe-snapshot/bobe_wb12.bin",
                "--manifest",
                "build-bobe-snapshot/manifest.json",
                "--offset",
                "0x30000",
                "--rollback-image",
                "build-ondevice-reviewed/bobe_wb12.bin",
                "--observe-seconds",
                "45",
                "--no-fixture",
            ]
        )
        self.assertEqual(args.image, Path("build-bobe-snapshot/bobe_wb12.bin"))
        self.assertEqual(int(str(args.offset), 0), 0x30000)
        self.assertEqual(args.observe_seconds, 45)
        self.assertTrue(args.no_fixture)

    def test_device_observe_accepts_repeated_markers(self) -> None:
        args = self.parser.parse_args(
            [
                "device",
                "observe",
                "--seconds",
                "40",
                "--expect",
                "health uptime=",
                "--expect",
                "startup completed",
                "--reject",
                "Guru Meditation",
            ]
        )
        self.assertEqual(args.seconds, 40)
        self.assertEqual(args.expect, ["health uptime=", "startup completed"])
        self.assertEqual(args.reject, ["Guru Meditation"])

    def test_fixture_press_requires_a_contact_and_accepts_duration(self) -> None:
        args = self.parser.parse_args(
            ["fixture", "press", "--contact", "volume-up", "--ms", "75"]
        )
        self.assertEqual(args.contact, "volume-up")
        self.assertEqual(args.ms, 75)
        with self.assertRaises(SystemExit):
            self.parser.parse_args(["fixture", "press", "--contact", "power"])

    def test_net_apply_accepts_profile_and_ttl(self) -> None:
        args = self.parser.parse_args(
            ["net", "apply", "--profile", "blackhole", "--ttl", "120"]
        )
        self.assertEqual(args.profile, "blackhole")
        self.assertEqual(args.ttl, 120)

    def test_scenario_dry_run_and_live_remain_exclusive(self) -> None:
        with self.assertRaises(SystemExit):
            self.parser.parse_args(
                ["scenario", "run", "offline-boot", "--dry-run", "--live"]
            )


class DispatchTests(unittest.TestCase):
    def run_cli(self, argv: list[str]) -> None:
        with mock.patch.object(cli.sys, "argv", ["bobe-hil", *argv]):
            cli.main()

    def test_fixture_press_reaches_the_fixture_library(self) -> None:
        with (
            mock.patch.object(
                cli.fixture,
                "press",
                return_value={"operation": "fixture-press"},
            ) as press,
            mock.patch.object(cli, "print_json"),
        ):
            self.run_cli(["fixture", "press", "--contact", "action", "--ms", "250"])
        press.assert_called_once_with("action", 250)

    def test_net_apply_reaches_the_network_library(self) -> None:
        with (
            mock.patch.object(
                cli.network,
                "apply",
                return_value={"operation": "network-apply"},
            ) as apply_call,
            mock.patch.object(cli, "print_json"),
        ):
            self.run_cli(["net", "apply", "--profile", "loss-1", "--ttl", "30"])
        apply_call.assert_called_once_with("loss-1", 30)

    def test_device_flash_forwards_every_caller_parameter(self) -> None:
        with (
            mock.patch.object(
                cli.device,
                "flash",
                return_value={"operation": "flash"},
            ) as flash,
            mock.patch.object(cli, "print_json"),
        ):
            self.run_cli(
                [
                    "device",
                    "flash",
                    "--image",
                    "image.bin",
                    "--offset",
                    "0x20000",
                    "--rollback-image",
                    "lkg.bin",
                    "--observe-seconds",
                    "30",
                ]
            )
        flash.assert_called_once_with(
            Path("image.bin"),
            manifest=None,
            offset=0x20000,
            rollback_image=Path("lkg.bin"),
            rollback_manifest=None,
            observe_seconds=30.0,
            port=None,
            use_fixture=True,
        )

    def test_observer_probe_requests_a_live_connection(self) -> None:
        with (
            mock.patch.object(
                cli.observer,
                "inspect_observer",
                return_value={"configured": False},
            ) as inspect,
            mock.patch.object(cli, "print_json"),
        ):
            self.run_cli(["observer", "probe"])
        inspect.assert_called_once_with(live_probe=True)

    def test_device_status_never_actuates(self) -> None:
        with (
            mock.patch.object(
                cli.device,
                "inspect_device",
                return_value={"available": False},
            ) as inspect,
            mock.patch.object(cli.device, "flash") as flash,
            mock.patch.object(cli, "print_json"),
        ):
            self.run_cli(["device", "status"])
        inspect.assert_called_once_with()
        flash.assert_not_called()


if __name__ == "__main__":
    unittest.main()
