import tempfile
import unittest
import xml.etree.ElementTree as ET
from dataclasses import replace
from pathlib import Path
from unittest import mock

from bobe_hil import evidence, runner, scenario


AVAILABLE = {"state": "available", "reason": "present", "evidence": {}}
UNAVAILABLE = {
    "state": "unavailable",
    "reason": "wb12-not-connected",
    "evidence": {},
}


class RecordingBackend:
    def __init__(self, run: evidence.EvidenceRun, failures: set[str] | None = None) -> None:
        self.run = run
        self.failures = failures or set()
        self.executed: list[str] = []
        self.finalized = False
        self.fixture_released = True
        self.forced: list[str] = []

    def execute(self, step: scenario.ScenarioStep) -> dict[str, object]:
        self.executed.append(step.step_id)
        if step.step_id in self.failures:
            raise RuntimeError(f"{step.step_id} failed")
        return {"step": step.step_id, "parameters": step.parameters}

    def finalize(self) -> list[str]:
        self.finalized = True
        return self.forced

    def state(self) -> dict[str, object]:
        return {
            "fixture_released": self.fixture_released,
            "background_services": [],
        }


class ScenarioRunnerTests(unittest.TestCase):
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

    def physical_scenario(self) -> scenario.Scenario:
        return scenario.load_named_scenario("physical-controls")

    def test_dry_run_plans_and_exits_zero(self) -> None:
        result = runner.run_scenario(
            self.physical_scenario(),
            execution_mode="dry-run",
        )
        self.assertEqual(result["status"], "planned")
        self.assertEqual(result["exit_code"], 0)
        self.assertEqual(
            [step["step_id"] for step in result["plan"]["steps"]],
            ["volume-up", "volume-down", "action", "observe-controls"],
        )
        self.assertEqual(
            [step["step_id"] for step in result["plan"]["cleanup"]],
            ["release-fixture"],
        )

    def test_dry_run_junit_is_not_a_skip(self) -> None:
        result = runner.run_scenario(
            self.physical_scenario(),
            execution_mode="dry-run",
        )
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "report.xml"
            runner.write_junit(result, output)
            root = ET.parse(output).getroot()
        self.assertEqual(root.attrib["skipped"], "0")
        self.assertEqual(root.attrib["failures"], "0")

    def test_planned_run_evidence_verifies(self) -> None:
        result = runner.run_scenario(
            self.physical_scenario(),
            execution_mode="dry-run",
        )
        verified = evidence.verify_run(Path(result["directory"]))
        self.assertEqual(verified["status"], "planned")
        self.assertTrue(verified["chain_intact"])
        self.assertFalse(verified["authenticated"])

    def test_live_run_dispatches_every_toml_step_in_order(self) -> None:
        backends: list[RecordingBackend] = []

        def build(run: evidence.EvidenceRun, _scenario: object = None) -> RecordingBackend:
            backend = RecordingBackend(run)
            backends.append(backend)
            return backend

        with (
            mock.patch.object(
                runner,
                "readiness_for",
                return_value={"wb12": AVAILABLE, "pico-fixture": AVAILABLE},
            ),
            mock.patch.object(runner, "LiveScenarioBackend", build),
        ):
            result = runner.run_scenario(
                self.physical_scenario(),
                execution_mode="live",
            )
        self.assertEqual(result["status"], "passed")
        self.assertEqual(
            backends[0].executed,
            ["volume-up", "volume-down", "action", "observe-controls", "release-fixture"],
        )
        self.assertTrue(backends[0].finalized)

    def test_live_step_failure_still_runs_cleanup(self) -> None:
        backends: list[RecordingBackend] = []

        def build(run: evidence.EvidenceRun, _scenario: object = None) -> RecordingBackend:
            backend = RecordingBackend(run, failures={"action"})
            backends.append(backend)
            return backend

        with (
            mock.patch.object(
                runner,
                "readiness_for",
                return_value={"wb12": AVAILABLE, "pico-fixture": AVAILABLE},
            ),
            mock.patch.object(runner, "LiveScenarioBackend", build),
        ):
            result = runner.run_scenario(
                self.physical_scenario(),
                execution_mode="live",
            )
        self.assertEqual(result["status"], "failed")
        self.assertIn("release-fixture", backends[0].executed)
        self.assertTrue(backends[0].finalized)

    def test_forced_cleanup_fails_the_run(self) -> None:
        def build(run: evidence.EvidenceRun, _scenario: object = None) -> RecordingBackend:
            backend = RecordingBackend(run)
            backend.forced = ["fixture-release-failed"]
            return backend

        with (
            mock.patch.object(
                runner,
                "readiness_for",
                return_value={"wb12": AVAILABLE, "pico-fixture": AVAILABLE},
            ),
            mock.patch.object(runner, "LiveScenarioBackend", build),
        ):
            result = runner.run_scenario(
                self.physical_scenario(),
                execution_mode="live",
            )
        self.assertEqual(result["status"], "failed")
        self.assertIn("fixture-release-failed", str(result["reason"]))

    def test_absent_hardware_blocks_before_any_actuation(self) -> None:
        executed: list[str] = []

        def build(run: evidence.EvidenceRun, _scenario: object = None) -> RecordingBackend:
            backend = RecordingBackend(run)
            executed.append("constructed")
            return backend

        with (
            mock.patch.object(
                runner,
                "readiness_for",
                return_value={"wb12": UNAVAILABLE, "pico-fixture": AVAILABLE},
            ),
            mock.patch.object(runner, "LiveScenarioBackend", build),
        ):
            result = runner.run_scenario(
                self.physical_scenario(),
                execution_mode="live",
            )
        self.assertEqual(result["status"], "blocked")
        self.assertEqual(result["exit_code"], 3)
        self.assertEqual(result["reason"], "scenario-hardware-unavailable")
        self.assertEqual(
            result["requirements"]["wb12"]["reason"],
            "wb12-not-connected",
        )
        self.assertEqual(executed, [])

    def test_edited_scenario_steps_change_execution(self) -> None:
        base = self.physical_scenario()
        edited = replace(
            base,
            steps=(
                scenario.ScenarioStep(
                    "single-press",
                    "fixture_press",
                    {"control": "action", "duration_ms": 900},
                ),
            ),
        )
        backends: list[RecordingBackend] = []

        def build(run: evidence.EvidenceRun, _scenario: object = None) -> RecordingBackend:
            backend = RecordingBackend(run)
            backends.append(backend)
            return backend

        with (
            mock.patch.object(runner, "readiness_for", return_value={}),
            mock.patch.object(runner, "LiveScenarioBackend", build),
        ):
            result = runner.run_scenario(edited, execution_mode="live")
        self.assertEqual(result["status"], "passed")
        self.assertEqual(
            backends[0].executed,
            ["single-press", "release-fixture"],
        )

    def test_mock_mode_executes_every_repository_scenario(self) -> None:
        with mock.patch(
            "bobe_hil.mock_backend.render_ui_step",
            return_value={"case_count": 26, "artifact_count": 0},
        ):
            for contract in scenario.list_scenarios():
                result = runner.run_scenario(contract, execution_mode="mock")
                self.assertEqual(result["status"], "passed", contract.name)
                self.assertEqual(result["claim"], "simulation")
                self.assertEqual(result["target_claim"], contract.claim)

    def test_passed_physical_evidence_verifies_locally(self) -> None:
        with (
            mock.patch.object(
                runner,
                "readiness_for",
                return_value={"wb12": AVAILABLE, "pico-fixture": AVAILABLE},
            ),
            mock.patch.object(
                runner,
                "LiveScenarioBackend",
                lambda run, _scenario=None: RecordingBackend(run),
            ),
        ):
            result = runner.run_scenario(
                self.physical_scenario(),
                execution_mode="live",
            )
        self.assertEqual(result["status"], "passed")
        verified = evidence.verify_run(Path(result["directory"]))
        self.assertTrue(verified["chain_intact"])
        self.assertFalse(verified["authenticated"])
        self.assertEqual(verified["claim"], "physical")


if __name__ == "__main__":
    unittest.main()
