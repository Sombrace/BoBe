import json
import tempfile
import threading
import unittest
from pathlib import Path
from unittest import mock

from bobe_hil import evidence, scenario


class EvidenceTests(unittest.TestCase):
    def test_challenges_are_channel_bound_and_deterministic(self) -> None:
        nonce = "01" * 16
        first = evidence.challenge(nonce, "visual")
        self.assertEqual(first, evidence.challenge(nonce, "visual"))
        self.assertNotEqual(first, evidence.challenge(nonce, "audio"))
        with self.assertRaises(ValueError):
            evidence.challenge("not-a-nonce", "visual")

    def test_run_verification_detects_artifact_tampering(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            runs_root = Path(directory) / "runs"
            with mock.patch.object(evidence, "RUNS_ROOT", runs_root):
                run = evidence.EvidenceRun.create("unit-test")
                try:
                    source = Path(directory) / "source.txt"
                    source.write_text("evidence")
                    artifact = run.add_artifact(source, "test")
                    run.event("unit-test.completed")
                    run.finish("passed")
                    verified = evidence.verify_run(run.directory)
                    self.assertTrue(verified["chain_intact"])
                    self.assertFalse(verified["authenticated"])
                    stored = run.directory / artifact["path"]
                    stored.write_text("tampered")
                    with self.assertRaisesRegex(ValueError, "mismatch"):
                        evidence.verify_run(run.directory)
                finally:
                    run.close()

    def test_run_manifest_challenges_cannot_be_rewritten(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            runs_root = Path(directory) / "runs"
            with mock.patch.object(evidence, "RUNS_ROOT", runs_root):
                run = evidence.EvidenceRun.create("unit-test")
                try:
                    run.finish("blocked", "fixture unavailable")
                    manifest_path = run.directory / "manifest.json"
                    manifest = json.loads(manifest_path.read_text())
                    manifest["challenges"]["visual"] = "0" * 64
                    manifest_path.write_text(json.dumps(manifest))
                    with self.assertRaisesRegex(ValueError, "challenges"):
                        evidence.verify_run(run.directory)
                finally:
                    run.close()

    def test_concurrent_events_preserve_one_hash_chain(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            runs_root = Path(directory) / "runs"
            with mock.patch.object(evidence, "RUNS_ROOT", runs_root):
                run = evidence.EvidenceRun.create("concurrent-events")
                source = Path(directory) / "source.txt"
                source.write_text("evidence")
                run.add_artifact(source, "test")
                failures: list[BaseException] = []

                def write_events(worker: int) -> None:
                    try:
                        for index in range(50):
                            run.event(
                                "worker.event",
                                worker=worker,
                                index=index,
                            )
                    except BaseException as error:
                        failures.append(error)

                threads = [
                    threading.Thread(target=write_events, args=(worker,))
                    for worker in range(4)
                ]
                for thread in threads:
                    thread.start()
                for thread in threads:
                    thread.join()
                self.assertEqual(failures, [])
                run.finish("passed")
                verified = evidence.verify_run(run.directory)
                self.assertTrue(verified["chain_intact"])
                self.assertEqual(verified["event_count"], 201)

    def test_empty_passed_run_is_not_verifiable(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            runs_root = Path(directory) / "runs"
            with mock.patch.object(evidence, "RUNS_ROOT", runs_root):
                run = evidence.EvidenceRun.create("empty-pass")
                try:
                    run.finish("passed")
                    with self.assertRaisesRegex(
                        ValueError,
                        "lacks scenario artifacts or events",
                    ):
                        evidence.verify_run(run.directory)
                finally:
                    run.close()

    def test_event_chain_and_physical_claim_prevent_false_passes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            runs_root = Path(directory) / "runs"
            with mock.patch.object(evidence, "RUNS_ROOT", runs_root):
                run = evidence.EvidenceRun.create(
                    "physical-unit-test",
                    "physical",
                )
                source = Path(directory) / "source.txt"
                source.write_text("evidence")
                run.add_artifact(source, "test")
                run.event("physical.observed")
                run.finish("passed")
                with self.assertRaisesRegex(
                    ValueError,
                    "lacks completed hardware step evidence",
                ):
                    evidence.verify_run(run.directory)

                events_path = run.directory / "events.jsonl"
                events = events_path.read_text().splitlines()
                changed = json.loads(events[0])
                changed["kind"] = "tampered"
                events[0] = json.dumps(changed)
                events_path.write_text("\n".join(events) + "\n")
                with self.assertRaisesRegex(ValueError, "chain"):
                    evidence.verify_run(run.directory)


class ScenarioTests(unittest.TestCase):
    def test_repository_scenarios_are_valid(self) -> None:
        scenarios = scenario.list_scenarios()
        self.assertGreaterEqual(len(scenarios), 4)
        self.assertEqual(
            {item.name for item in scenarios},
            {
                "offline-boot",
                "physical-controls",
                "ui-golden",
                "voice-roundtrip",
            },
        )

    def test_rejects_unsupported_operation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "unsafe.toml"
            path.write_text(
                "\n".join(
                    (
                        "schema_version = 2",
                        'name = "unsafe"',
                        'description = "unsafe"',
                        'claim = "logical"',
                        "requires = []",
                        "[[steps]]",
                        'id = "unsafe-step"',
                        'operation = "shell"',
                    )
                )
            )
            with mock.patch.object(scenario, "SCENARIOS_ROOT", root):
                with self.assertRaisesRegex(ValueError, "has no implementation"):
                    scenario.load_scenario(path)


if __name__ == "__main__":
    unittest.main()
