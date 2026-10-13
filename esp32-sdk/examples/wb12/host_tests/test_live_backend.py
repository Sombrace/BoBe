import tempfile
import unittest
from pathlib import Path
from unittest import mock

from bobe_hil import evidence, live_backend, scenario
from bobe_hil.paths import exclusive_device_access


def step(step_id: str, operation: str, **parameters: object) -> scenario.ScenarioStep:
    return scenario.ScenarioStep(step_id, operation, parameters)


class LiveBackendTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        patcher = mock.patch.object(
            evidence,
            "RUNS_ROOT",
            Path(self.directory.name) / "runs",
        )
        patcher.start()
        self.addCleanup(patcher.stop)
        lock_root = Path(self.directory.name) / "locks"
        lock_patcher = mock.patch.object(
            live_backend,
            "exclusive_device_access",
            lambda name: exclusive_device_access(name, root=lock_root),
        )
        lock_patcher.start()
        self.addCleanup(lock_patcher.stop)
        self.run = evidence.EvidenceRun.create("live-backend-test", "physical")
        self.addCleanup(self.run.close)
        self.backend = live_backend.LiveScenarioBackend(self.run, identity="mac")
        self.addCleanup(self.backend.finalize)

    def new_run(self, name: str = "live-backend-extra") -> evidence.EvidenceRun:
        run = evidence.EvidenceRun.create(name, "physical")
        self.addCleanup(run.close)
        return run

    def test_serial_monitor_starts_before_any_actuating_step(self) -> None:
        contract = scenario.load_named_scenario("physical-controls")
        with mock.patch.object(
            live_backend.device,
            "SerialMonitor",
        ) as monitor_class:
            backend = live_backend.LiveScenarioBackend(
                self.new_run(),
                contract,
                identity="mac",
            )
            self.addCleanup(backend.finalize)
        monitor_class.assert_called_once_with("mac")
        monitor_class.return_value.start.assert_called_once_with()
        self.assertIsNotNone(backend.monitor)

    def test_serial_monitor_follows_the_one_connected_wb12(self) -> None:
        contract = scenario.load_named_scenario("physical-controls")
        with (
            mock.patch.object(
                live_backend.device,
                "resolve_port",
                return_value=("aa:bb:cc:dd:ee:ff", "/dev/cu.wb12"),
            ) as resolve,
            mock.patch.object(
                live_backend.device,
                "SerialMonitor",
            ) as monitor_class,
        ):
            backend = live_backend.LiveScenarioBackend(self.new_run(), contract)
            self.addCleanup(backend.finalize)
        resolve.assert_called_once_with()
        monitor_class.assert_called_once_with("aa:bb:cc:dd:ee:ff")
        self.assertEqual(backend.identity, "aa:bb:cc:dd:ee:ff")

    def test_absent_wb12_leaves_the_serial_monitor_unstarted(self) -> None:
        contract = scenario.load_named_scenario("physical-controls")
        with (
            mock.patch.object(
                live_backend.device,
                "resolve_port",
                side_effect=RuntimeError("no WB-12 is connected over USB"),
            ),
            mock.patch.object(
                live_backend.device,
                "SerialMonitor",
            ) as monitor_class,
        ):
            backend = live_backend.LiveScenarioBackend(self.new_run(), contract)
            self.addCleanup(backend.finalize)
        monitor_class.assert_not_called()
        self.assertIsNone(backend.monitor)
        self.assertIsNone(backend.identity)

    def test_absent_device_does_not_prevent_backend_construction(self) -> None:
        contract = scenario.load_named_scenario("physical-controls")
        with mock.patch.object(
            live_backend.device,
            "SerialMonitor",
            side_effect=RuntimeError("no device"),
        ):
            backend = live_backend.LiveScenarioBackend(
                self.new_run(),
                contract,
                identity="mac",
            )
            self.addCleanup(backend.finalize)
        self.assertIsNone(backend.monitor)

    def test_logical_scenario_opens_no_serial_endpoint(self) -> None:
        contract = scenario.load_named_scenario("ui-golden")
        with mock.patch.object(live_backend.device, "SerialMonitor") as monitor:
            live_backend.LiveScenarioBackend(
                self.new_run(),
                contract,
                identity="mac",
            )
        monitor.assert_not_called()

    def test_fixture_steps_forward_toml_parameters(self) -> None:
        with (
            mock.patch.object(
                live_backend.fixture,
                "press",
                return_value={"operation": "fixture-press"},
            ) as press,
            mock.patch.object(
                live_backend.fixture,
                "chord",
                return_value={"operation": "fixture-chord"},
            ) as chord,
        ):
            self.backend.execute(
                step("tap", "fixture_press", control="action", duration_ms=250)
            )
            self.backend.execute(
                step("hold", "fixture_chord", chord="shutdown", duration_ms=2000)
            )
        press.assert_called_once_with("action", 250)
        chord.assert_called_once_with("shutdown", 2000)
        self.assertTrue(self.backend.fixture_used)

    def test_network_profile_clean_clears_the_lease(self) -> None:
        with mock.patch.object(
            live_backend.network,
            "apply",
            return_value={"operation": "network-clear", "cleared": True},
        ) as apply_call:
            self.backend.execute(
                step("restore", "network_profile", profile="clean", ttl_seconds=5)
            )
        apply_call.assert_called_once_with("clean", 5)
        self.assertIsNone(self.backend.network_lease)

    def test_network_profile_records_the_applied_lease(self) -> None:
        lease = {"lease": "a" * 32, "profile": "blackhole"}
        with mock.patch.object(
            live_backend.network,
            "apply",
            return_value={"operation": "network-apply", "lease": lease},
        ):
            self.backend.execute(
                step(
                    "isolate",
                    "network_profile",
                    profile="blackhole",
                    ttl_seconds=120,
                )
            )
        self.assertEqual(self.backend.network_lease, lease)

    def test_capture_serial_uses_the_shared_monitor_and_retains_the_log(self) -> None:
        monitor = mock.Mock()
        monitor.snapshot.return_value = b"health uptime=10s\n"
        monitor.stop.return_value = b""
        self.backend.monitor = monitor
        with mock.patch.object(
            live_backend.device,
            "observe",
            return_value={"passed": True, "expected": {}, "rejected": []},
        ) as observe:
            result = self.backend.execute(
                step(
                    "watch",
                    "capture_serial",
                    duration_seconds=20,
                    expect=["health uptime="],
                    reject=["Guru Meditation"],
                )
            )
        self.assertIs(observe.call_args.kwargs["monitor"], monitor)
        self.assertEqual(observe.call_args.kwargs["seconds"], 20.0)
        self.assertEqual(observe.call_args.kwargs["expect"], ("health uptime=",))
        self.assertEqual(result["artifact"]["kind"], "serial")

    def test_capture_serial_failure_raises_with_rejected_markers(self) -> None:
        monitor = mock.Mock()
        monitor.snapshot.return_value = b"Guru Meditation\n"
        monitor.stop.return_value = b""
        self.backend.monitor = monitor
        with mock.patch.object(
            live_backend.device,
            "observe",
            return_value={
                "passed": False,
                "expected": {},
                "rejected": ["Guru Meditation"],
            },
        ):
            with self.assertRaises(RuntimeError) as error:
                self.backend.execute(
                    step(
                        "watch",
                        "capture_serial",
                        duration_seconds=5,
                        expect=["health uptime="],
                        reject=["Guru Meditation"],
                    )
                )
        self.assertIn("Guru Meditation", str(error.exception))

    def test_finalize_releases_only_a_used_fixture(self) -> None:
        with mock.patch.object(live_backend.fixture, "release_all") as release:
            forced = self.backend.finalize()
        release.assert_not_called()
        self.assertEqual(forced, [])

        self.backend.fixture_used = True
        with mock.patch.object(
            live_backend.fixture,
            "release_all",
            return_value={"released": True},
        ) as release:
            forced = self.backend.finalize()
        release.assert_called_once_with()
        self.assertEqual(forced, [])

    def test_finalize_reports_a_failed_release_as_forced_cleanup(self) -> None:
        self.backend.fixture_used = True
        with mock.patch.object(
            live_backend.fixture,
            "release_all",
            side_effect=RuntimeError("fixture disconnected"),
        ):
            forced = self.backend.finalize()
        self.assertIn("fixture-release-failed", forced)

    def test_finalize_clears_an_applied_network_lease(self) -> None:
        self.backend.network_lease = {"lease": "b" * 32}
        with mock.patch.object(
            live_backend.network,
            "clear",
            return_value={"cleared": True},
        ) as clear:
            forced = self.backend.finalize()
        clear.assert_called_once_with()
        self.assertEqual(forced, [])
        self.assertIsNone(self.backend.network_lease)

    def test_finalize_stops_the_serial_monitor(self) -> None:
        monitor = mock.Mock()
        monitor.stop.return_value = b"log"
        self.backend.monitor = monitor
        self.backend.finalize()
        monitor.stop.assert_called_once_with()
        self.assertIsNone(self.backend.monitor)

    def test_capture_frame_uses_the_calibrated_camera_identity(self) -> None:
        calibration = {"camera_id": "camera-uid", "roi": {"x": 0.0}}
        completed = mock.Mock(returncode=0, stdout='{"status": "OK"}', stderr="")

        def run(command, **_kwargs):
            Path(command[command.index("--output") + 1]).write_bytes(b"\x89PNG")
            return completed

        with (
            mock.patch.object(
                live_backend,
                "load_camera_calibration",
                return_value=calibration,
            ),
            mock.patch.object(live_backend.subprocess, "run", side_effect=run),
        ):
            result = self.backend.execute(step("frame", "capture_frame", state="idle"))
        self.assertEqual(result["camera_id"], "camera-uid")
        self.assertEqual(result["artifact"]["kind"], "frame")

    def test_unimplemented_operation_names_the_missing_implementation(self) -> None:
        with self.assertRaises(RuntimeError) as error:
            self.backend.execute(step("odd", "teleport"))
        self.assertIn("teleport", str(error.exception))

    def test_every_scenario_operation_has_a_live_handler(self) -> None:
        self.assertEqual(
            set(live_backend.OPERATION_HANDLERS),
            scenario.OPERATIONS,
        )
        for name in live_backend.OPERATION_HANDLERS.values():
            self.assertTrue(callable(getattr(self.backend, name)), name)


if __name__ == "__main__":
    unittest.main()
